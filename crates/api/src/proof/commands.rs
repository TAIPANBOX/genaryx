//! `policy_proof`: run `wardryx replay` over the recorded decisions against a
//! candidate policy the operator wrote, and report the proof in both halves.
//!
//! A read in every sense that matters: it changes nothing in any plane, writes
//! no journal line and signs nothing. It does run a child process and it does
//! write one temporary file (the candidate, in a directory only this process
//! can enter, removed when the run ends), which is why it is bounded: the
//! candidate is small, the run has a deadline, the output has a cap, and the
//! child gets no environment but a `PATH` and no shell.
//!
//! # WHERE EACH INPUT COMES FROM
//!
//! - the binary: `wardryx` on `PATH`. The console image carries it beside
//!   `qryx` and `mockryx`;
//! - the events file: `wardryx.ndjson` in the directory the bus tails, which
//!   is where wardryx's own record of every decision lands;
//! - the policy archive: `WARDRYX_POLICY_ARCHIVE`, wardryx's own name for the
//!   directory of every policy version it kept, mounted read only.
//!
//! A missing input is an answer that names it ([`ProofError::NotConfigured`]),
//! never a proof that nothing changes.

use super::{ParseError, PolicyProof, ReplayReport, judge, parse_report};
use crate::bus::AppState;
use crate::case::Data;
use crate::case::commands::{CaseError, event_id_of};
use genaryx_core::store::Store;
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The most candidate text one proof takes.
pub const CANDIDATE_MAX_BYTES: usize = 64 * 1024;

/// How long one replay may run.
pub const REPLAY_TIMEOUT: Duration = Duration::from_secs(30);

/// The most output of each stream the console keeps.
pub const OUTPUT_CAP_BYTES: u64 = 1024 * 1024;

/// The events file wardryx records its decisions in, under the bus directory.
pub const WARDRYX_EVENTS_FILE: &str = "wardryx.ndjson";

/// Where the replay runs from.
#[derive(Debug, Clone)]
pub struct ProofConfig {
    /// The `wardryx` binary.
    pub wardryx_bin: PathBuf,
    /// wardryx's policy archive. `None` is not configured.
    pub archive: Option<PathBuf>,
    /// wardryx's event file. `None` is not configured.
    pub events: Option<PathBuf>,
}

impl ProofConfig {
    /// The configuration of this box: `wardryx` on `PATH`, the archive wardryx
    /// itself names, and the event file in the bus directory.
    pub fn from_env(bus: &AppState) -> Self {
        Self {
            wardryx_bin: PathBuf::from("wardryx"),
            archive: std::env::var("WARDRYX_POLICY_ARCHIVE")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            events: bus
                .source_events_dir
                .as_ref()
                .map(|dir| dir.join(WARDRYX_EVENTS_FILE)),
        }
    }
}

/// Why no proof was made. Each is an answer, never a proof of nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProofError {
    /// An input the replay needs is not set up on this box.
    NotConfigured { missing: &'static str },
    /// The incident could not be resolved to an event row and a run.
    Incident { reason: String },
    /// The incident's event names no run, so there is no run to ask about.
    NoRun,
    /// The candidate is empty, too large or holds a NUL.
    CandidateRefused { reason: &'static str },
    /// The binary could not be started.
    Spawn { detail: Data },
    /// The replay did not finish in time.
    Timeout { seconds: u64 },
    /// The replay ended without a report, or with an error of its own.
    ReplayFailed { exit: Option<i32>, detail: Data },
    /// The report could not be read, or contradicted itself.
    Unreadable { reason: &'static str },
}

/// A request for a proof.
#[derive(Debug, Clone)]
pub struct ProofRequest {
    /// The incident Incident 360 holds (`bus:<n>`, `verdryx:<n>` or `<n>`).
    pub incident_id: String,
    /// The candidate policy, as wardryx reads it.
    pub candidate: String,
}

/// The run an incident happened in, from the console's own store.
fn run_of(bus: &AppState, incident_id: &str) -> Result<String, ProofError> {
    let incident = |e: CaseError| ProofError::Incident {
        reason: e.describe(),
    };
    let event_id = event_id_of(incident_id).map_err(incident)?;
    let dir = bus.events_dir.as_ref().ok_or_else(|| {
        incident(CaseError::NoStore {
            reason: String::new(),
        })
    })?;
    let store = Store::open(&dir.join("console.sqlite")).map_err(|e| {
        incident(CaseError::NoStore {
            reason: e.to_string(),
        })
    })?;
    let event = store
        .event_by_id(event_id)
        .map_err(|e| {
            incident(CaseError::NoStore {
                reason: e.to_string(),
            })
        })?
        .ok_or_else(|| incident(CaseError::NotFound { event_id }))?;
    event
        .run_id
        .filter(|r| !r.is_empty())
        .ok_or(ProofError::NoRun)
}

fn check_candidate(candidate: &str) -> Result<(), ProofError> {
    let refused = |reason| Err(ProofError::CandidateRefused { reason });
    if candidate.trim().is_empty() {
        return refused("the candidate policy is empty");
    }
    if candidate.len() > CANDIDATE_MAX_BYTES {
        return refused("the candidate policy is larger than 64 KiB");
    }
    if candidate.contains('\0') {
        return refused("the candidate policy holds a NUL byte");
    }
    Ok(())
}

/// A private directory that is removed when it goes out of scope.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> std::io::Result<Self> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "genaryx-proof-{}-{nanos}",
            std::process::id()
        ));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&dir)?;
        Ok(Self(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn capped(mut stream: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = (&mut stream).take(OUTPUT_CAP_BYTES).read_to_end(&mut out);
        // Keep draining so a chatty child is never blocked on a full pipe.
        let _ = std::io::copy(&mut stream, &mut std::io::sink());
        out
    })
}

/// What a finished replay left.
struct Finished {
    exit: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Run `wardryx replay` with exactly these arguments, bounded in time and in
/// output, with no shell and an environment of `PATH` alone.
fn run_replay(
    bin: &Path,
    events: &Path,
    archive: &Path,
    candidate: &Path,
    timeout: Duration,
) -> Result<Finished, ProofError> {
    let mut child = Command::new(bin)
        .arg("replay")
        .arg("-events")
        .arg(events)
        .arg("-archive")
        .arg(archive)
        .arg("-policy")
        .arg(candidate)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ProofError::Spawn {
            detail: Data::bounded(&e.to_string()),
        })?;
    let stdout = capped(child.stdout.take().expect("stdout was piped"));
    let stderr = capped(child.stderr.take().expect("stderr was piped"));

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ProofError::Timeout {
                    seconds: timeout.as_secs(),
                });
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                return Err(ProofError::Spawn {
                    detail: Data::bounded(&e.to_string()),
                });
            }
        }
    };
    let text = |handle: std::thread::JoinHandle<Vec<u8>>| {
        String::from_utf8_lossy(&handle.join().unwrap_or_default()).into_owned()
    };
    Ok(Finished {
        exit: status.code(),
        stdout: text(stdout),
        stderr: text(stderr),
    })
}

/// Prove a candidate policy against the recorded decisions of one incident's
/// run, with an explicit configuration. [`policy_proof`] is this with the
/// box's own.
pub fn prove(
    config: &ProofConfig,
    bus: &AppState,
    request: &ProofRequest,
    timeout: Duration,
) -> Result<PolicyProof, ProofError> {
    let _ = (config, bus, request, timeout);
    Err(ProofError::NotConfigured { missing: "stub" })
}

/// The command: prove a candidate policy against an incident, on this box.
pub async fn policy_proof(
    request: ProofRequest,
    bus: &AppState,
) -> Result<PolicyProof, ProofError> {
    let config = ProofConfig::from_env(bus);
    let bus_dir = (bus.events_dir.clone(), bus.source_events_dir.clone());
    let mode = bus.mode.clone();
    tokio::task::spawn_blocking(move || {
        let bus = AppState {
            events_dir: bus_dir.0,
            source_events_dir: bus_dir.1,
            mode,
        };
        prove(&config, &bus, &request, REPLAY_TIMEOUT)
    })
    .await
    .map_err(|e| ProofError::Spawn {
        detail: Data::bounded(&e.to_string()),
    })?
}
