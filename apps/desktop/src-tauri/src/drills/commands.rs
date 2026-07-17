//! Tauri commands for the Drills view: `drills_status` plus [`drills_run`]
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
async fn ready_client(state: &tauri::State<'_, DrillsState>) -> Result<DrillsClient, DrillsError> {
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
    tauri::async_runtime::spawn_blocking(f)
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
#[tauri::command]
pub async fn drills_status(state: tauri::State<'_, DrillsState>) -> Result<DrillsStatusDto, ()> {
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
#[tauri::command(rename_all = "snake_case")]
pub async fn drills_run(
    scenario_dir: String,
    api_key: Option<String>,
    fail_on_skip: bool,
    save_path: Option<String>,
    state: tauri::State<'_, DrillsState>,
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
}
