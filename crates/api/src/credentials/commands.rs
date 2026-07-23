//! Commands for the Credentials card: [`credentials_status`] (the
//! state-tagged connection DTO, mirrors `identity::commands::identity_status`)
//! and [`credentials_keys`] (the gateway's key-lifecycle report, straight
//! through - `GatewayKeysReport` already derives `Serialize`, no UI-facing
//! mirror struct needed, the exact idryx precedent `identity::commands`'s
//! module doc names).
//!
//! Read-only plane (I15): no mutation command, no `console_actor`, no
//! `genaryx_core::command::record` journal entry, no signer - this plane
//! changes nothing in any other plane, mirroring `identity::commands`'s own
//! "Identity is READ-ONLY this wave" rule exactly.

use super::env::EnvSource;
use super::state::{CredentialsClient, CredentialsInner, CredentialsState};
use genaryx_connectors::{GatewayError, GatewayKeysReport};
use serde::Serialize;

// ============================================================================
// DTOs
// ============================================================================

/// Whole-panel connection state, for the frontend to render up front (never
/// inferred from a read command's error shape) - mirrors
/// `identity::commands::IdentityStatusDto`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CredentialsStatusDto {
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

/// Every error a credentials command can return - mirrors
/// `identity::commands::IdentityError`'s shape, minus the Rescan-specific
/// variant Identity has and this plane does not.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CredentialsError {
    Bootstrapping,
    NoEnvironment,
    Unreachable {
        reason: String,
    },
    /// Any gateway-side failure: transport, a plain non-2xx, or a response
    /// that failed to parse. `status` is `None` when the failure never had
    /// an HTTP status to begin with - an honest `None` beats a made-up code.
    Gateway {
        status: Option<u16>,
        message: String,
    },
}

impl From<GatewayError> for CredentialsError {
    fn from(e: GatewayError) -> Self {
        match e {
            GatewayError::Transport(err) => CredentialsError::Gateway {
                status: None,
                message: format!("could not reach the gateway: {err}"),
            },
            GatewayError::Json(err) => CredentialsError::Gateway {
                status: None,
                message: format!("unexpected response shape from the gateway: {err}"),
            },
            GatewayError::Api { status, body } => CredentialsError::Gateway {
                status: Some(status),
                message: body,
            },
        }
    }
}

// ============================================================================
// helpers
// ============================================================================

/// Resolve the current [`CredentialsClient`] out of managed state, or the
/// appropriate [`CredentialsError`] when the panel is not ready. Only holds
/// the state lock long enough to clone the (cheap, `Arc`-backed) client out -
/// mirrors `identity::commands::ready_client` exactly.
async fn ready_client(state: &&CredentialsState) -> Result<CredentialsClient, CredentialsError> {
    let guard = state.inner.lock().await;
    match &*guard {
        CredentialsInner::Ready(client) => Ok(client.clone()),
        CredentialsInner::Bootstrapping => Err(CredentialsError::Bootstrapping),
        CredentialsInner::NoEnvironment => Err(CredentialsError::NoEnvironment),
        CredentialsInner::Unreachable { reason, .. } => Err(CredentialsError::Unreachable {
            reason: reason.clone(),
        }),
    }
}

/// Pure `CredentialsInner` -> `CredentialsStatusDto` mapping, factored out of
/// [`credentials_status`] so it is directly unit-testable without a live
/// shell wrapper - same rationale as `identity::commands::status_dto`.
fn status_dto(inner: &CredentialsInner) -> CredentialsStatusDto {
    match inner {
        CredentialsInner::Bootstrapping => CredentialsStatusDto::Bootstrapping,
        CredentialsInner::NoEnvironment => CredentialsStatusDto::NoEnvironment,
        CredentialsInner::Unreachable {
            source,
            gateway_url,
            reason,
        } => CredentialsStatusDto::Unreachable {
            source: source.clone(),
            gateway_url: gateway_url.clone(),
            reason: reason.clone(),
        },
        CredentialsInner::Ready(client) => CredentialsStatusDto::Ready {
            source: client.source.clone(),
            gateway_url: client.gateway_url.clone(),
        },
    }
}

// ============================================================================
// commands
// ============================================================================

/// Whole-panel connection state. Never fails: every outcome of
/// [`super::state::bootstrap`] is a renderable [`CredentialsStatusDto`]
/// variant.
pub async fn credentials_status(state: &CredentialsState) -> Result<CredentialsStatusDto, ()> {
    let guard = state.inner.lock().await;
    Ok(status_dto(&guard))
}

/// `GET /v1/keys` - the gateway's live key-lifecycle report. Always a fresh
/// read (no caching at this layer): the report changes as calls come in, and
/// the Credentials card polls this on its own 30s cadence.
pub async fn credentials_keys(
    state: &CredentialsState,
) -> Result<GatewayKeysReport, CredentialsError> {
    let client = ready_client(&state).await?;
    client
        .client
        .get_keys()
        .await
        .map_err(CredentialsError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::env::EnvSource;
    use crate::credentials::state::CredentialsClient;
    use genaryx_connectors::GatewayClient;
    use std::sync::Arc;

    fn fixture_client() -> CredentialsClient {
        CredentialsClient {
            client: Arc::new(GatewayClient::new("http://127.0.0.1:4100").expect("build a client")),
            source: EnvSource::Taipan {
                name: "p1full".to_string(),
            },
            gateway_url: "http://127.0.0.1:4100".to_string(),
        }
    }

    #[test]
    fn status_dto_maps_bootstrapping_and_no_environment_directly() {
        assert!(matches!(
            status_dto(&CredentialsInner::Bootstrapping),
            CredentialsStatusDto::Bootstrapping
        ));
        assert!(matches!(
            status_dto(&CredentialsInner::NoEnvironment),
            CredentialsStatusDto::NoEnvironment
        ));
    }

    #[test]
    fn status_dto_unreachable_preserves_source_url_and_reason() {
        let unreachable = CredentialsInner::Unreachable {
            source: EnvSource::Taipan {
                name: "p1full".to_string(),
            },
            gateway_url: "http://127.0.0.1:4100".to_string(),
            reason: "connection refused".to_string(),
        };
        match status_dto(&unreachable) {
            CredentialsStatusDto::Unreachable {
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
    fn status_dto_ready_reports_the_gateway_url() {
        let ready = CredentialsInner::Ready(fixture_client());
        match status_dto(&ready) {
            CredentialsStatusDto::Ready { gateway_url, .. } => {
                assert_eq!(gateway_url, "http://127.0.0.1:4100");
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn credentials_error_from_gateway_error_preserves_status_and_message() {
        let e = CredentialsError::from(GatewayError::Api {
            status: 404,
            body: "not found".to_string(),
        });
        match e {
            CredentialsError::Gateway {
                status: Some(404),
                message,
            } => assert_eq!(message, "not found"),
            other => panic!("expected Gateway{{404,..}}, got {other:?}"),
        }
    }

    #[test]
    fn credentials_error_from_json_error_has_no_status() {
        let json_err = serde_json::from_str::<GatewayKeysReport>("not json").unwrap_err();
        let e = CredentialsError::from(GatewayError::from(json_err));
        assert!(matches!(e, CredentialsError::Gateway { status: None, .. }));
    }
}
