//! Admission's three commands (docs/ADMISSION.md): [`admission_status`]
//! (never fails - every leg's honest resolution state), [`admission_check`]
//! (viewer-safe: is this key known to the gateway, is it bound, has it seen
//! traffic - straight off `GatewayClient::get_keys`), and
//! [`admission_baseline`] (admin-only: shells `verdryx` to run an eval THEN a
//! baseline snapshot through the gateway under the newcomer's own key, then
//! reads the result back via the EXISTING read-only
//! `genaryx_connectors::VerdryxClient`).
//!
//! ## The docs/20 pattern grammar - a faithful copy, not a shared import
//!
//! [`agent_bound_in_report`]'s `valid_pattern`/`pattern_matches` pair below is
//! a byte-for-byte copy of `crate::onboard::commands`'s own private functions
//! of the same name (the docs/20 grammar: a pattern is a literal, or a single
//! `*` as its final character). This build's own brief keeps `crate::onboard`
//! untouched (it stays offline and network-free by design), so those private
//! functions cannot be bumped to `pub(crate)` and shared the way a same-crate
//! reuse normally would be - this is the fallback the brief itself names for
//! that case: a faithful copy, with this comment naming
//! `crate::onboard::commands::{valid_pattern, pattern_matches}` as the source
//! of truth, so a later change that DOES touch onboard can lift both call
//! sites onto one shared definition. The AGGREGATION the two are used inside
//! genuinely differs (onboard's `id_bound_in_map` scans a parsed identity-map
//! FILE's `keys[].agents`; this module's [`agent_bound_in_report`] scans the
//! GATEWAY's live `keys[].agents` off `GatewayKeysReport` - the same field
//! name, a different source), so only the grammar itself is copied, not the
//! aggregator.
//!
//! ## Secret hygiene (critical - see docs/ADMISSION.md)
//!
//! `admission_baseline`'s `api_key` argument is used exactly twice: as
//! `ANTHROPIC_API_KEY`/`ANTHROPIC_BASE_URL` set ONLY on the verdryx child
//! process ([`spawn_verdryx`], via `std::process::Command::env` - never
//! `std::env::set_var` on this console's own process, and never appended to
//! the child's `args`, which would otherwise put it in that process's own
//! argv, visible to anything inspecting the OS process table on that host).
//! It is NEVER placed in any DTO field, and any subprocess stderr this module
//! captures on a failure is defensively passed through [`redact_secret`]
//! before it can reach an [`AdmissionError`] - the last line of defense
//! against an underlying SDK error message that happens to echo back the
//! credential it was given. Neither `admission_baseline` nor any other
//! command in this module ever calls `genaryx_core::command::record`: this
//! plane has no journal entry at all, mirroring `crate::drills`'s identical
//! "no journal" contract for its own `drills_run` (real side effects OUTSIDE
//! the console, but nothing here mutates any TAIPANBOX plane's governance
//! state) - see this module's own doc comment on why that matters for
//! `drills_run`'s pre-existing `api_key` argument too, which this module does
//! NOT change.

use super::env::{self, EnvSource, VerdryxDbSource};
use super::state::{AdmissionClient, AdmissionInner, AdmissionState};
use genaryx_connectors::{
    GatewayError, GatewayKeyEntry, GatewayKeysReport, VerdryxClient, VerdryxError,
};
use serde::Serialize;
use std::path::Path;

// ============================================================================
// DTOs
// ============================================================================

/// The gateway leg's own connection state - mirrors
/// `credentials::commands::CredentialsStatusDto` field-for-field (same four
/// shapes, same meaning).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GatewayStatusDto {
    Bootstrapping,
    NoEnvironment,
    Unreachable {
        source: EnvSource,
        gateway_url: String,
        reason: String,
    },
    Ready {
        source: EnvSource,
        gateway_url: String,
    },
}

/// The verdryx db leg's resolution, when one resolved at all.
#[derive(Debug, Clone, Serialize)]
pub struct VerdryxDbStatusDto {
    pub source: VerdryxDbSource,
    pub path: String,
}

/// `admission_status`'s result: every leg reported independently and
/// honestly (see `env.rs`'s module doc, "Honest per-piece resolution
/// states") - never fails.
#[derive(Debug, Clone, Serialize)]
pub struct AdmissionStatusDto {
    pub gateway: GatewayStatusDto,
    /// The one candidate path this plane looks for the `verdryx` binary at,
    /// always named even when it does not exist.
    pub verdryx_bin: String,
    pub verdryx_bin_present: bool,
    /// `None` when no `verdryx.db` candidate resolved at all.
    pub verdryx_db: Option<VerdryxDbStatusDto>,
    /// `crate::drills::env`'s own well-known scenario dir, when it exists -
    /// surfaced so the operator knows the drill leg (reusing
    /// `crate::drills::commands::drills_run` unmodified) has somewhere to
    /// run from before they click "Run drill as this key".
    pub drills_scenario_dir: Option<String>,
}

/// `admission_check`'s result: a viewer-safe read straight off the gateway's
/// key-lifecycle report, plus the docs/20 `in_map` check.
#[derive(Debug, Clone, Serialize)]
pub struct AdmissionCheckDto {
    pub key_id: String,
    pub agent_id: String,
    /// `"off" | "warn" | "enforce"` (tokenfuse docs/20), straight off the
    /// report - not a closed enum here either, same tolerance
    /// `GatewayKeysReport::strict_mode`'s own doc comment states.
    pub strict_mode: String,
    pub identity_map_configured: bool,
    /// `None` when no entry in the report has this `key_id` at all - "key
    /// unknown to the gateway", the scoreboard's most basic red flag. `Some`
    /// carries the WHOLE `GatewayKeyEntry` straight through (already
    /// `Serialize`, no UI-facing mirror struct needed - the exact
    /// `credentials::commands`/`quality::commands` precedent their own
    /// module docs name), so the frontend's existing `lib/credentials.ts`
    /// helpers (`totalCalls`, `maxLastSeenMillis`, `lastSeenLabel`) work on
    /// it unchanged.
    pub key: Option<GatewayKeyEntry>,
    /// Whether `agent_id` matches ANY `agents` pattern on ANY key entry in
    /// the report (docs/20 grammar: literal, or a single trailing `*`) - see
    /// this module's doc comment for why this is a faithful copy of
    /// `onboard::commands`'s grammar, not a shared import.
    pub in_map: bool,
}

/// `admission_baseline`'s result.
#[derive(Debug, Clone, Serialize)]
pub struct AdmissionBaselineDto {
    pub run_id: String,
    pub case_count: u64,
    /// `None` when the run scored zero cases (never a fabricated 0.0) -
    /// mirrors `genaryx_connectors::VerdryxRunSummary::mean_score`'s own
    /// honesty.
    pub mean_score: Option<f64>,
    pub total_cost_usd: f64,
    /// The parsed baseline id when `verdryx baseline`'s stdout could be read
    /// (see [`parse_baseline_id`]), else the `--label` this call requested
    /// (`admission-<agent_id>`) - still a genuine, queryable handle on the
    /// baseline even when the id itself could not be parsed.
    pub baseline_id_or_label: String,
}

/// Every error an admission command can return.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AdmissionError {
    Bootstrapping,
    NoEnvironment,
    Unreachable {
        reason: String,
    },
    /// Any gateway-side failure: transport, a plain non-2xx, or a response
    /// that failed to parse - mirrors
    /// `credentials::commands::CredentialsError::Gateway` exactly.
    Gateway {
        status: Option<u16>,
        message: String,
    },
    /// `admission_baseline` only: the resolved verdryx binary candidate does
    /// not exist as a file.
    VerdryxBinMissing {
        path: String,
    },
    /// `admission_baseline` only: no `verdryx.db` candidate resolved at all
    /// (no descriptor entry, no well-known file).
    VerdryxDbMissing,
    /// `admission_baseline` only: the verdryx CLI failed to spawn, exited
    /// non-zero, or a read-back through `VerdryxClient` failed. `message` is
    /// defensively redacted (see this module's doc comment on secret
    /// hygiene) so the operator's `api_key` can never ride back inside a
    /// captured stderr or SDK exception text.
    Verdryx {
        message: String,
    },
    /// `admission_baseline` only: verdryx's own stdout did not contain a
    /// parseable eval run id where one was required. `stdout_excerpt` is
    /// truncated and redacted the same way.
    UnparseableOutput {
        context: String,
        stdout_excerpt: String,
    },
    /// `admission_baseline` only: the eval run just written to the store has
    /// no summary immediately afterward - should not happen; reported
    /// honestly rather than fabricating one.
    RunNotFound {
        run_id: String,
    },
}

impl From<GatewayError> for AdmissionError {
    fn from(e: GatewayError) -> Self {
        match e {
            GatewayError::Transport(err) => AdmissionError::Gateway {
                status: None,
                message: format!("could not reach the gateway: {err}"),
            },
            GatewayError::Json(err) => AdmissionError::Gateway {
                status: None,
                message: format!("unexpected response shape from the gateway: {err}"),
            },
            GatewayError::Api { status, body } => AdmissionError::Gateway {
                status: Some(status),
                message: body,
            },
        }
    }
}

impl From<VerdryxError> for AdmissionError {
    fn from(e: VerdryxError) -> Self {
        AdmissionError::Verdryx {
            message: e.to_string(),
        }
    }
}

// ============================================================================
// docs/20 pattern grammar (faithful copy - see this module's doc comment)
// ============================================================================

/// A docs/20 pattern: a literal, or a single `*` as the final character.
fn valid_pattern(pattern: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    match pattern.find('*') {
        None => true,
        Some(pos) => pos == pattern.len() - 1,
    }
}

fn pattern_matches(pattern: &str, value: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => value.starts_with(prefix),
        None => pattern == value,
    }
}

/// Whether `agent_id` matches any `agents` pattern across every key entry in
/// the gateway's report - see this module's doc comment.
fn agent_bound_in_report(report: &GatewayKeysReport, agent_id: &str) -> bool {
    report
        .keys
        .iter()
        .flat_map(|k| k.agents.iter())
        .any(|p| valid_pattern(p) && pattern_matches(p, agent_id))
}

// ============================================================================
// secret hygiene helpers
// ============================================================================

/// Replace every literal occurrence of `secret` in `text` with a fixed
/// placeholder - the last line of defense so a raw child-process
/// stdout/stderr capture can never carry the operator's `api_key` back to
/// the frontend inside an error string (see this module's doc comment).  A
/// blank `secret` is left alone: an empty needle would match at every
/// position and shred the text into a wall of placeholders.
fn redact_secret(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        return text.to_string();
    }
    text.replace(secret, "[redacted]")
}

/// Truncate `s` to a bounded number of chars for embedding in an error
/// message - an eval run's stdout has one line per case and could otherwise
/// make an error message unreadably long. Char-boundary-safe (never slices
/// mid multi-byte character).
const EXCERPT_MAX_CHARS: usize = 400;

fn excerpt(s: &str) -> String {
    let mut out: String = s.chars().take(EXCERPT_MAX_CHARS).collect();
    if s.chars().count() > EXCERPT_MAX_CHARS {
        out.push_str("... [truncated]");
    }
    out
}

// ============================================================================
// verdryx stdout parsing (fixture-grounded against verdryx/cli.py, read
// 2026-07-23 - see this module's doc comment)
// ============================================================================

/// Defensively parse the eval run id out of `verdryx eval`'s stdout. Exact
/// format (`verdryx/cli.py::_cmd_eval`):
/// `"\nEval run <uuid>  (model=<model>, db=<db_path>)\n"` followed by a
/// per-case line for each score (or `"  (no cases)\n"`), then a mean-score
/// summary line. Looks for a line starting with `"Eval run "` rather than
/// assuming a fixed line number, so a future banner/warning line ahead of it
/// does not break this; the id itself is read as the first whitespace-
/// delimited token after that prefix (a uuid4 never contains whitespace, so
/// this stops cleanly before the following `"  (model=..."` text). `None`
/// when no such line is found, or the token after it is empty - the caller
/// refuses honestly rather than guessing.
fn parse_eval_run_id(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .find_map(|line| line.trim_start().strip_prefix("Eval run "))
        .and_then(|rest| rest.split_whitespace().next())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Defensively parse the baseline id out of `verdryx baseline`'s stdout.
/// Exact format (`verdryx/cli.py::_cmd_baseline`):
/// `"\nBaseline <uuid>  (run=<run_id>, mean_score=<f>)\n"`. Best-effort only,
/// unlike [`parse_eval_run_id`] (whose result is REQUIRED to run the next
/// `verdryx baseline` call): a `None` here is not fatal, the caller falls
/// back to the `--label` it requested, still a genuine, queryable handle on
/// the baseline (`genaryx_connectors::VerdryxBaseline::label`).
fn parse_baseline_id(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .find_map(|line| line.trim_start().strip_prefix("Baseline "))
        .and_then(|rest| rest.split_whitespace().next())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

// ============================================================================
// helpers
// ============================================================================

/// Resolve the current [`AdmissionClient`] out of managed state, or the
/// appropriate [`AdmissionError`] when the gateway leg is not ready - mirrors
/// `credentials::commands::ready_client` exactly (including its `&&`
/// signature: every caller already holds `state: &AdmissionState`).
async fn ready_client(state: &&AdmissionState) -> Result<AdmissionClient, AdmissionError> {
    let guard = state.inner.lock().await;
    match &*guard {
        AdmissionInner::Ready(client) => Ok(client.clone()),
        AdmissionInner::Bootstrapping => Err(AdmissionError::Bootstrapping),
        AdmissionInner::NoEnvironment => Err(AdmissionError::NoEnvironment),
        AdmissionInner::Unreachable { reason, .. } => Err(AdmissionError::Unreachable {
            reason: reason.clone(),
        }),
    }
}

/// Pure `AdmissionInner` -> `GatewayStatusDto` mapping, factored out of
/// [`admission_status`] so it is directly unit-testable - mirrors
/// `credentials::commands::status_dto`.
fn gateway_status_dto(inner: &AdmissionInner) -> GatewayStatusDto {
    match inner {
        AdmissionInner::Bootstrapping => GatewayStatusDto::Bootstrapping,
        AdmissionInner::NoEnvironment => GatewayStatusDto::NoEnvironment,
        AdmissionInner::Unreachable {
            source,
            gateway_url,
            reason,
        } => GatewayStatusDto::Unreachable {
            source: source.clone(),
            gateway_url: gateway_url.clone(),
            reason: reason.clone(),
        },
        AdmissionInner::Ready(client) => GatewayStatusDto::Ready {
            source: client.source.clone(),
            gateway_url: client.gateway_url.clone(),
        },
    }
}

/// Pure `GatewayKeysReport` -> `AdmissionCheckDto` assembly, factored out of
/// [`admission_check`] so it is directly unit-testable against a hand-built
/// report fixture, without a live gateway (see `gateway.rs`'s own tests for
/// the identical "parsed/built offline" discipline).
fn check_dto(report: &GatewayKeysReport, key_id: String, agent_id: String) -> AdmissionCheckDto {
    let key = report.keys.iter().find(|k| k.key_id == key_id).cloned();
    let in_map = agent_bound_in_report(report, &agent_id);
    AdmissionCheckDto {
        key_id,
        agent_id,
        strict_mode: report.strict_mode.clone(),
        identity_map_configured: report.identity_map_configured,
        key,
        in_map,
    }
}

/// Run a blocking admission-baseline call off the async executor thread -
/// mirrors `drills::commands::run_blocking` exactly (the "off-thread runner"
/// precedent this feature's brief names), simplified since the closure
/// already returns `Result<T, AdmissionError>` directly (no separate
/// connector error type to map afterward).
async fn run_blocking<T, F>(f: F) -> Result<T, AdmissionError>
where
    F: FnOnce() -> Result<T, AdmissionError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(e) => Err(AdmissionError::Verdryx {
            message: format!("admission baseline task failed to run: {e}"),
        }),
    }
}

/// One `verdryx <args>` invocation's stdout, on success.
#[derive(Debug)]
struct VerdryxOutput {
    stdout: String,
}

/// Shell the `verdryx` binary once, synchronously (the caller runs this
/// inside [`run_blocking`]). `gateway_url`/`api_key` are set as
/// `ANTHROPIC_BASE_URL`/`ANTHROPIC_API_KEY` ONLY on this child process via
/// `Command::env` - see this module's doc comment on secret hygiene for why
/// this is deliberately NOT an argv flag the way mockryx's `--api-key` is.
fn spawn_verdryx(
    bin: &Path,
    args: &[&str],
    gateway_url: &str,
    api_key: &str,
) -> Result<VerdryxOutput, AdmissionError> {
    let mut cmd = std::process::Command::new(bin);
    cmd.args(args);
    cmd.env("ANTHROPIC_BASE_URL", gateway_url);
    cmd.env("ANTHROPIC_API_KEY", api_key);

    let out = cmd.output().map_err(|e| AdmissionError::Verdryx {
        message: format!("could not spawn {}: {e}", bin.display()),
    })?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let safe_stderr = redact_secret(stderr.trim(), api_key);
        return Err(AdmissionError::Verdryx {
            message: format!(
                "verdryx exited {}: {safe_stderr}",
                out.status.code().unwrap_or(-1)
            ),
        });
    }

    Ok(VerdryxOutput {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
    })
}

/// The actual eval -> baseline -> read-back sequence, entirely synchronous
/// (called only from inside [`run_blocking`]'s closure).
#[allow(clippy::too_many_arguments)]
fn run_baseline_blocking(
    bin: &Path,
    evalset_path: &str,
    model: &str,
    agent_id: &str,
    api_key: &str,
    gateway_url: &str,
    db_path: &Path,
    label: &str,
) -> Result<AdmissionBaselineDto, AdmissionError> {
    let db_str = db_path.display().to_string();

    // `verdryx eval <evalset_path> --model <model> --agent-id <agent_id> --db <db>`
    let eval_args: [&str; 8] = [
        "eval",
        evalset_path,
        "--model",
        model,
        "--agent-id",
        agent_id,
        "--db",
        &db_str,
    ];
    let eval_out = spawn_verdryx(bin, &eval_args, gateway_url, api_key)?;
    let run_id =
        parse_eval_run_id(&eval_out.stdout).ok_or_else(|| AdmissionError::UnparseableOutput {
            context: "eval run id".to_string(),
            stdout_excerpt: redact_secret(&excerpt(&eval_out.stdout), api_key),
        })?;

    // `verdryx baseline <run_id> --db <db> --label admission-<agent_id>`
    let baseline_args: [&str; 6] = ["baseline", &run_id, "--db", &db_str, "--label", label];
    let baseline_out = spawn_verdryx(bin, &baseline_args, gateway_url, api_key)?;
    let baseline_id = parse_baseline_id(&baseline_out.stdout);

    let summary = VerdryxClient::open(db_path)?
        .run_summary(&run_id)?
        .ok_or_else(|| AdmissionError::RunNotFound {
            run_id: run_id.clone(),
        })?;

    Ok(AdmissionBaselineDto {
        run_id,
        case_count: summary.case_count,
        mean_score: summary.mean_score,
        total_cost_usd: summary.total_cost_usd,
        baseline_id_or_label: baseline_id.unwrap_or_else(|| label.to_string()),
    })
}

// ============================================================================
// commands
// ============================================================================

/// Whole-plane status: the gateway leg's own connection state, plus the
/// verdryx binary/db legs' independent presence and the drills scenario
/// dir's default - see this module's own doc comment and `env.rs`'s "Honest
/// per-piece resolution states". Never fails.
pub async fn admission_status(state: &AdmissionState) -> Result<AdmissionStatusDto, ()> {
    let gateway = {
        let guard = state.inner.lock().await;
        gateway_status_dto(&guard)
    };

    let bin = env::resolve_verdryx_bin();
    let db = env::resolve_verdryx_db();
    let scenario_dir = env::drills_scenario_dir_default();

    Ok(AdmissionStatusDto {
        gateway,
        verdryx_bin: bin.path.display().to_string(),
        verdryx_bin_present: bin.exists,
        verdryx_db: db.map(|d| VerdryxDbStatusDto {
            source: d.source,
            path: d.db_path.display().to_string(),
        }),
        drills_scenario_dir: scenario_dir.map(|p| p.display().to_string()),
    })
}

/// `GET /v1/keys`, projected onto one key + one candidate agent id -
/// viewer-safe (a plain read, no side effects): is `key_id` known to the
/// gateway, is it bound, has it seen traffic, and does `agent_id` match the
/// live identity map's `agents` patterns anywhere. Always a fresh read (no
/// caching): the report changes as calls come in.
pub async fn admission_check(
    key_id: String,
    agent_id: String,
    state: &AdmissionState,
) -> Result<AdmissionCheckDto, AdmissionError> {
    let client = ready_client(&state).await?;
    let report = client
        .client
        .get_keys()
        .await
        .map_err(AdmissionError::from)?;
    Ok(check_dto(&report, key_id, agent_id))
}

/// Run a Verdryx eval THROUGH the gateway under the newcomer's own key, then
/// snapshot it as a baseline, then read the result back through the
/// EXISTING read-only `VerdryxClient` - admin-only (real provider spend
/// under a fresh key, `crates/web/src/roles.rs`'s `ADMIN_COMMANDS`). See this
/// module's doc comment for the exact secret-hygiene contract on `api_key`.
pub async fn admission_baseline(
    evalset_path: String,
    model: String,
    agent_id: String,
    api_key: String,
    state: &AdmissionState,
) -> Result<AdmissionBaselineDto, AdmissionError> {
    let client = ready_client(&state).await?;
    let gateway_url = client.gateway_url.clone();

    let bin_resolution = env::resolve_verdryx_bin();
    if !bin_resolution.exists {
        return Err(AdmissionError::VerdryxBinMissing {
            path: bin_resolution.path.display().to_string(),
        });
    }
    let db_resolution = env::resolve_verdryx_db().ok_or(AdmissionError::VerdryxDbMissing)?;

    let bin = bin_resolution.path;
    let db_path = db_resolution.db_path;
    let label = format!("admission-{agent_id}");

    run_blocking(move || {
        run_baseline_blocking(
            &bin,
            &evalset_path,
            &model,
            &agent_id,
            &api_key,
            &gateway_url,
            &db_path,
            &label,
        )
    })
    .await
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use genaryx_connectors::{GatewayClient, GatewayKeyStats, GatewayUnauthorized};
    use std::sync::Arc;

    // ---- gateway_status_dto ----

    #[test]
    fn gateway_status_dto_maps_bootstrapping_and_no_environment_directly() {
        assert!(matches!(
            gateway_status_dto(&AdmissionInner::Bootstrapping),
            GatewayStatusDto::Bootstrapping
        ));
        assert!(matches!(
            gateway_status_dto(&AdmissionInner::NoEnvironment),
            GatewayStatusDto::NoEnvironment
        ));
    }

    #[test]
    fn gateway_status_dto_unreachable_preserves_source_url_and_reason() {
        let unreachable = AdmissionInner::Unreachable {
            source: EnvSource::Taipan {
                name: "p1full".to_string(),
            },
            gateway_url: "http://127.0.0.1:4100".to_string(),
            reason: "connection refused".to_string(),
        };
        match gateway_status_dto(&unreachable) {
            GatewayStatusDto::Unreachable {
                gateway_url,
                reason,
                ..
            } => {
                assert_eq!(gateway_url, "http://127.0.0.1:4100");
                assert_eq!(reason, "connection refused");
            }
            other => panic!("expected Unreachable, got {other:?}"),
        }
    }

    #[test]
    fn gateway_status_dto_ready_reports_the_gateway_url() {
        let ready = AdmissionInner::Ready(AdmissionClient {
            client: Arc::new(GatewayClient::new("http://127.0.0.1:4100").expect("build a client")),
            source: EnvSource::Taipan {
                name: "p1full".to_string(),
            },
            gateway_url: "http://127.0.0.1:4100".to_string(),
        });
        match gateway_status_dto(&ready) {
            GatewayStatusDto::Ready { gateway_url, .. } => {
                assert_eq!(gateway_url, "http://127.0.0.1:4100");
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    // ---- error mapping ----

    #[test]
    fn admission_error_from_gateway_error_preserves_status_and_message() {
        let e = AdmissionError::from(GatewayError::Api {
            status: 404,
            body: "not found".to_string(),
        });
        match e {
            AdmissionError::Gateway {
                status: Some(404),
                message,
            } => assert_eq!(message, "not found"),
            other => panic!("expected Gateway{{404,..}}, got {other:?}"),
        }
    }

    #[test]
    fn admission_error_from_json_error_has_no_status() {
        let json_err = serde_json::from_str::<GatewayKeysReport>("not json").unwrap_err();
        let e = AdmissionError::from(GatewayError::from(json_err));
        assert!(matches!(e, AdmissionError::Gateway { status: None, .. }));
    }

    #[test]
    fn admission_error_from_verdryx_error_carries_a_message() {
        // A genuine VerdryxError (missing-file open failure), same fixture
        // `VerdryxClient`'s own `open_missing_db_is_fail_closed` test uses.
        let path = std::env::temp_dir().join("genaryx-admission-commands-test-does-not-exist.db");
        let _ = std::fs::remove_file(&path);
        let err = VerdryxClient::open(&path).expect_err("a missing file must fail to open");

        let mapped = AdmissionError::from(err);
        let AdmissionError::Verdryx { message } = mapped else {
            panic!("expected a Verdryx-shaped AdmissionError")
        };
        assert!(!message.is_empty());
    }

    // ---- docs/20 grammar (ported from onboard::commands - see this
    // module's doc comment) ----

    #[test]
    fn valid_pattern_accepts_literals_and_one_trailing_star() {
        assert!(valid_pattern("agent://acme.local/finance/billing-agent"));
        assert!(valid_pattern("agent://acme.local/finance/*"));
        assert!(valid_pattern("*"));
    }

    #[test]
    fn valid_pattern_rejects_empty_and_a_star_anywhere_but_the_end() {
        assert!(!valid_pattern(""));
        assert!(!valid_pattern("agent://acme.local/*/billing-agent"));
        assert!(!valid_pattern("agent://acme.local/finance/**"));
    }

    #[test]
    fn pattern_matches_literal_requires_exact_equality() {
        assert!(pattern_matches("agent://a/b", "agent://a/b"));
        assert!(!pattern_matches("agent://a/b", "agent://a/bc"));
    }

    #[test]
    fn pattern_matches_trailing_star_is_a_prefix_match() {
        assert!(pattern_matches(
            "agent://acme.local/finance/*",
            "agent://acme.local/finance/billing-agent"
        ));
        assert!(!pattern_matches(
            "agent://acme.local/finance/*",
            "agent://acme.local/hr/billing-agent"
        ));
    }

    fn key_entry(key_id: &str, bound: bool, agents: &[&str]) -> GatewayKeyEntry {
        GatewayKeyEntry {
            key_id: key_id.to_string(),
            configured: true,
            bound,
            unit: Some("finance".to_string()),
            agents: agents.iter().map(|s| s.to_string()).collect(),
            created: Some("2026-07-01".to_string()),
            since_startup: GatewayKeyStats {
                calls: 42,
                identity_mismatches: 0,
                first_seen_millis: None,
                last_seen_millis: Some(1_753_000_000_000),
            },
            history: None,
        }
    }

    fn fixture_report(keys: Vec<GatewayKeyEntry>) -> GatewayKeysReport {
        GatewayKeysReport {
            strict_mode: "warn".to_string(),
            identity_map_configured: true,
            history_available: false,
            unauthorized_since_startup: GatewayUnauthorized {
                attempts: 0,
                last_millis: None,
            },
            keys,
        }
    }

    #[test]
    fn agent_bound_in_report_matches_across_any_key_entry() {
        let report = fixture_report(vec![
            key_entry("other-key", true, &["agent://acme.local/hr/*"]),
            key_entry(
                "billing-agent",
                true,
                &["agent://acme.local/finance/billing-agent"],
            ),
        ]);
        assert!(agent_bound_in_report(
            &report,
            "agent://acme.local/finance/billing-agent"
        ));
        assert!(!agent_bound_in_report(
            &report,
            "agent://acme.local/finance/other-agent"
        ));
    }

    // ---- check_dto assembly (against a hand-built report, no live gateway -
    // see this module's doc comment) ----

    #[test]
    fn check_dto_finds_the_matching_key_and_computes_in_map() {
        let report = fixture_report(vec![key_entry(
            "billing-agent",
            true,
            &["agent://acme.local/finance/billing-agent"],
        )]);
        let dto = check_dto(
            &report,
            "billing-agent".to_string(),
            "agent://acme.local/finance/billing-agent".to_string(),
        );
        assert_eq!(dto.strict_mode, "warn");
        assert!(dto.identity_map_configured);
        let key = dto.key.expect("key must be found");
        assert!(key.configured);
        assert!(key.bound);
        assert_eq!(key.unit.as_deref(), Some("finance"));
        assert!(dto.in_map);
    }

    #[test]
    fn check_dto_reports_key_unknown_to_the_gateway_as_a_clean_none() {
        let report = fixture_report(vec![key_entry(
            "some-other-key",
            true,
            &["agent://acme.local/finance/*"],
        )]);
        let dto = check_dto(
            &report,
            "newcomer-key".to_string(),
            "agent://acme.local/finance/newcomer".to_string(),
        );
        assert!(
            dto.key.is_none(),
            "an unconfigured key_id must yield None, not an error"
        );
        // The agent id still happens to match another key's pattern here -
        // in_map is independent of whether THIS key_id itself was found.
        assert!(dto.in_map);
    }

    #[test]
    fn check_dto_in_map_is_false_when_no_pattern_matches_anywhere() {
        let report = fixture_report(vec![key_entry(
            "billing-agent",
            true,
            &["agent://acme.local/finance/billing-agent"],
        )]);
        let dto = check_dto(
            &report,
            "billing-agent".to_string(),
            "agent://acme.local/hr/newcomer".to_string(),
        );
        assert!(dto.key.is_some());
        assert!(!dto.in_map);
    }

    // ---- verdryx stdout parsing (fixture strings from the real output
    // format - verdryx/cli.py, read 2026-07-23) ----

    #[test]
    fn parse_eval_run_id_reads_the_real_cli_shape() {
        let stdout = "\nEval run 3fa85f64-5717-4562-b3fc-2c963f66afa6  (model=claude-sonnet-5, db=/root/.taipan/verdryx.db)\n\n  [0.90] case-01\n  [0.80] case-02\n\n  mean score: 0.850   cases: 2   tokens: 4096\n";
        assert_eq!(
            parse_eval_run_id(stdout).as_deref(),
            Some("3fa85f64-5717-4562-b3fc-2c963f66afa6")
        );
    }

    #[test]
    fn parse_eval_run_id_handles_the_no_cases_shape() {
        let stdout = "\nEval run 11111111-1111-1111-1111-111111111111  (model=stub, db=verdryx.db)\n\n  (no cases)\n";
        assert_eq!(
            parse_eval_run_id(stdout).as_deref(),
            Some("11111111-1111-1111-1111-111111111111")
        );
    }

    #[test]
    fn parse_eval_run_id_is_none_when_the_prefix_never_appears() {
        assert!(parse_eval_run_id("error: no such evalset: /nope.json\n").is_none());
        assert!(parse_eval_run_id("").is_none());
    }

    #[test]
    fn parse_baseline_id_reads_the_real_cli_shape() {
        let stdout = "\nBaseline 22222222-2222-2222-2222-222222222222  (run=3fa85f64-5717-4562-b3fc-2c963f66afa6, mean_score=0.850)\n";
        assert_eq!(
            parse_baseline_id(stdout).as_deref(),
            Some("22222222-2222-2222-2222-222222222222")
        );
    }

    #[test]
    fn parse_baseline_id_is_none_when_unparseable_so_the_caller_falls_back_to_the_label() {
        assert!(parse_baseline_id("error: no such eval run: 'bogus'\n").is_none());
    }

    // ---- secret hygiene ----

    #[test]
    fn redact_secret_removes_every_occurrence() {
        let secret = "sk-ant-supersecrettoken";
        let text = format!(
            "verdryx exited 1: invalid key {secret} rejected (retried with {secret} again)"
        );
        let safe = redact_secret(&text, secret);
        assert!(!safe.contains(secret), "secret must not survive redaction");
        assert!(safe.contains("[redacted]"));
    }

    #[test]
    fn redact_secret_is_a_noop_for_an_empty_secret() {
        assert_eq!(redact_secret("hello there", ""), "hello there");
    }

    #[test]
    fn admission_error_verdryx_variant_never_carries_the_raw_secret_once_redacted() {
        // The task's own "does the secret appear in a serialized error"
        // check, applied at the one place this module ever builds an
        // AdmissionError from untrusted subprocess text - see this module's
        // doc comment on why there is no CommandRecord/journal call at all
        // for this command to intercept instead (drills_run's identical "no
        // journal" contract).
        let secret = "sk-ant-do-not-leak-me";
        let raw_stderr = format!("anthropic.AuthenticationError: invalid x-api-key {secret}");
        let err = AdmissionError::Verdryx {
            message: format!("verdryx exited 1: {}", redact_secret(&raw_stderr, secret)),
        };
        let serialized = serde_json::to_string(&err).expect("serialize");
        assert!(
            !serialized.contains(secret),
            "the api key must never appear in a serialized AdmissionError: {serialized}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn spawn_verdryx_redacts_the_api_key_out_of_a_nonzero_exit_stderr() {
        // A fake "verdryx" that echoes the api key it was actually given
        // (via the env var, never argv - see this module's doc comment) back
        // to stderr and exits 1, simulating the realistic leak vector (an
        // underlying SDK's own exception text echoing the credential it was
        // called with) rather than only unit-testing `redact_secret` in
        // isolation.
        use std::os::unix::fs::PermissionsExt;

        let script = std::env::temp_dir().join(format!(
            "genaryx-admission-fake-verdryx-{}-{}.sh",
            std::process::id(),
            nanos()
        ));
        std::fs::write(
            &script,
            "#!/bin/sh\necho \"auth failed for key $ANTHROPIC_API_KEY\" 1>&2\nexit 1\n",
        )
        .expect("write fixture script");
        let mut perms = std::fs::metadata(&script)
            .expect("stat fixture script")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).expect("chmod fixture script executable");

        let secret = "sk-ant-totally-real-secret-99";
        let err = spawn_verdryx(&script, &["eval"], "http://127.0.0.1:4100", secret)
            .expect_err("the fake script always exits 1");
        let AdmissionError::Verdryx { message } = err else {
            panic!("expected a Verdryx-shaped AdmissionError")
        };
        assert!(
            !message.contains(secret),
            "the api key leaked into the error message: {message}"
        );
        assert!(message.contains("[redacted]"));

        let _ = std::fs::remove_file(&script);
    }

    #[cfg(unix)]
    fn nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }

    // ---- excerpt ----

    #[test]
    fn excerpt_passes_short_text_through_unchanged() {
        assert_eq!(excerpt("short"), "short");
    }

    #[test]
    fn excerpt_truncates_long_text_at_a_char_boundary() {
        let long = "x".repeat(EXCERPT_MAX_CHARS + 50);
        let got = excerpt(&long);
        assert!(got.ends_with("... [truncated]"));
        assert!(got.len() < long.len());
    }
}
