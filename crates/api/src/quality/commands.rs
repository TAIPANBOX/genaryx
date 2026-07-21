//! Tauri commands for the Quality view: `quality_status` plus three plain
//! reads over Verdryx's quality plane (docs/PHASE4.md W1) -
//! [`quality_list_run_summaries`] (eval-run history, each row pre-joined
//! with its own case_count/mean_score/total_cost so the Eval-runs table
//! never needs a second round trip per row), [`quality_run_scores`] (one
//! run's per-case detail table), and [`quality_list_baselines`].
//!
//! Drift alerts are deliberately NOT a command here at all: they are the
//! live `quality_drift` bus event, already flowing through the existing
//! `recent_events`/`bus:event` pipeline (`lib.rs`/`live.rs`) exactly like the
//! Policy panel's Decision Stream reads `source == "wardryx"` off that SAME
//! feed - see `src/components/QualityDriftStream.tsx`, which is a pure
//! frontend filter, not a new backend read (docs/PHASE4.md: "Data also from
//! the verdryx.* ... bus events already tailed").
//!
//! Read-only, same as Identity: no mutation command, no
//! `genaryx_core::command::record` journal entry - Verdryx has no write API
//! this console could call even if it wanted to (`VerdryxClient` issues only
//! `SELECT`s), and nothing here changes any other plane's state.
//!
//! Every read opens its OWN fresh `VerdryxClient` inside a `spawn_blocking`
//! (rusqlite is synchronous IO; see `state.rs`'s module doc for why a
//! connection is never parked in managed state) - mirrors
//! `identity::commands::identity_rescan`'s identical "blocking work never
//! runs straight on the async executor" discipline.

use super::env::EnvSource;
use super::state::{QualityClient, QualityInner, QualityState};
use genaryx_connectors::{
    VerdryxBaseline, VerdryxClient, VerdryxError, VerdryxRunSummary, VerdryxScore,
};
use serde::Serialize;

// ============================================================================
// DTOs
// ============================================================================

/// Whole-panel connection state, for the frontend to render up front - mirrors
/// `identity::commands::IdentityStatusDto`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum QualityStatusDto {
    Bootstrapping,
    NoEnvironment,
    Unreachable {
        source: EnvSource,
        db_path: String,
        reason: String,
    },
    Ready {
        source: EnvSource,
        db_path: String,
    },
}

/// Every error a quality command can return - mirrors
/// `identity::commands::IdentityError`'s shape, minus the HTTP-status
/// distinction Idryx's REST errors carry: `VerdryxError` is always either a
/// filesystem/SQLite open failure or a query failure, so one message-carrying
/// variant is honest here (no fabricated status code).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QualityError {
    Bootstrapping,
    NoEnvironment,
    Unreachable { reason: String },
    Verdryx { message: String },
}

impl From<VerdryxError> for QualityError {
    fn from(e: VerdryxError) -> Self {
        QualityError::Verdryx {
            message: e.to_string(),
        }
    }
}

// ============================================================================
// helpers
// ============================================================================

/// Resolve the current [`QualityClient`] out of managed state, or the
/// appropriate [`QualityError`] when the panel is not ready. Mirrors
/// `identity::commands::ready_client` exactly.
async fn ready_client(state: &&QualityState) -> Result<QualityClient, QualityError> {
    let guard = state.inner.lock().await;
    match &*guard {
        QualityInner::Ready(client) => Ok(client.clone()),
        QualityInner::Bootstrapping => Err(QualityError::Bootstrapping),
        QualityInner::NoEnvironment => Err(QualityError::NoEnvironment),
        QualityInner::Unreachable { reason, .. } => Err(QualityError::Unreachable {
            reason: reason.clone(),
        }),
    }
}

/// Pure `QualityInner` -> `QualityStatusDto` mapping, factored out of
/// [`quality_status`] so it is directly unit-testable - same rationale as
/// `identity::commands::status_dto`.
fn status_dto(inner: &QualityInner) -> QualityStatusDto {
    match inner {
        QualityInner::Bootstrapping => QualityStatusDto::Bootstrapping,
        QualityInner::NoEnvironment => QualityStatusDto::NoEnvironment,
        QualityInner::Unreachable {
            source,
            db_path,
            reason,
        } => QualityStatusDto::Unreachable {
            source: source.clone(),
            db_path: db_path.display().to_string(),
            reason: reason.clone(),
        },
        QualityInner::Ready(client) => QualityStatusDto::Ready {
            source: client.source.clone(),
            db_path: client.db_path.display().to_string(),
        },
    }
}

/// Run a blocking Verdryx read off the async executor thread - shared by
/// every command below. `f` opens its own fresh connection (see `state.rs`'s
/// module doc for why).
async fn run_blocking<T, F>(f: F) -> Result<T, QualityError>
where
    F: FnOnce() -> Result<T, VerdryxError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| QualityError::Verdryx {
            message: format!("quality read task failed to run: {e}"),
        })?
        .map_err(QualityError::from)
}

// ============================================================================
// commands
// ============================================================================

/// Whole-panel connection state. Never fails: every outcome of
/// [`super::state::bootstrap`] is a renderable [`QualityStatusDto`] variant.
pub async fn quality_status(state: &QualityState) -> Result<QualityStatusDto, ()> {
    let guard = state.inner.lock().await;
    Ok(status_dto(&guard))
}

/// Every eval run, newest-started first, each pre-joined with its own
/// summary (case_count/mean_score/total_tokens/total_cost_usd) - the
/// Eval-runs history table AND the Run-detail header once a row is
/// selected, in one round trip (docs/PHASE4.md W1 positions 1-2). Composes
/// `VerdryxClient::list_eval_runs` + `run_summary` over ONE connection - not
/// a connector change, this is the Tauri layer combining two existing public
/// reads, same as `graph::build_agent_graph` composes
/// `DelegationGraph::from_store` + `layout_view` from core.
pub async fn quality_list_run_summaries(
    state: &QualityState,
) -> Result<Vec<VerdryxRunSummary>, QualityError> {
    let client = ready_client(&state).await?;
    run_blocking(move || {
        let conn = VerdryxClient::open(&client.db_path)?;
        let runs = conn.list_eval_runs()?;
        let mut summaries = Vec::with_capacity(runs.len());
        for run in runs {
            // `run_summary` only answers `None` for a run id that does not
            // exist - impossible here since `run.id` just came from
            // `list_eval_runs` on the SAME connection, but this never
            // panics or fabricates a row if that invariant is ever wrong.
            if let Some(summary) = conn.run_summary(&run.id)? {
                summaries.push(summary);
            }
        }
        Ok(summaries)
    })
    .await
}

/// One run's per-case scores, in evaluation order - the Run-detail table
/// (docs/PHASE4.md W1 position 2).
pub async fn quality_run_scores(
    run_id: String,
    state: &QualityState,
) -> Result<Vec<VerdryxScore>, QualityError> {
    let client = ready_client(&state).await?;
    run_blocking(move || VerdryxClient::open(&client.db_path)?.scores_for_run(&run_id)).await
}

/// Every saved baseline, newest-created first (docs/PHASE4.md W1 position
/// 3).
pub async fn quality_list_baselines(
    state: &QualityState,
) -> Result<Vec<VerdryxBaseline>, QualityError> {
    let client = ready_client(&state).await?;
    run_blocking(move || VerdryxClient::open(&client.db_path)?.list_baselines()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn status_dto_maps_bootstrapping_and_no_environment_directly() {
        assert!(matches!(
            status_dto(&QualityInner::Bootstrapping),
            QualityStatusDto::Bootstrapping
        ));
        assert!(matches!(
            status_dto(&QualityInner::NoEnvironment),
            QualityStatusDto::NoEnvironment
        ));
    }

    #[test]
    fn status_dto_unreachable_preserves_source_path_and_reason() {
        let unreachable = QualityInner::Unreachable {
            source: EnvSource::WellKnown,
            db_path: PathBuf::from("/tmp/verdryx.db"),
            reason: "open failed".to_string(),
        };
        match status_dto(&unreachable) {
            QualityStatusDto::Unreachable {
                db_path, reason, ..
            } => {
                assert_eq!(db_path, "/tmp/verdryx.db");
                assert_eq!(reason, "open failed");
            }
            other => panic!("expected Unreachable, got {other:?}"),
        }
    }

    #[test]
    fn status_dto_ready_carries_source_and_path() {
        let ready = QualityInner::Ready(QualityClient {
            source: EnvSource::WellKnown,
            db_path: PathBuf::from("/tmp/verdryx.db"),
        });
        match status_dto(&ready) {
            QualityStatusDto::Ready { db_path, .. } => assert_eq!(db_path, "/tmp/verdryx.db"),
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn quality_error_from_verdryx_error_carries_a_message() {
        // A genuine VerdryxError (missing-file open failure), same fixture
        // `VerdryxClient`'s own `open_missing_db_is_fail_closed` test uses -
        // avoids hand-constructing a `rusqlite::Error` just for this test.
        let path = std::env::temp_dir().join("genaryx-quality-commands-test-does-not-exist.db");
        let _ = std::fs::remove_file(&path);
        let err = VerdryxClient::open(&path).expect_err("a missing file must fail to open");

        let mapped = QualityError::from(err);
        let QualityError::Verdryx { message } = mapped else {
            panic!("expected a Verdryx-shaped QualityError")
        };
        assert!(!message.is_empty());
    }
}
