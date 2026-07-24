//! Console commands for the Identity view: three plain reads
//! (`identity_list_identities`/`identity_list_alerts`/
//! `identity_list_remediations`) plus [`identity_rescan`] (an on-demand
//! `idryx detect --format json` batch, run off the async executor thread -
//! see its own doc), plus [`identity_status`] so the frontend can render a
//! clean "no identity plane" / "unreachable" state up front instead of
//! guessing from a read command's error shape.
//!
//! Identity is READ-ONLY this wave (docs/PHASE3.md W2): no mutation
//! command, no `genaryx_core::command::record` journal entry, no signer -
//! idryx itself has no write API at all (07 §4.4), and nothing in this
//! module changes any other plane's state either. Every list command
//! returns a `genaryx_connectors::Idryx*` DTO directly: unlike
//! `WardryxClient`'s types (`Deserialize`-only, since they exist only to
//! parse Wardryx's responses), idryx's connector DTOs already derive
//! `Serialize` too (`crates/connectors/src/idryx.rs`), so they can be handed
//! to the frontend as-is with no UI-facing mirror struct needed.

use super::env::EnvSource;
use super::state::{IdentityClient, IdentityInner, IdentityState};
use genaryx_connectors::{IdryxAlert, IdryxClient, IdryxError, IdryxIdentity, IdryxRecommendation};
use serde::Serialize;

// ============================================================================
// DTOs
// ============================================================================

/// Whole-panel connection state, for the frontend to render up front (never
/// inferred from a read command's error shape) - mirrors
/// `policy::commands::PolicyStatusDto`. `Ready::rescan_available` says
/// whether the `idryx` binary resolved (`~/.taipan/bin/idryx`), so the
/// Rescan button can disable itself with an honest tooltip up front instead
/// of only discovering unavailability after a click.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum IdentityStatusDto {
    Bootstrapping,
    NoEnvironment,
    Unreachable {
        source: EnvSource,
        idryx_url: String,
        reason: String,
    },
    Ready {
        source: EnvSource,
        idryx_url: String,
        rescan_available: bool,
    },
}

/// Every error an identity command can return - mirrors
/// `policy::commands::PolicyError`'s shape. [`IdentityError::RescanUnavailable`]
/// is specific to this module: the `idryx` binary never resolved
/// (`state::IdentityClient::idryx_bin` is `None`), a precondition checked
/// BEFORE ever touching the connector, so it is never reached through the
/// `From<IdryxError>` impl below.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IdentityError {
    Bootstrapping,
    NoEnvironment,
    Unreachable {
        reason: String,
    },
    /// Any Idryx-side failure: transport, a plain non-2xx, a response that
    /// failed to parse, or a `detect` spawn/exit failure. `status` is
    /// `None` when the failure never had an HTTP status to begin with
    /// (including every `detect`/CLI failure) - an honest `None` beats a
    /// made-up code.
    Idryx {
        status: Option<u16>,
        message: String,
    },
    /// Rescan was requested but no usable `idryx` binary was ever resolved
    /// (`~/.taipan/bin/idryx` is not a file) - reported honestly instead of
    /// a fake success or a generic transport-shaped error.
    RescanUnavailable,
}

impl From<IdryxError> for IdentityError {
    fn from(e: IdryxError) -> Self {
        match e {
            IdryxError::Transport(err) => IdentityError::Idryx {
                status: None,
                message: format!("could not reach idryx: {err}"),
            },
            IdryxError::Json(err) => IdentityError::Idryx {
                status: None,
                message: format!("unexpected response shape from idryx: {err}"),
            },
            IdryxError::Api { status, body } => IdentityError::Idryx {
                status: Some(status),
                message: body,
            },
            IdryxError::Cli(message) => IdentityError::Idryx {
                status: None,
                message: format!("idryx detect failed: {message}"),
            },
        }
    }
}

// ============================================================================
// helpers
// ============================================================================

/// Resolve the current [`IdentityClient`] out of managed state, or the
/// appropriate [`IdentityError`] when the panel is not ready. Only holds the
/// state lock long enough to clone the (cheap, `Arc`-backed) client out -
/// mirrors `policy::commands::ready_client` exactly.
async fn ready_client(state: &&IdentityState) -> Result<IdentityClient, IdentityError> {
    let guard = state.inner.lock().await;
    match &*guard {
        IdentityInner::Ready(client) => Ok(client.clone()),
        IdentityInner::Bootstrapping => Err(IdentityError::Bootstrapping),
        IdentityInner::NoEnvironment => Err(IdentityError::NoEnvironment),
        IdentityInner::Unreachable { reason, .. } => Err(IdentityError::Unreachable {
            reason: reason.clone(),
        }),
    }
}

/// Pure `IdentityInner` -> `IdentityStatusDto` mapping, factored out of
/// [`identity_status`] so it is directly unit-testable without a live
/// shell wrapper - same rationale as
/// `policy::commands::describe_decision_result` being its own free
/// function.
fn status_dto(inner: &IdentityInner) -> IdentityStatusDto {
    match inner {
        IdentityInner::Bootstrapping => IdentityStatusDto::Bootstrapping,
        IdentityInner::NoEnvironment => IdentityStatusDto::NoEnvironment,
        IdentityInner::Unreachable {
            source,
            idryx_url,
            reason,
        } => IdentityStatusDto::Unreachable {
            source: source.clone(),
            idryx_url: idryx_url.clone(),
            reason: reason.clone(),
        },
        IdentityInner::Ready(client) => IdentityStatusDto::Ready {
            source: client.source.clone(),
            idryx_url: client.idryx_url.clone(),
            rescan_available: client.idryx_bin.is_some(),
        },
    }
}

/// Passed as `idryx detect`'s required `--min-severity` flag. **This value
/// has no effect on the `--format json` output at all** - grounded in the
/// idryx Go source (`~/Development/Idryx/cmd/idryx/main.go:394-414`):
/// `runDetectors` computes every alert unconditionally, `report.JSON`
/// (`internal/report/report.go:54-71`) sorts and prints all of them
/// unfiltered, and `--min-severity` is parsed and used only AFTER that, to
/// gate the `--slack`/`--webhook` sinks this connector never configures
/// (`IdryxClient::rescan` passes neither flag). Kept as an explicit, valid
/// value (idryx rejects an invalid one, `main.go:411-414`) rather than a
/// magic minimum, so a future sink wiring does not silently start dropping
/// low-severity alerts because of a stray default here - "low" means
/// exactly what it says.
const MIN_SEVERITY: &str = "low";

// ============================================================================
// commands: status + reads
// ============================================================================

/// Whole-panel connection state. Never fails: every outcome of
/// [`super::state::bootstrap`] is a renderable [`IdentityStatusDto`]
/// variant.
pub async fn identity_status(state: &IdentityState) -> Result<IdentityStatusDto, ()> {
    let guard = state.inner.lock().await;
    Ok(status_dto(&guard))
}

/// `GET /api/identities` - every identity in idryx's load-once snapshot
/// (docs/PHASE3.md: "as of load", never live).
pub async fn identity_list_identities(
    state: &IdentityState,
) -> Result<Vec<IdryxIdentity>, IdentityError> {
    let client = ready_client(&state).await?;
    client
        .client
        .list_identities()
        .await
        .map_err(IdentityError::from)
}

/// `GET /api/alerts` - every detector alert in idryx's load-once snapshot;
/// see [`identity_rescan`] for the on-demand recompute path.
pub async fn identity_list_alerts(state: &IdentityState) -> Result<Vec<IdryxAlert>, IdentityError> {
    let client = ready_client(&state).await?;
    client
        .client
        .list_alerts()
        .await
        .map_err(IdentityError::from)
}

/// `GET /api/remediations` - every right-size/rotation suggestion idryx
/// generated at load time.
pub async fn identity_list_remediations(
    state: &IdentityState,
) -> Result<Vec<IdryxRecommendation>, IdentityError> {
    let client = ready_client(&state).await?;
    client
        .client
        .list_remediations()
        .await
        .map_err(IdentityError::from)
}

// ============================================================================
// commands: Rescan
// ============================================================================

/// Recompute the 21 detectors on demand (`idryx detect --format json`,
/// docs/PHASE3.md: "serve is load-once... Rescan is how the console picks
/// up new findings without restarting idryx"). Returns the SAME
/// [`IdryxAlert`] shape [`identity_list_alerts`] does - the frontend treats
/// a successful Rescan's result as the new authoritative alerts view.
///
/// `IdryxClient::rescan` is a synchronous batch call (its own doc: "a batch
/// job the caller runs off the UI thread" - it shells out to `idryx detect`
/// and blocks on the child process), so this runs it inside
/// [`tokio::task::spawn_blocking`] rather than awaiting it
/// directly on the async command's own task, exactly the way a blocking
/// call must never run straight on an async executor.
///
/// Fails closed with [`IdentityError::RescanUnavailable`] - never a fake
/// success - when no `idryx` binary was resolved at bootstrap
/// (`~/.taipan/bin/idryx`); the frontend is expected to have already
/// disabled the Rescan control from
/// `IdentityStatusDto::Ready::rescan_available`, but this command re-checks
/// independently rather than trusting the caller.
pub async fn identity_rescan(state: &IdentityState) -> Result<Vec<IdryxAlert>, IdentityError> {
    let client = ready_client(&state).await?;
    let Some(bin) = client.idryx_bin.clone() else {
        return Err(IdentityError::RescanUnavailable);
    };
    let loads_owned: Vec<(String, String)> = client
        .rescan_loads
        .iter()
        .map(|(source, path)| (source.clone(), path.to_string_lossy().into_owned()))
        .collect();

    let result = tokio::task::spawn_blocking(move || {
        let loads: Vec<(&str, &str)> = loads_owned
            .iter()
            .map(|(source, path)| (source.as_str(), path.as_str()))
            .collect();
        IdryxClient::rescan(&bin, &loads, MIN_SEVERITY)
    })
    .await
    .map_err(|e| IdentityError::Idryx {
        status: None,
        message: format!("rescan task failed to run: {e}"),
    })?;

    result.map_err(IdentityError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::env::EnvSource;
    use crate::identity::state::IdentityClient;
    use std::sync::Arc;

    fn fixture_client(idryx_bin: Option<std::path::PathBuf>) -> IdentityClient {
        IdentityClient {
            client: Arc::new(IdryxClient::new("http://127.0.0.1:8081").expect("build a client")),
            source: EnvSource::Taipan {
                name: "p1full".to_string(),
            },
            idryx_url: "http://127.0.0.1:8081".to_string(),
            idryx_bin,
            rescan_loads: Vec::new(),
        }
    }

    #[test]
    fn status_dto_maps_bootstrapping_and_no_environment_directly() {
        assert!(matches!(
            status_dto(&IdentityInner::Bootstrapping),
            IdentityStatusDto::Bootstrapping
        ));
        assert!(matches!(
            status_dto(&IdentityInner::NoEnvironment),
            IdentityStatusDto::NoEnvironment
        ));
    }

    #[test]
    fn status_dto_unreachable_preserves_source_url_and_reason() {
        let unreachable = IdentityInner::Unreachable {
            source: EnvSource::Taipan {
                name: "p1full".to_string(),
            },
            idryx_url: "http://127.0.0.1:8081".to_string(),
            reason: "connection refused".to_string(),
        };
        match status_dto(&unreachable) {
            IdentityStatusDto::Unreachable {
                idryx_url, reason, ..
            } => {
                assert_eq!(idryx_url, "http://127.0.0.1:8081");
                assert_eq!(reason, "connection refused");
            }
            other => panic!("expected Unreachable, got {other:?}"),
        }
    }

    #[test]
    fn status_dto_ready_reports_rescan_availability_honestly() {
        let with_bin =
            IdentityInner::Ready(fixture_client(Some(std::path::PathBuf::from("/tmp/idryx"))));
        match status_dto(&with_bin) {
            IdentityStatusDto::Ready {
                rescan_available, ..
            } => assert!(rescan_available),
            other => panic!("expected Ready, got {other:?}"),
        }

        let without_bin = IdentityInner::Ready(fixture_client(None));
        match status_dto(&without_bin) {
            IdentityStatusDto::Ready {
                rescan_available, ..
            } => assert!(!rescan_available),
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn identity_error_from_idryx_error_preserves_status_and_message() {
        let e = IdentityError::from(IdryxError::Api {
            status: 404,
            body: "not found".to_string(),
        });
        match e {
            IdentityError::Idryx {
                status: Some(404),
                message,
            } => assert_eq!(message, "not found"),
            other => panic!("expected Idryx{{404,..}}, got {other:?}"),
        }

        let e = IdentityError::from(IdryxError::Cli("exit status: 1".to_string()));
        match e {
            IdentityError::Idryx {
                status: None,
                message,
            } => {
                assert!(message.contains("exit status: 1"), "got {message:?}");
            }
            other => panic!("expected Idryx{{None,..}}, got {other:?}"),
        }
    }

    #[test]
    fn identity_error_from_json_error_has_no_status() {
        let json_err = serde_json::from_str::<Vec<IdryxAlert>>("not json").unwrap_err();
        let e = IdentityError::from(IdryxError::from(json_err));
        assert!(matches!(e, IdentityError::Idryx { status: None, .. }));
    }
}
