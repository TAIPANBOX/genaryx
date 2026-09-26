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
use genaryx_connectors::{GatewayError, GatewayKeysReport, GatewayRunView};
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
            GatewayError::Unauthorized => CredentialsError::Gateway {
                status: Some(401),
                message: "the gateway refused the console's key: check \
                          TOKENFUSE_GATEWAY_ADMIN_KEY against the gateway's TOKENFUSE_ADMIN_KEYS"
                    .to_string(),
            },
            GatewayError::AdminKeysRequired => CredentialsError::Gateway {
                status: Some(403),
                message: "the gateway requires TOKENFUSE_ADMIN_KEYS on a non-loopback bind and \
                          the console holds no key (TOKENFUSE_GATEWAY_ADMIN_KEY is unset)"
                    .to_string(),
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

/// One run the gateway is holding money open for, after a call whose outcome
/// it never learned. Money "held after unknown outcome" (invariant 50 in
/// tokenfuse): deliberately not released and not counted spent.
#[derive(Debug, Clone, Serialize)]
pub struct RetainedRunDto {
    pub run_id: String,
    pub retained: u32,
    pub retained_usd: f64,
}

impl From<&GatewayRunView> for RetainedRunDto {
    fn from(r: &GatewayRunView) -> Self {
        RetainedRunDto {
            run_id: r.run_id.clone(),
            retained: r.retained,
            retained_usd: r.retained_usd,
        }
    }
}

/// The gateway's held-reservations report: every run with a non-zero
/// `retained` count, plus the fleet-wide total. `total_retained`/
/// `total_retained_usd` are summed over EVERY run the gateway answered, not
/// only the ones shown, so the total is exact even though the list is
/// filtered.
#[derive(Debug, Clone, Serialize)]
pub struct GatewayRetainedDto {
    /// Runs with `retained > 0`, most-retained first.
    pub runs: Vec<RetainedRunDto>,
    pub total_retained: u32,
    pub total_retained_usd: f64,
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

/// `GET /v1/runs` on the gateway - reservations the gateway is holding open
/// after a call whose outcome it never learned (money "held after unknown
/// outcome"), never shown as a `0` when the gateway is not configured or not
/// reachable: an unready plane surfaces through the normal
/// [`ready_client`]/[`CredentialsError`] path, exactly like
/// [`credentials_keys`], so the panel says NoEnvironment/Unreachable/Gateway
/// rather than defaulting to an empty, zero-looking report.
pub async fn credentials_gateway_retained_runs(
    state: &CredentialsState,
) -> Result<GatewayRetainedDto, CredentialsError> {
    let client = ready_client(&state).await?;
    let runs = client
        .client
        .get_runs()
        .await
        .map_err(CredentialsError::from)?;

    let total_retained: u32 = runs.iter().map(|r| r.retained).sum();
    let total_retained_usd: f64 = runs.iter().map(|r| r.retained_usd).sum();
    let mut held: Vec<RetainedRunDto> = runs
        .iter()
        .filter(|r| r.retained > 0)
        .map(RetainedRunDto::from)
        .collect();
    held.sort_by(|a, b| {
        b.retained_usd
            .partial_cmp(&a.retained_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(GatewayRetainedDto {
        runs: held,
        total_retained,
        total_retained_usd,
    })
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

    // ---- credentials_gateway_retained_runs ---------------------------------

    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use tokio::sync::Mutex;

    fn spawn_mock_gateway_runs(body: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let port = listener.local_addr().expect("local_addr").port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept one connection");
            let mut buf = [0u8; 4096];
            let mut request = Vec::new();
            loop {
                let n = stream.read(&mut buf).expect("read the request");
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
                if request.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        });
        port
    }

    fn ready_state_at(port: u16) -> CredentialsState {
        let client = CredentialsClient {
            client: Arc::new(
                GatewayClient::new(format!("http://127.0.0.1:{port}")).expect("build a client"),
            ),
            source: EnvSource::Taipan {
                name: "p1full".to_string(),
            },
            gateway_url: format!("http://127.0.0.1:{port}"),
        };
        CredentialsState {
            inner: Mutex::new(CredentialsInner::Ready(client)),
        }
    }

    #[tokio::test]
    async fn retained_runs_filters_to_non_zero_and_sums_the_whole_fleet() {
        let port = spawn_mock_gateway_runs(
            r#"[
              {"run_id":"held","budget_usd":10.0,"spent_usd":1.0,"reserved_usd":0.01,
               "remaining_usd":8.99,"steps":3,"pct_used":10.0,"killed":false,
               "retained":1,"retained_usd":0.01},
              {"run_id":"clean","budget_usd":5.0,"spent_usd":2.0,"reserved_usd":0.0,
               "remaining_usd":3.0,"steps":1,"pct_used":40.0,"killed":false,
               "retained":0,"retained_usd":0.0},
              {"run_id":"also-held","budget_usd":2.0,"spent_usd":0.5,"reserved_usd":0.02,
               "remaining_usd":1.48,"steps":2,"pct_used":25.0,"killed":false,
               "retained":2,"retained_usd":0.02}
            ]"#,
        );
        let state = ready_state_at(port);

        let dto = credentials_gateway_retained_runs(&state)
            .await
            .expect("must succeed against a mock gateway");

        // The total is over the WHOLE fleet, not only the non-zero rows.
        assert_eq!(dto.total_retained, 3);
        assert!((dto.total_retained_usd - 0.03).abs() < 1e-9);
        // Only non-zero rows are listed, most-retained-usd first.
        assert_eq!(dto.runs.len(), 2);
        assert_eq!(dto.runs[0].run_id, "also-held");
        assert_eq!(dto.runs[1].run_id, "held");
        assert!(dto.runs.iter().all(|r| r.retained > 0));
    }

    #[tokio::test]
    async fn a_fleet_with_nothing_retained_reports_a_real_zero() {
        let port = spawn_mock_gateway_runs(
            r#"[{"run_id":"clean","budget_usd":1.0,"spent_usd":0.1,"reserved_usd":0.0,
                 "remaining_usd":0.9,"steps":1,"pct_used":10.0,"killed":false,
                 "retained":0,"retained_usd":0.0}]"#,
        );
        let state = ready_state_at(port);

        let dto = credentials_gateway_retained_runs(&state)
            .await
            .expect("must succeed");
        assert_eq!(dto.total_retained, 0);
        assert_eq!(dto.total_retained_usd, 0.0);
        assert!(dto.runs.is_empty());
    }

    #[tokio::test]
    async fn retained_runs_reports_no_environment_rather_than_a_fabricated_zero() {
        let state = CredentialsState::pending();
        let err = credentials_gateway_retained_runs(&state)
            .await
            .expect_err("bootstrapping must not answer a report");
        assert!(matches!(err, CredentialsError::Bootstrapping));

        *state.inner.lock().await = CredentialsInner::NoEnvironment;
        let err = credentials_gateway_retained_runs(&state)
            .await
            .expect_err("no environment must not answer a report");
        assert!(matches!(err, CredentialsError::NoEnvironment));
    }

    #[tokio::test]
    async fn retained_runs_reports_unreachable_rather_than_a_fabricated_zero() {
        let state = CredentialsState {
            inner: Mutex::new(CredentialsInner::Unreachable {
                source: EnvSource::Taipan {
                    name: "p1full".to_string(),
                },
                gateway_url: "http://127.0.0.1:1".to_string(),
                reason: "connection refused".to_string(),
            }),
        };
        let err = credentials_gateway_retained_runs(&state)
            .await
            .expect_err("unreachable must not answer a report");
        assert!(matches!(err, CredentialsError::Unreachable { .. }));
    }
}
