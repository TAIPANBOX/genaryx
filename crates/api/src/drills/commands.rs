//! Console commands for the Drills view: `drills_status` plus [`drills_run`]
//! (docs/PHASE4.md W2) - a `mockryx run --format json` batch, off the async
//! executor thread (mockryx's own process spawn+wait is blocking IO, same
//! discipline `crypto::commands`'s qryx scans follow).
//!
//! Genuinely on-demand, same as Crypto's qryx: mockryx has no live feed at
//! all, so there is no "as of load" snapshot to keep fresh - the frontend
//! labels every result "as of last run <time>" and nothing here ever
//! auto-triggers a run on its own (docs/PHASE4.md Drills position 1: "never
//! auto-run").
//!
//! Read-only in the sense that matters here (no plane-mutating command, no
//! `genaryx_core::command::record` journal entry): a drill run has real side
//! effects OUTSIDE the console (it makes live calls against the TokenFuse
//! gateway and burns real budget, per `MockryxReport.results[].metrics` -
//! that IS the point of a fire drill) but never touches any TAIPANBOX
//! plane's governance state the way Money's kill/set-budget do, so this
//! mirrors Crypto/Quality's "no journal" contract, not Money/Policy's
//! mutation one.

use super::env::EnvSource;
use super::state::{DrillsClient, DrillsInner, DrillsState};
use genaryx_connectors::{MockryxError, MockryxReport};
use serde::Serialize;
use std::path::{Path, PathBuf};

// ============================================================================
// DTOs
// ============================================================================

/// Whole-panel connection state - mirrors `crypto::commands::CryptoStatusDto`,
/// minus `Unreachable` (see `state.rs`'s module doc for why Drills has none).
/// `has_api_key` reports only WHETHER a bearer resolved, never the value
/// itself - the same discipline `MoneyStatusDto`/`PolicyStatusDto` follow for
/// their own admin bearers (never put a secret on the IPC wire beyond what a
/// command genuinely needs it for).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DrillsStatusDto {
    Bootstrapping,
    NoEnvironment,
    Ready {
        source: EnvSource,
        mockryx_bin: String,
        gateway_url: String,
        has_api_key: bool,
        scenario_dir: Option<String>,
    },
}

/// Every error a drills command can return - mirrors
/// `crypto::commands::CryptoError`'s shape: `MockryxError` carries no
/// HTTP-style status to preserve either, just a message.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DrillsError {
    Bootstrapping,
    NoEnvironment,
    Mockryx { message: String },
}

impl From<MockryxError> for DrillsError {
    fn from(e: MockryxError) -> Self {
        DrillsError::Mockryx {
            message: e.to_string(),
        }
    }
}

// ============================================================================
// helpers
// ============================================================================

/// Resolve the current [`DrillsClient`] out of managed state, or the
/// appropriate [`DrillsError`] when the panel is not ready - mirrors
/// `crypto::commands::ready_client` exactly.
async fn ready_client(state: &&DrillsState) -> Result<DrillsClient, DrillsError> {
    let guard = state.inner.lock().await;
    match &*guard {
        DrillsInner::Ready(client) => Ok(client.clone()),
        DrillsInner::Bootstrapping => Err(DrillsError::Bootstrapping),
        DrillsInner::NoEnvironment => Err(DrillsError::NoEnvironment),
    }
}

/// Pure `DrillsInner` -> `DrillsStatusDto` mapping, factored out of
/// [`drills_status`] so it is directly unit-testable - same rationale as
/// `crypto::commands::status_dto`.
fn status_dto(inner: &DrillsInner) -> DrillsStatusDto {
    match inner {
        DrillsInner::Bootstrapping => DrillsStatusDto::Bootstrapping,
        DrillsInner::NoEnvironment => DrillsStatusDto::NoEnvironment,
        DrillsInner::Ready(client) => DrillsStatusDto::Ready {
            source: client.source.clone(),
            mockryx_bin: client.mockryx_bin.display().to_string(),
            gateway_url: client.gateway_url.clone(),
            has_api_key: client.api_key.is_some(),
            scenario_dir: client
                .scenario_dir
                .as_ref()
                .map(|p| p.display().to_string()),
        },
    }
}

/// Run a blocking mockryx call off the async executor thread - shared by
/// every command below.
async fn run_blocking<T, F>(f: F) -> Result<T, DrillsError>
where
    F: FnOnce() -> Result<T, MockryxError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| DrillsError::Mockryx {
            message: format!("drills run task failed to run: {e}"),
        })?
        .map_err(DrillsError::from)
}

// ============================================================================
// commands
// ============================================================================

/// Whole-panel connection state. Never fails: every outcome of
/// [`super::state::bootstrap`] is a renderable [`DrillsStatusDto`] variant.
pub async fn drills_status(state: &DrillsState) -> Result<DrillsStatusDto, ()> {
    let guard = state.inner.lock().await;
    Ok(status_dto(&guard))
}

/// `mockryx run --gateway <gateway> --format json [--api-key K]
/// [--fail-on-skip] [--save P] <scenario_dir>` (docs/PHASE4.md W2 Drills
/// position 1) - never auto-run, only on an explicit operator click. The
/// gateway is always the resolved environment's own (not overridable per
/// call - it identifies WHICH plane this run targets, the same way Money's
/// `cloud_url` is fixed once paired). `api_key`/`save_path` are optional
/// per-call overrides: a blank string means "use the resolved environment's
/// own value" (so the frontend can leave its override fields empty without
/// accidentally clearing a resolved bearer or disabling the save).
pub async fn drills_run(
    scenario_dir: String,
    api_key: Option<String>,
    fail_on_skip: bool,
    save_path: Option<String>,
    state: &DrillsState,
) -> Result<MockryxReport, DrillsError> {
    let client = ready_client(&state).await?;
    let gateway_url = client.gateway_url.clone();
    let key = api_key
        .filter(|k| !k.trim().is_empty())
        .or_else(|| client.api_key.clone());

    run_blocking(move || {
        let save: Option<PathBuf> = save_path
            .filter(|p| !p.trim().is_empty())
            .map(PathBuf::from);
        client.client.run(
            Path::new(&scenario_dir),
            &gateway_url,
            key.as_deref(),
            fail_on_skip,
            save.as_deref(),
        )
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use genaryx_connectors::MockryxClient;

    #[test]
    fn status_dto_maps_bootstrapping_and_no_environment_directly() {
        assert!(matches!(
            status_dto(&DrillsInner::Bootstrapping),
            DrillsStatusDto::Bootstrapping
        ));
        assert!(matches!(
            status_dto(&DrillsInner::NoEnvironment),
            DrillsStatusDto::NoEnvironment
        ));
    }

    #[test]
    fn status_dto_ready_reports_has_api_key_honestly() {
        let with_key = DrillsInner::Ready(DrillsClient {
            client: MockryxClient::new("/tmp/mockryx"),
            source: EnvSource::Taipan {
                name: "p1full".to_string(),
            },
            mockryx_bin: PathBuf::from("/tmp/mockryx"),
            gateway_url: "http://127.0.0.1:41000".to_string(),
            api_key: Some("tp_deadbeef".to_string()),
            scenario_dir: Some(PathBuf::from("/tmp/scenarios")),
        });
        match status_dto(&with_key) {
            DrillsStatusDto::Ready {
                gateway_url,
                has_api_key,
                scenario_dir,
                ..
            } => {
                assert_eq!(gateway_url, "http://127.0.0.1:41000");
                assert!(has_api_key);
                assert_eq!(scenario_dir.as_deref(), Some("/tmp/scenarios"));
            }
            other => panic!("expected Ready, got {other:?}"),
        }

        let without_key = DrillsInner::Ready(DrillsClient {
            client: MockryxClient::new("/tmp/mockryx"),
            source: EnvSource::Taipan {
                name: "p1full".to_string(),
            },
            mockryx_bin: PathBuf::from("/tmp/mockryx"),
            gateway_url: "http://127.0.0.1:41000".to_string(),
            api_key: None,
            scenario_dir: None,
        });
        match status_dto(&without_key) {
            DrillsStatusDto::Ready {
                has_api_key,
                scenario_dir,
                ..
            } => {
                assert!(!has_api_key);
                assert!(scenario_dir.is_none());
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn drills_error_from_mockryx_error_carries_a_message() {
        // A binary that cannot spawn -> a genuine MockryxError::Spawn, same
        // fixture `MockryxClient`'s own tests use.
        let c = MockryxClient::new("/nonexistent/mockryx-binary-xyz");
        let err = c
            .run(
                Path::new("/scenarios"),
                "http://127.0.0.1:4100",
                None,
                false,
                None,
            )
            .expect_err("a nonexistent binary must fail to spawn");

        let mapped = DrillsError::from(err);
        let DrillsError::Mockryx { message } = mapped else {
            panic!("expected a Mockryx-shaped DrillsError")
        };
        assert!(!message.is_empty());
    }

    // ---- drills_run end to end against a stand-in mockryx ----
    //
    // The same hermetic shape `admission::commands` uses for verdryx: a shell
    // script in a temp dir plays the CLI and records what it was handed.

    const FALLBACK_KEY: &str = "test-key-not-a-secret-fallback";
    const GATEWAY: &str = "http://127.0.0.1:41000";

    #[cfg(unix)]
    fn scratch_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "genaryx-drills-run-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// A stand-in mockryx that writes its argv and the key it was given to
    /// `args.log`, then runs `tail` (the report and exit code).
    #[cfg(unix)]
    fn fake_mockryx(dir: &Path, tail: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("mockryx");
        let log = dir.join("args.log");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\n\
                 echo \"$@\" > '{log}'\n\
                 echo \"key=$MOCKRYX_API_KEY\" >> '{log}'\n\
                 {tail}\n",
                log = log.display()
            ),
        )
        .expect("write the stand-in");
        let mut perms = std::fs::metadata(&script).expect("stat").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).expect("chmod");
        script
    }

    fn ready_state(bin: &Path) -> DrillsState {
        DrillsState {
            inner: tokio::sync::Mutex::new(DrillsInner::Ready(DrillsClient {
                client: MockryxClient::new(bin),
                source: EnvSource::Taipan {
                    name: "lab".to_string(),
                },
                mockryx_bin: bin.to_path_buf(),
                gateway_url: GATEWAY.to_string(),
                api_key: Some(FALLBACK_KEY.to_string()),
                scenario_dir: None,
            })),
        }
    }

    /// `drills_run`, retried only on ETXTBSY (another test thread forking
    /// while the script was still open for writing).
    #[cfg(unix)]
    async fn run_fake(
        api_key: Option<&str>,
        save: Option<&str>,
        fail_on_skip: bool,
        state: &DrillsState,
    ) -> Result<MockryxReport, DrillsError> {
        let mut last = None;
        for _ in 0..5 {
            let result = drills_run(
                "/tmp/genaryx-drills-scenarios".to_string(),
                api_key.map(str::to_string),
                fail_on_skip,
                save.map(str::to_string),
                state,
            )
            .await;
            let busy = matches!(
                &result,
                Err(DrillsError::Mockryx { message }) if message.contains("os error 26")
            );
            if !busy {
                return result;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            last = Some(result);
        }
        last.expect("at least one attempt")
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_blank_key_and_save_path_fall_back_and_a_gap_report_is_still_a_report() {
        let dir = scratch_dir("fallback");
        let bin = fake_mockryx(
            &dir,
            "printf '{\"run_id\":\"r-drill\",\"gateway\":\"%s\",\"generated_at\":\"t\",\"results\":[]}' \"$3\"\nexit 1",
        );
        let state = ready_state(&bin);
        let report = run_fake(Some("   "), Some("  "), true, &state)
            .await
            .expect("exit 1 is a report with gaps, not an error");
        assert_eq!(report.run_id, "r-drill");
        assert_eq!(
            report.gateway, GATEWAY,
            "the client's gateway is the one drilled"
        );
        let log = std::fs::read_to_string(dir.join("args.log")).expect("the stand-in ran");
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(
            lines,
            [
                "run --gateway http://127.0.0.1:41000 --format json --fail-on-skip /tmp/genaryx-drills-scenarios",
                "key=test-key-not-a-secret-fallback",
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_explicit_key_and_save_path_are_handed_over_and_a_failed_run_is_an_error() {
        let dir = scratch_dir("explicit");
        let bin = fake_mockryx(&dir, "echo 'no scenarios here' >&2\nexit 2");
        let state = ready_state(&bin);
        let save = dir.join("report.json").display().to_string();
        match run_fake(Some("test-key-explicit"), Some(&save), false, &state).await {
            Err(DrillsError::Mockryx { message }) => {
                assert_eq!(message, "mockryx exited 2: no scenarios here");
            }
            other => panic!("expected a Mockryx error, got {other:?}"),
        }
        let log = std::fs::read_to_string(dir.join("args.log")).expect("the stand-in ran");
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            format!(
                "run --gateway http://127.0.0.1:41000 --format json --save {save} /tmp/genaryx-drills-scenarios"
            )
        );
        assert_eq!(lines[1], "key=test-key-explicit");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_binary_that_cannot_spawn_is_a_mockryx_error_from_the_command() {
        let state = ready_state(Path::new("/nonexistent/genaryx-drills-test/mockryx"));
        match drills_run("/tmp/s".to_string(), None, false, None, &state).await {
            Err(DrillsError::Mockryx { message }) => {
                assert!(message.starts_with("mockryx spawn "), "{message}");
            }
            other => panic!("expected a Mockryx error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_plane_that_is_not_ready_refuses_the_run_and_reports_its_state() {
        let pending = DrillsState::pending();
        assert!(matches!(
            drills_run("/tmp/s".to_string(), None, false, None, &pending).await,
            Err(DrillsError::Bootstrapping)
        ));
        assert!(matches!(
            drills_status(&pending).await,
            Ok(DrillsStatusDto::Bootstrapping)
        ));
        let none = DrillsState::pending();
        *none.inner.lock().await = DrillsInner::NoEnvironment;
        assert!(matches!(
            drills_run("/tmp/s".to_string(), None, false, None, &none).await,
            Err(DrillsError::NoEnvironment)
        ));
        assert!(matches!(
            drills_status(&none).await,
            Ok(DrillsStatusDto::NoEnvironment)
        ));
        let ready = ready_state(Path::new("/opt/mockryx"));
        match drills_status(&ready).await {
            Ok(DrillsStatusDto::Ready {
                mockryx_bin,
                has_api_key,
                ..
            }) => {
                assert_eq!(mockryx_bin, "/opt/mockryx");
                assert!(has_api_key);
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }
}
