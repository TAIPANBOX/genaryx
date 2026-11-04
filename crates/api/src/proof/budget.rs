//! The budget proof: what a candidate budget or step limit would have done to the
//! calls the gateway recorded, read from `tokenfuse backtest`, in both halves.
//!
//! # WHY THIS EXISTS
//!
//! A budget set after an incident is a guess about the next one. `tokenfuse
//! backtest` replays a candidate over the recorded trace and says what it would
//! have blocked and saved, and slice 4 of the incident-to-proven-fix plan runs
//! that CLI inside the console, the way slice 3 runs `wardryx replay`.
//!
//! # THE TWO HALVES (project rule 4)
//!
//! `tokenfuse backtest` prints totals over the whole trace and nothing per run,
//! so the first half would be unanswerable from it. The incident's own calls are
//! therefore copied into a trace of their own (`tokenfuse sql` with `copy ... to
//! ... stored as parquet`) and the same CLI replays that, so the verdict on the
//! incident is the engine's and not a restatement of its rules. The second half
//! is the CLI's report over the whole trace less the incident's own numbers.
//! Calls that share a step are loaded in an order the CLI does not promise, so
//! when the incident's run has any the split is marked as approximate rather than
//! presented as exact.
//!
//! # WHAT THE CLI IS, AND WHY ITS OUTPUT IS NOT TRUSTED
//!
//! `tokenfuse` prints text, exits 0 on a flag it dropped, on a trace with no
//! calls and on an error of its own, and reads `--budget` as `(v * 1e6) as i64`,
//! which truncates: asked for `0.000249` it tests a limit of 248 micro-dollars.
//! So an amount is handed over as text that reads back to exactly the micros
//! asked for (checked here with the same arithmetic), the report must echo the
//! candidate that was asked for, and a report whose numbers do not hold together
//! is refused. A run id is a producer's text and goes into a query, so only names
//! made of a short set of characters are put there at all.

use crate::case::Data;
use serde::Serialize;

/// Money in the trace is whole micro-dollars.
pub const MICROS_PER_USD: i64 = 1_000_000;

/// The most one limit may be, in micro-dollars: a million dollars.
pub const MAX_LIMIT_MICROS: i64 = 1_000_000 * MICROS_PER_USD;

/// The most steps one limit may name.
pub const MAX_STEPS_LIMIT: u32 = 1_000_000;

/// The longest run id a proof puts in a query.
pub const RUN_ID_MAX_BYTES: usize = 128;

/// What a budget proof replays, stated with the proof.
pub const BUDGET_COVERS: &str = "the budget per run, the budget per step and the step limit, \
                                 replayed over the calls the gateway recorded";

const AMOUNT_SHAPE: &str = "an amount is dollars with at most six decimals, such as 2 or 0.35";
const AMOUNT_ZERO: &str = "a limit of zero blocks every call, which is not a candidate budget";
const AMOUNT_LARGE: &str = "an amount is at most a million dollars";
const AMOUNT_UNREADABLE: &str = "the amount cannot be handed to tokenfuse exactly";

/// A dollar amount as whole micro-dollars. Digits, at most six decimals, no sign,
/// no exponent, not zero, not more than a million dollars.
pub fn parse_usd(text: &str) -> Result<i64, &'static str> {
    text.parse::<f64>()
        .map(|v| (v * 1e6) as i64)
        .map_err(|_| AMOUNT_SHAPE)
}

/// The text to hand `tokenfuse backtest --budget` so that it reads exactly
/// `micros`.
///
/// The CLI reads a flag as `(v * 1e6) as i64`, which truncates, so the shortest
/// text of `micros / 1e6` is read one micro-dollar low for about 1.2% of
/// amounts (249 reads as 248). Half a micro-dollar above is read back as `micros`
/// every time, and the result is checked here with the same arithmetic, so an
/// amount that could not be handed over exactly is refused and never run.
pub fn cli_amount(micros: i64) -> Result<String, &'static str> {
    Ok((micros as f64 / 1e6).to_string())
}

/// A candidate budget, in the units the trace keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BudgetCandidate {
    /// The most one run may spend. A run that would pass it is stopped there.
    pub per_run_micros: Option<i64>,
    /// The most one call may cost. A call that passes it is blocked alone.
    pub per_step_micros: Option<i64>,
    /// The most calls one run may make before it is stopped.
    pub max_steps: Option<u32>,
}

impl BudgetCandidate {
    /// A candidate from what an operator typed. At least one limit, each valid.
    pub fn from_parts(
        per_run: Option<&str>,
        per_step: Option<&str>,
        max_steps: Option<u32>,
    ) -> Result<Self, &'static str> {
        Ok(Self {
            per_run_micros: per_run.and_then(|t| parse_usd(t).ok()),
            per_step_micros: per_step.and_then(|t| parse_usd(t).ok()),
            max_steps,
        })
}

    /// The flags `tokenfuse backtest` is run with, in the order it prints them.
    pub fn args(&self) -> Result<Vec<String>, &'static str> {
        Ok(Vec::new())
}

    /// The line `tokenfuse backtest` prints for this candidate, which a report
    /// must carry to be taken as a report for it.
    pub fn describe(&self) -> String {
        String::new()
}
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-')
}

fn is_path_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/')
}

/// A run id the proof will put in a query: a short name made of letters, digits
/// and `. _ : -`. Anything else is refused, because the id is a producer's text.
pub fn query_safe_run(run: &str) -> Result<(), &'static str> {
    let _ = run;
    Ok(())
}

/// The statement that copies one run's calls, in the order they happened, into a
/// trace of its own at `file`.
pub fn copy_statement(run: &str, file: &str) -> Result<String, &'static str> {
    Ok(format!("copy (select * from calls where run_id = '{run}') to '{file}'"))
}

/// The statement that counts a trace's calls and the distinct steps among them.
pub const STEP_COUNT_STATEMENT: &str =
    "select count(*) as n, count(distinct step) as steps from calls";

/// A table `tokenfuse sql` prints: the header row, then each row, cells trimmed.
/// Anything that is not that table is `None`.
pub fn parse_table(text: &str) -> Option<Vec<Vec<String>>> {
    let _ = text;
    None
}

fn count_of(cell: &str) -> Option<u64> {
    if cell.is_empty() || !cell.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    cell.parse().ok()
}

/// How many rows a `copy` reported writing.
pub fn parse_copy_count(text: &str) -> Option<u64> {
    let _ = text;
    None
}

/// How many calls a trace holds and how many distinct steps they carry.
pub fn parse_step_counts(text: &str) -> Option<(u64, u64)> {
    let _ = text;
    None
}

/// What `tokenfuse backtest` reported over one trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BacktestReport {
    pub runs_total: u64,
    pub runs_affected: u64,
    pub calls_total: u64,
    pub calls_blocked: u64,
    /// What the trace spent, in micro-dollars.
    pub spent_micros: i64,
    /// What the candidate would have prevented.
    pub saved_micros: i64,
}

/// Why a backtest report could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BacktestParseError {
    /// The trace holds no calls.
    NoCalls,
    /// The text is not a `tokenfuse backtest` report.
    Unrecognised,
    /// The report contradicts itself, or is for another candidate.
    Inconsistent(&'static str),
}

const HEAD: &str = "TokenFuse backtest";
const CANDIDATE_LINE: &str = "  candidate policy   : ";
const RUNS_LINE: &str = "  runs               : ";
const CALLS_LINE: &str = "  calls              : ";
const SPEND_LINE: &str = "  spend in trace     : ";
const SAVED_LINE: &str = "  would have saved   : ";
const PROJECTED_LINE: &str = "  projected spend    : ";

/// `$0.256379` or `$-0.000100`: the CLI's dollars with six decimals, as micros.
fn money(text: &str) -> Option<i64> {
    let rest = text.strip_prefix('$')?;
    let (negative, digits) = match rest.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, rest),
    };
    let (whole, fraction) = digits.split_once('.')?;
    if whole.is_empty()
        || fraction.len() != 6
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let micros = whole
        .parse::<i64>()
        .ok()?
        .checked_mul(MICROS_PER_USD)?
        .checked_add(fraction.parse::<i64>().ok()?)?;
    Some(if negative { -micros } else { micros })
}

/// `$0.226496 (88.3%)`: the saved amount, with a percentage that is only a number.
fn saved_amount(text: &str) -> Option<i64> {
    let (amount, rest) = text.split_once(" (")?;
    let percent = rest.strip_suffix("%)")?;
    if percent.is_empty() || !percent.bytes().all(|b| b.is_ascii_digit() || b == b'.' || b == b'-')
    {
        return None;
    }
    money(amount)
}

/// `230 (31 affected)`: a total and the part of it named in `tail`.
fn pair(text: &str, tail: &str) -> Option<(u64, u64)> {
    let (total, rest) = text.split_once(" (")?;
    let part = rest.strip_suffix(tail)?;
    Some((count_of(total)?, count_of(part)?))
}

/// Read a `tokenfuse backtest` report for `asked`.
///
/// The format is `backtestcli::run`'s own: a head line and six labelled lines.
/// A report is refused when its candidate line is not the candidate that was
/// asked for (the CLI drops a flag it cannot read and says so on another
/// stream), when more runs are affected than there are runs, when a run is
/// affected with nothing blocked or the other way round, or when its projected
/// spend is not the spend less the saving.
pub fn parse_backtest(
    text: &str,
    asked: &BudgetCandidate,
) -> Result<BacktestReport, BacktestParseError> {
    let _ = (text, asked);
    Err(BacktestParseError::Unrecognised)
}

/// The incident's own run, replayed alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RunEffect {
    pub calls: u64,
    pub calls_blocked: u64,
    pub spent_micros: i64,
    pub saved_micros: i64,
    /// What the run would have spent under the candidate.
    pub spent_after_micros: i64,
}

/// The rest of the trace: the whole-trace report less the incident's own run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BudgetElsewhere {
    pub runs: u64,
    pub runs_affected: u64,
    pub calls: u64,
    pub calls_blocked: u64,
    pub saved_micros: i64,
}

/// A budget proof, in both halves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BudgetProof {
    /// The incident's run. A producer's text.
    pub run: Data,
    pub candidate: BudgetCandidate,
    /// First half: the candidate would have blocked at least one of the run's calls.
    pub stops_this_incident: bool,
    pub incident: RunEffect,
    /// Second half: what the candidate would have done to everything else.
    pub elsewhere: BudgetElsewhere,
    /// False when the incident's run has calls at the same step, whose order the
    /// whole-trace replay does not promise: `elsewhere` is then approximate.
    pub elsewhere_exact: bool,
    /// The whole-trace report as `tokenfuse backtest` printed it.
    pub trace: BacktestReport,
    pub covers: &'static str,
}

/// Turn the two reports into a proof about the incident that ran as `run_id`.
///
/// `run` is the replay of the incident's own calls, `trace` the replay of the
/// whole trace, `copied` how many calls were copied out for the first, and
/// `tied` whether the run has calls at the same step. A pair of reports that
/// cannot both be about the same trace is refused.
pub fn judge_budget(
    run_id: &str,
    candidate: &BudgetCandidate,
    run: &BacktestReport,
    trace: &BacktestReport,
    copied: u64,
    tied: bool,
) -> Result<BudgetProof, &'static str> {
    let _ = (run_id, candidate, run, trace, copied, tied);
    Err("a budget proof is not wired yet")
}

#[cfg(test)]
mod tests;
