//! `budget_proof`: run `tokenfuse backtest` over the calls the gateway recorded
//! against a candidate budget the operator wrote, and report the proof in both
//! halves.
//!
//! A read in every sense that matters: it changes nothing in any plane, writes
//! no journal line and signs nothing. It does run child processes and it does
//! write one temporary file (a copy of the incident's own calls, in a directory
//! only this process can enter, removed when the proof ends), which is why it is
//! bounded: the proof takes one of the slots every proof shares, all its
//! children share one deadline, each child gets an output cap and no environment
//! but a `PATH` and the trace directory it is pointed at, and none runs in a
//! shell.
//!
//! # THE STEPS
//!
//! 1. `tokenfuse sql "copy (...) to <scratch>/part.parquet ..."` over the
//!    gateway's trace copies the incident's run into a trace of its own and
//!    reports how many calls it copied. None is [`ProofError::NotInTrace`].
//! 2. `tokenfuse sql` over that copy counts its calls and distinct steps; fewer
//!    steps than calls means the run has calls at the same step.
//! 3. `tokenfuse backtest <candidate>` over the copy is the first half.
//! 4. `tokenfuse backtest <candidate>` over the gateway's trace is the whole
//!    trace, of which the second half is what is left when the first is taken
//!    out.
//!
//! # WHERE EACH INPUT COMES FROM
//!
//! - the binary: `tokenfuse` on `PATH`, which the console image carries beside
//!   `wardryx`, `qryx` and `mockryx`;
//! - the trace: `TOKENFUSE_DATA_DIR`, tokenfuse's own name for the directory of
//!   Parquet files the gateway writes, mounted read only.
//!
//! A missing input is an answer that names it ([`ProofError::NotConfigured`]),
//! never a proof that nothing changes.

use super::budget::{
    BacktestParseError, BacktestReport, BudgetCandidate, BudgetProof, STEP_COUNT_STATEMENT,
    copy_statement, judge_budget, parse_backtest, parse_copy_count, parse_step_counts,
    query_safe_run,
};
use super::commands::{Finished, PROOF_SLOTS, ProofError, Scratch, run_bounded, run_of};
use super::slots::Slots;
use crate::bus::AppState;
use crate::case::Data;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long one budget proof may run, all its children together.
pub const BUDGET_TIMEOUT: Duration = Duration::from_secs(30);

/// Where the proof runs from.
#[derive(Debug, Clone)]
pub struct BudgetConfig {
    /// The `tokenfuse` binary.
    pub tokenfuse_bin: PathBuf,
    /// The gateway's trace directory. `None` is not configured.
    pub trace_dir: Option<PathBuf>,
}

impl BudgetConfig {
    /// The configuration of this box: `tokenfuse` on `PATH` and the trace
    /// directory tokenfuse itself names.
    pub fn from_env() -> Self {
        Self {
            tokenfuse_bin: PathBuf::from("tokenfuse"),
            trace_dir: std::env::var("TOKENFUSE_DATA_DIR")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
        }
    }
}

/// A request for a budget proof.
#[derive(Debug, Clone)]
pub struct BudgetRequest {
    /// The incident Incident 360 holds (`bus:<n>`, `verdryx:<n>` or `<n>`).
    pub incident_id: String,
    /// The most one run may spend, in dollars, as typed.
    pub per_run_usd: Option<String>,
    /// The most one call may cost, in dollars, as typed.
    pub per_step_usd: Option<String>,
    /// The most calls one run may make.
    pub max_steps: Option<u32>,
}

/// When every child of one proof must be done: the proof's own clock, not each
/// child's.
fn deadline_of(started: Instant, timeout: Duration) -> Instant {
    started + timeout
}

fn refused(reason: &'static str) -> ProofError {
    ProofError::CandidateRefused { reason }
}

fn spawn_failed(e: std::io::Error) -> ProofError {
    ProofError::Spawn {
        detail: Data::bounded(&e.to_string()),
    }
}

/// What a child that said something other than what was expected leaves: its
/// own error text, or its output when it kept the error to itself.
fn failed(done: &Finished) -> ProofError {
    let detail = if done.stderr.trim().is_empty() {
        &done.stdout
    } else {
        &done.stderr
    };
    ProofError::ReplayFailed {
        exit: done.exit,
        detail: Data::bounded(detail),
    }
}

/// Run `tokenfuse` with these arguments over the trace in `data_dir`. The child
/// has until `timeout` after the proof `started`, whatever the children before it
/// took.
fn tokenfuse(
    config: &BudgetConfig,
    args: &[String],
    data_dir: &Path,
    started: Instant,
    timeout: Duration,
) -> Result<Finished, ProofError> {
    let argv: Vec<OsString> = args.iter().map(OsString::from).collect();
    run_bounded(
        &config.tokenfuse_bin,
        &argv,
        &[("TOKENFUSE_DATA_DIR", data_dir.as_os_str())],
        deadline_of(started, timeout),
        timeout.as_secs(),
    )
}

/// A backtest report, or the cause it could not be read.
fn read_report(done: &Finished, asked: &BudgetCandidate) -> Result<BacktestReport, ProofError> {
    match parse_backtest(&done.stdout, asked) {
        Ok(report) => Ok(report),
        Err(BacktestParseError::Inconsistent(reason)) => Err(ProofError::Unreadable { reason }),
        Err(BacktestParseError::NoCalls) => Err(ProofError::Unreadable {
            reason: "tokenfuse found no calls in a trace that holds some",
        }),
        Err(BacktestParseError::Unrecognised) => Err(failed(done)),
    }
}

/// Prove a candidate budget against the recorded calls of one incident's run,
/// under `slots`: the request is checked first, so a request that would be
/// refused anyway takes no slot, and then one slot is held until the proof has
/// ended. No free slot is [`ProofError::Busy`], at once.
pub fn prove_budget_in(
    slots: &Slots,
    config: &BudgetConfig,
    bus: &AppState,
    request: &BudgetRequest,
    timeout: Duration,
) -> Result<BudgetProof, ProofError> {
    let candidate = BudgetCandidate::from_parts(
        request.per_run_usd.as_deref(),
        request.per_step_usd.as_deref(),
        request.max_steps,
    )
    .map_err(refused)?;
    let mut backtest = vec!["backtest".to_string()];
    backtest.extend(candidate.args().map_err(refused)?);
    let trace = config.trace_dir.as_ref().ok_or(ProofError::NotConfigured {
        missing: "the gateway's trace directory (TOKENFUSE_DATA_DIR)",
    })?;
    let run = run_of(bus, &request.incident_id)?;
    query_safe_run(&run).map_err(|reason| ProofError::RunRefused { reason })?;

    let _slot = slots
        .try_acquire()
        .map_err(|busy| ProofError::Busy { limit: busy.limit })?;

    let started = Instant::now();
    let scratch = Scratch::new().map_err(spawn_failed)?;
    let part = scratch.0.join("part.parquet");
    let part_text = part.to_str().ok_or(ProofError::RunRefused {
        reason: "the scratch path is not text",
    })?;
    let statement =
        copy_statement(&run, part_text).map_err(|reason| ProofError::RunRefused { reason })?;

    let copied = tokenfuse(
        config,
        &["sql".to_string(), statement],
        trace,
        started,
        timeout,
    )?;
    let count = parse_copy_count(&copied.stdout).ok_or_else(|| failed(&copied))?;
    if count == 0 {
        return Err(ProofError::NotInTrace {
            run: Data::bounded(&run),
        });
    }

    let counting = ["sql".to_string(), STEP_COUNT_STATEMENT.to_string()];
    let counted = tokenfuse(config, &counting, &scratch.0, started, timeout)?;
    let (rows, steps) = parse_step_counts(&counted.stdout).ok_or_else(|| failed(&counted))?;
    if rows != count {
        return Err(ProofError::Unreadable {
            reason: "the copy of the run and a count of it disagree",
        });
    }

    let alone = tokenfuse(config, &backtest, &scratch.0, started, timeout)?;
    let run_report = read_report(&alone, &candidate)?;
    let whole = tokenfuse(config, &backtest, trace, started, timeout)?;
    let trace_report = read_report(&whole, &candidate)?;

    judge_budget(
        &run,
        &candidate,
        &run_report,
        &trace_report,
        count,
        steps < rows,
    )
    .map_err(|reason| ProofError::Unreadable { reason })
}

/// The command: prove a candidate budget against an incident, on this box.
pub async fn budget_proof(
    request: BudgetRequest,
    bus: &AppState,
) -> Result<BudgetProof, ProofError> {
    budget_proof_in(
        &PROOF_SLOTS,
        BudgetConfig::from_env(),
        request,
        bus,
        BUDGET_TIMEOUT,
    )
    .await
}

/// [`budget_proof`] with its slots, configuration and deadline given.
pub async fn budget_proof_in(
    slots: &Slots,
    config: BudgetConfig,
    request: BudgetRequest,
    bus: &AppState,
    timeout: Duration,
) -> Result<BudgetProof, ProofError> {
    let bus_dir = (bus.events_dir.clone(), bus.source_events_dir.clone());
    let mode = bus.mode.clone();
    let slots = slots.clone();
    tokio::task::spawn_blocking(move || {
        let bus = AppState {
            events_dir: bus_dir.0,
            source_events_dir: bus_dir.1,
            mode,
        };
        prove_budget_in(&slots, &config, &bus, &request, timeout)
    })
    .await
    .map_err(|e| ProofError::Spawn {
        detail: Data::bounded(&e.to_string()),
    })?
}
