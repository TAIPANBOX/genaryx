//! `POST /relay/v1/pair` (docs/PHASE5.md "pairing" module; itrat-console/13
//! D12.2 step 6-8): the phone redeems the one-time code it scanned off the
//! desktop's QR. The SAME code is redeemed upstream at the Cloud's own
//! `POST /v1/pair` (`http.rs:1540`) so the phone's public key is registered
//! at the Cloud itself -- the relay never becomes a signature authority, it
//! only gatekeeps ITS OWN single-device slot and forwards the credential.
//!
//! No `CloudClient::pair()` reuse here: that helper both MINTS (`/v1/pair/new`)
//! and redeems a code with a signer it generates itself, which is the
//! desktop's flow (W2), not this one -- the relay redeems an
//! ALREADY-MINTED code with the PHONE's own already-generated public key, a
//! plain unauthenticated POST (`devices.rs`'s own doc: "the code is the
//! credential"). This module reuses the wire DTO (`genaryx_connectors::PairResponse`)
//! and hits the endpoint directly with the same `reqwest`/manual-JSON pattern
//! `cloud_rest.rs` itself uses (no `reqwest` `json` feature is enabled in this
//! workspace, by that module's own stated choice).

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use genaryx_connectors::PairResponse;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

use crate::registry::{NewDevice, RegistryError};

#[derive(Debug, thiserror::Error)]
pub enum PairingError {
    /// Maps to HTTP 409 (docs/PHASE5.md: "a second pair attempt while a row
    /// exists returns a `DeviceExists` error (HTTP 409)").
    #[error("device already paired")]
    DeviceExists,
    /// No window open / expired / wrong code -- ALL map to HTTP 404
    /// ("only inside an armed window, otherwise 404", D12.3), deliberately
    /// indistinguishable to the caller (D12.3: the pairing route stays dark
    /// outside a window).
    #[error("no matching open pairing window")]
    WindowNotOpen,
    /// Per-IP rate limit on the pre-auth pairing route (D12.3 R2: "rate
    /// limits" -- this route is the one public endpoint with no bearer at
    /// all, so it is the one guarded by caller IP instead of device id).
    #[error("rate limit exceeded")]
    RateLimited,
    #[error("malformed request: {0}")]
    BadRequest(String),
    /// The caller declared which slot it expected and the code admits the
    /// other one. A distinct, loud error rather than a generic 400: it almost
    /// always means the desktop built the QR with the two codes crossed, and
    /// that is worth naming rather than leaving someone to guess.
    #[error("this code is for the other device slot")]
    KindMismatch,
    #[error("could not reach the Cloud to redeem the code: {0}")]
    UpstreamTransport(String),
    #[error("the Cloud rejected the code: {0}")]
    UpstreamRejected(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl From<RegistryError> for PairingError {
    fn from(e: RegistryError) -> Self {
        match e {
            RegistryError::DeviceExists => PairingError::DeviceExists,
            RegistryError::WindowNotOpen => PairingError::WindowNotOpen,
            other => PairingError::Internal(other.to_string()),
        }
    }
}

impl IntoResponse for PairingError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            PairingError::DeviceExists => (StatusCode::CONFLICT, "device_exists"),
            PairingError::WindowNotOpen => (StatusCode::NOT_FOUND, "not_found"),
            PairingError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            PairingError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            PairingError::KindMismatch => (StatusCode::BAD_REQUEST, "kind_mismatch"),
            PairingError::UpstreamRejected(_) => {
                (StatusCode::BAD_REQUEST, "invalid_or_expired_code")
            }
            PairingError::UpstreamTransport(_) => (StatusCode::BAD_GATEWAY, "upstream_unavailable"),
            PairingError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };
        (status, Json(serde_json::json!({ "error": code }))).into_response()
    }
}

/// `POST /relay/v1/pair` request body (D12.2 step 6's literal field names).
#[derive(Debug, Deserialize)]
pub struct PairRequestIn {
    pub code: String,
    pub pubkey_x963_b64: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub platform: String,
    /// Which slot the CALLER believes this code is for.
    ///
    /// This never selects the slot: the code still does that, exactly as
    /// before (`registry.rs`, "the kind is bound to the CODE"). It is only
    /// ever used to REFUSE, which is a strictly weaker power and cannot be
    /// abused to claim a slot.
    ///
    /// Why it matters: without it, a device handed the wrong half of a
    /// two-code QR pairs successfully into the OTHER slot, notices only from
    /// the response, and throws its credentials away. The slot is then
    /// occupied forever by a device that cannot use it, and the rightful
    /// device can never pair. Optional so an older client still works.
    #[serde(default)]
    pub expect_kind: Option<String>,
}

/// `POST /relay/v1/pair` response body (D12.2 step 8's literal shape, plus
/// `kind`).
///
/// `kind` is additive and tells the device which slot it was actually admitted
/// to. A client that asked for the watch slot and is handed `"phone"` knows the
/// two codes in the QR were crossed and can refuse rather than quietly become
/// the phone. Older clients that do not decode the field are unaffected.
#[derive(Debug, Serialize)]
pub struct PairResponseOut {
    pub plane_url: String,
    pub device_id: String,
    pub device_token: String,
    pub org: String,
    pub role: String,
    pub kind: String,
}

pub async fn pair_handler(
    State(state): State<crate::PublicState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(req): Json<PairRequestIn>,
) -> Result<Json<PairResponseOut>, PairingError> {
    // Pre-auth route (the code itself is the only credential): rate-limit by
    // caller IP before touching the registry or the Cloud at all.
    if !state.pairing_rate_limiter.check(&peer.ip().to_string()) {
        return Err(PairingError::RateLimited);
    }

    if req.code.trim().is_empty() || req.pubkey_x963_b64.trim().is_empty() {
        return Err(PairingError::BadRequest(
            "code and pubkey_x963_b64 are required".to_string(),
        ));
    }

    // Resolve the code FIRST, because the code is what decides which slot is
    // being claimed (registry.rs: "the kind is bound to the CODE, never
    // claimed by the device"). `req.platform` is recorded for display only and
    // is never consulted here, so a device cannot take the watch slot by
    // calling itself a watch.
    //
    // This also makes the route darker than the V1 order did. Previously a
    // paired relay answered 409 `device_exists` to any caller, which told an
    // unauthenticated stranger that the relay was claimed; now an unknown code
    // gets the same flat 404 `not_found` whether or not anything is paired.
    // The only remaining 409 is a genuine race (see below).
    let now = crate::exceptions::now_unix();
    let kind = match state.registry.check_pairing_code(&req.code, now) {
        Ok(kind) => kind,
        Err(e) => {
            // This route is silent in normal operation: nobody reaches it
            // except the one device the operator is deliberately pairing, and
            // that device presents a code that works. So a miss is worth a
            // line with the source address. The per-IP limiter already caps
            // this at 10/min/IP, so it cannot be turned into a log flood.
            // Registry-side, the same miss bumps `failed_attempts` on every
            // live window; neither the log nor the counter closes anything.
            if matches!(e, RegistryError::WindowNotOpen) {
                eprintln!(
                    "genaryx-relay: pairing: invalid code presented from {} \
                     (no matching open window)",
                    peer.ip()
                );
            }
            return Err(e.into());
        }
    };

    // Refuse a crossed code BEFORE spending it upstream. Checking this after
    // redemption (which is where the client used to catch it) is too late:
    // the Cloud has already burned the code and the relay has already
    // committed the row, so the device discovers the mistake holding
    // credentials it will throw away, having permanently filled a slot the
    // rightful device now cannot claim. Here it costs one comparison and the
    // window survives for a correct retry.
    if let Some(raw) = req.expect_kind.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        match crate::registry::DeviceKind::parse(raw) {
            Some(expected) if expected == kind => {}
            Some(expected) => {
                eprintln!(
                    "genaryx-relay: pairing: refusing a crossed code (caller expected {}, \
                     the code admits {}); the pairing QR is likely built wrong",
                    expected.as_str(),
                    kind.as_str()
                );
                return Err(PairingError::KindMismatch);
            }
            None => {
                return Err(PairingError::BadRequest(
                    "expect_kind must be \"phone\" or \"watch\"".to_string(),
                ));
            }
        }
    }

    // Belt and braces, per slot: a window can never even be armed while that
    // kind is paired (`registry.rs::arm_pairing_window`), but check explicitly
    // anyway so a stray pre-existing window from a bug can never let a second
    // device into an occupied slot.
    if state.registry.has_device_of_kind(kind)? {
        return Err(PairingError::DeviceExists);
    }

    // Redeem the SAME code upstream. A rejection here (expired/invalid at
    // the Cloud's own clock, or a transport failure) leaves the relay's
    // window untouched, so the phone can simply retry within the TTL.
    let upstream = redeem_at_cloud(&state.http, &state.cloud_base_url, &req).await?;

    match state.registry.insert_paired_device(
        kind,
        NewDevice {
            device_id: upstream.device_id.clone(),
            name: req.name,
            platform: req.platform,
            org: upstream.org.clone(),
            role: upstream.role.clone(),
            device_token: upstream.device_token.clone(),
            paired_at_unix: now,
        },
    ) {
        Ok(()) => {}
        Err(RegistryError::DeviceExists) => {
            // Lost a race to a concurrent pairing attempt for the same
            // window, AFTER already redeeming (and thus burning) this code
            // at the Cloud: the phone that lost gets a real, but orphaned,
            // Cloud device_id/token it can never use locally. Cloud-side
            // cleanup needs the not-yet-existing `DELETE /v1/devices/{id}`
            // (itrat-console/13 D12.3 R5 / Appendix item 1) -- logged, not
            // silently dropped.
            eprintln!(
                "genaryx-relay: pairing: lost a concurrent-pairing race after redeeming \
                 at the Cloud (orphaned device_id={}); Cloud-side revoke is a later PR",
                upstream.device_id
            );
            return Err(PairingError::DeviceExists);
        }
        Err(e) => return Err(e.into()),
    }

    Ok(Json(PairResponseOut {
        plane_url: state.public_advertise_url.clone(),
        device_id: upstream.device_id,
        device_token: upstream.device_token,
        org: upstream.org,
        role: upstream.role,
        kind: kind.as_str().to_string(),
    }))
}

/// `POST {cloud_base_url}/v1/pair`: no bearer, the code itself is the
/// credential (`devices.rs` module doc). Exact wire shape of `http.rs::PairRequest`
/// (`code`, `pubkey_b64`, `platform`, `name`); the relay's own request DTO
/// names the key field `pubkey_x963_b64` (D12.2 step 6) to say plainly what
/// encoding it is, remapped to the Cloud's `pubkey_b64` field name here.
async fn redeem_at_cloud(
    http: &reqwest::Client,
    cloud_base_url: &str,
    req: &PairRequestIn,
) -> Result<PairResponse, PairingError> {
    #[derive(Serialize)]
    struct CloudPairRequest<'a> {
        code: &'a str,
        pubkey_b64: &'a str,
        platform: &'a str,
        name: &'a str,
    }

    let body = serde_json::to_vec(&CloudPairRequest {
        code: &req.code,
        pubkey_b64: &req.pubkey_x963_b64,
        platform: &req.platform,
        name: &req.name,
    })
    .map_err(|e| PairingError::Internal(format!("encoding upstream pair request: {e}")))?;

    let resp = http
        .post(format!("{cloud_base_url}/v1/pair"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| PairingError::UpstreamTransport(e.to_string()))?;

    let status = resp.status();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| PairingError::UpstreamTransport(e.to_string()))?;
    if status.is_success() {
        serde_json::from_slice(&bytes)
            .map_err(|e| PairingError::Internal(format!("upstream pair response: {e}")))
    } else {
        Err(PairingError::UpstreamRejected(
            String::from_utf8_lossy(&bytes).into_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_errors_map_to_the_right_pairing_error() {
        assert!(matches!(
            PairingError::from(RegistryError::DeviceExists),
            PairingError::DeviceExists
        ));
        assert!(matches!(
            PairingError::from(RegistryError::WindowNotOpen),
            PairingError::WindowNotOpen
        ));
    }

    /// A relay whose Cloud address points at a closed port. Any code path
    /// that reaches the Cloud therefore fails with `UpstreamTransport`, which
    /// is what makes the test below able to prove ORDERING and not just the
    /// final answer.
    fn state_with_dead_cloud() -> crate::PublicState {
        crate::PublicState {
            registry: std::sync::Arc::new(crate::registry::Registry::open_in_memory().unwrap()),
            engine: std::sync::Arc::new(crate::exceptions::ExceptionEngine::new("acme", 0.8, 600)),
            http: reqwest::Client::new(),
            cloud_base_url: "http://127.0.0.1:1".to_string(),
            public_advertise_url: "https://127.0.0.1:8443".to_string(),
            mutation_rate_limiter: std::sync::Arc::new(crate::ratelimit::RateLimiter::new(
                100,
                std::time::Duration::from_secs(60),
            )),
            pairing_rate_limiter: std::sync::Arc::new(crate::ratelimit::RateLimiter::new(
                100,
                std::time::Duration::from_secs(60),
            )),
        }
    }

    fn pair_request(code: &str, expect_kind: Option<&str>) -> PairRequestIn {
        PairRequestIn {
            code: code.to_string(),
            pubkey_x963_b64: "AAAA".to_string(),
            name: "test".to_string(),
            platform: "ios".to_string(),
            expect_kind: expect_kind.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn a_crossed_code_is_refused_before_it_is_ever_spent_at_the_cloud() {
        let state = state_with_dead_cloud();
        // A window armed for the WATCH slot.
        let code = "WATCHCODE";
        state
            .registry
            .arm_pairing_window(
                crate::registry::DeviceKind::Watch,
                &genaryx_signing::body_sha256_hex(code.as_bytes()),
                crate::exceptions::now_unix() + 300,
            )
            .unwrap();

        // A caller presenting it while believing it is the PHONE's code: the
        // shape a QR built with its two codes crossed produces.
        let peer = std::net::SocketAddr::from(([127, 0, 0, 1], 40_000));
        let err = pair_handler(
            axum::extract::State(state.clone()),
            axum::extract::ConnectInfo(peer),
            axum::Json(pair_request(code, Some("phone"))),
        )
        .await
        .expect_err("a crossed code must be refused");

        // KindMismatch, NOT UpstreamTransport: reaching the Cloud at all would
        // have produced the latter, since its address is a closed port. That
        // is the whole point, the code is never spent.
        assert!(
            matches!(err, PairingError::KindMismatch),
            "expected KindMismatch, got {err:?}"
        );

        // And the window is intact, so the RIGHT device can still use it.
        assert_eq!(
            state
                .registry
                .check_pairing_code(code, crate::exceptions::now_unix())
                .unwrap(),
            crate::registry::DeviceKind::Watch
        );
    }

    #[tokio::test]
    async fn a_matching_expectation_proceeds_to_the_cloud() {
        // The mirror of the test above: when the declared kind agrees, the
        // handler carries on and only then discovers the Cloud is unreachable.
        // Proves `expect_kind` gates nothing it should not.
        let state = state_with_dead_cloud();
        let code = "WATCHCODE";
        state
            .registry
            .arm_pairing_window(
                crate::registry::DeviceKind::Watch,
                &genaryx_signing::body_sha256_hex(code.as_bytes()),
                crate::exceptions::now_unix() + 300,
            )
            .unwrap();

        let peer = std::net::SocketAddr::from(([127, 0, 0, 1], 40_001));
        let err = pair_handler(
            axum::extract::State(state),
            axum::extract::ConnectInfo(peer),
            axum::Json(pair_request(code, Some("watch"))),
        )
        .await
        .expect_err("the dead Cloud still fails it, just later");
        assert!(
            matches!(err, PairingError::UpstreamTransport(_)),
            "expected to have reached the Cloud, got {err:?}"
        );
    }

    #[tokio::test]
    async fn an_unparseable_expectation_is_a_bad_request_not_a_silent_pass() {
        let state = state_with_dead_cloud();
        let code = "PHONECODE";
        state
            .registry
            .arm_pairing_window(
                crate::registry::DeviceKind::Phone,
                &genaryx_signing::body_sha256_hex(code.as_bytes()),
                crate::exceptions::now_unix() + 300,
            )
            .unwrap();

        let peer = std::net::SocketAddr::from(([127, 0, 0, 1], 40_002));
        let err = pair_handler(
            axum::extract::State(state),
            axum::extract::ConnectInfo(peer),
            axum::Json(pair_request(code, Some("laptop"))),
        )
        .await
        .expect_err("an unknown expect_kind must not be ignored");
        assert!(matches!(err, PairingError::BadRequest(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn redeem_against_no_live_cloud_is_a_transport_error() {
        // Skip-gracefully style (connectors crate's own live-test convention:
        // eprintln SKIP, never fail CI for lack of a live Cloud). No real
        // Cloud runs here; this proves the fail-closed transport-error path
        // never panics and never silently succeeds against a closed port.
        let http = reqwest::Client::new();
        let req = PairRequestIn {
            code: "ABCD1234".to_string(),
            pubkey_x963_b64: "AAAA".to_string(),
            name: "test".to_string(),
            platform: "ios".to_string(),
            expect_kind: None,
        };
        match redeem_at_cloud(&http, "http://127.0.0.1:1", &req).await {
            Err(PairingError::UpstreamTransport(_)) => {
                eprintln!(
                    "SKIP: redeem_against_no_live_cloud_is_a_transport_error (no live Cloud, \
                     fail-closed path proven against a closed port instead)"
                );
            }
            other => panic!("expected UpstreamTransport against a closed port, got {other:?}"),
        }
    }
}
