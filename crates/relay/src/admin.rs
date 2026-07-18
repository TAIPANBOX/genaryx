//! Admin API (docs/PHASE5.md "admin" module; itrat-console/13 D12.2 step 2,
//! 10): pairing-window arm, paired-device view, disconnect, plus (Phase 5 W2)
//! `GET /admin/pairing-info` so the desktop's Pocket panel can build the
//! pairing QR without duplicating the relay's own TLS/config internals.
//! Served on a SEPARATE listener bound to loopback only (`main.rs` refuses to
//! construct it on anything else, mirroring `config.rs`'s own validation) --
//! never the public interface the phone talks to.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::registry::RegistryError;

#[derive(Debug, thiserror::Error)]
pub enum AdminError {
    /// A device is already paired -- the desktop shows Disconnect instead
    /// (D12.2 step 2).
    #[error("device already paired")]
    DeviceExists,
    #[error("invalid request: {0}")]
    BadRequest(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl From<RegistryError> for AdminError {
    fn from(e: RegistryError) -> Self {
        match e {
            RegistryError::DeviceExists => AdminError::DeviceExists,
            other => AdminError::Internal(other.to_string()),
        }
    }
}

impl IntoResponse for AdminError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            AdminError::DeviceExists => (StatusCode::CONFLICT, "device_exists"),
            AdminError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            AdminError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };
        (status, Json(serde_json::json!({ "error": code }))).into_response()
    }
}

/// `GET /admin/pairing-info` response (Phase 5 W2, docs/PHASE5.md: "the
/// desktop needs the relay's SPKI pin + `public_advertise_url` + `org` to
/// build the QR"): the three static, license-free values the Pocket panel
/// folds into the `genaryx-pocket://pair/v1?relay=...&pin=...&code=...&org=...`
/// QR content (D12.2 step 3) verbatim -- `code` itself comes from the
/// SEPARATE `POST /v1/pair/new` call at the Cloud (admin key), never from
/// this endpoint, so this response alone is never enough to mint a working
/// pairing QR (D12.3's trust-boundary table: the relay holds no admin
/// authority of its own).
#[derive(Debug, Serialize)]
pub struct PairingInfoResponse {
    /// The public listener's SPKI-SHA256 pin, base64 (`tls.rs::spki_sha256_b64`).
    pub pin: String,
    /// `public_advertise_url` (`config.rs`) -- what the relay tells a pairing
    /// phone its own base URL is; may differ from the raw bind address.
    pub relay_url: String,
    /// The org this relay serves (single-tenant per relay instance, `config.rs`).
    pub org: String,
}

/// Infallible: every field is a value the relay already resolved at startup
/// (TLS identity, config) and holds for its whole lifetime, so there is
/// nothing here that can fail per-request.
pub async fn pairing_info(State(state): State<crate::AdminState>) -> Json<PairingInfoResponse> {
    Json(PairingInfoResponse {
        pin: state.pin.clone(),
        relay_url: state.relay_url.clone(),
        org: state.org.clone(),
    })
}

/// `POST /admin/pairing-window` request: the desktop already knows the
/// plaintext code (it minted it via `POST /v1/pair/new`) and sends only the
/// hash -- the relay never learns the code itself until the phone presents
/// it (D12.3's trust-boundary table).
#[derive(Debug, Deserialize)]
pub struct ArmPairingWindowRequest {
    pub code_sha256: String,
    pub ttl_secs: i64,
}

#[derive(Debug, Serialize)]
pub struct ArmPairingWindowResponse {
    pub ok: bool,
    pub expires_unix: i64,
}

/// Arm the pairing window; refuses with `409 device_exists` while a device
/// is paired (D12.2 step 2: "If a device is already paired, the relay
/// refuses with `device_exists` and the desktop shows the Disconnect button
/// instead").
pub async fn arm_pairing_window(
    State(state): State<crate::AdminState>,
    Json(req): Json<ArmPairingWindowRequest>,
) -> Result<Json<ArmPairingWindowResponse>, AdminError> {
    if req.code_sha256.trim().is_empty() {
        return Err(AdminError::BadRequest(
            "code_sha256 is required".to_string(),
        ));
    }
    if req.ttl_secs <= 0 {
        return Err(AdminError::BadRequest(
            "ttl_secs must be positive".to_string(),
        ));
    }
    let now = crate::exceptions::now_unix();
    let expires_unix = now + req.ttl_secs;
    state
        .registry
        .arm_pairing_window(&req.code_sha256, expires_unix)?;
    Ok(Json(ArmPairingWindowResponse {
        ok: true,
        expires_unix,
    }))
}

/// `GET /admin/device`: the paired-device view for the desktop's Pocket
/// panel (D12.2 step 10: "name, platform, paired_at, last_seen") --
/// deliberately never includes `device_token` (a bearer secret).
#[derive(Debug, Serialize)]
pub struct DeviceView {
    pub paired: bool,
    pub device_id: Option<String>,
    pub name: Option<String>,
    pub platform: Option<String>,
    pub paired_at_unix: Option<i64>,
    pub last_seen_unix: Option<i64>,
}

pub async fn get_device(
    State(state): State<crate::AdminState>,
) -> Result<Json<DeviceView>, AdminError> {
    let device = state.registry.current_device()?;
    Ok(Json(match device {
        Some(d) => DeviceView {
            paired: true,
            device_id: Some(d.device_id),
            name: Some(d.name),
            platform: Some(d.platform),
            paired_at_unix: Some(d.paired_at_unix),
            last_seen_unix: Some(d.last_seen_unix),
        },
        None => DeviceView {
            paired: false,
            device_id: None,
            name: None,
            platform: None,
            paired_at_unix: None,
            last_seen_unix: None,
        },
    }))
}

#[derive(Debug, Serialize)]
pub struct DisconnectResponse {
    pub ok: bool,
    pub was_paired: bool,
}

/// `POST /admin/disconnect`: deletes the device row (+ its APNs token, same
/// row). Upstream Cloud-side revocation of the device token needs the
/// not-yet-existing `DELETE /v1/devices/{id}` (D12.3 R5 / Appendix item 1) --
/// noted explicitly here rather than silently implied by a 200.
pub async fn disconnect(
    State(state): State<crate::AdminState>,
) -> Result<Json<DisconnectResponse>, AdminError> {
    let was_paired = state.registry.disconnect()?;
    if was_paired {
        eprintln!(
            "genaryx-relay: admin: disconnected the paired device (Cloud-side token revoke \
             is a later PR, itrat-console/13 D12.3 R5)"
        );
    }
    Ok(Json(DisconnectResponse {
        ok: true,
        was_paired,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_device_exists_maps_to_admin_device_exists() {
        assert!(matches!(
            AdminError::from(RegistryError::DeviceExists),
            AdminError::DeviceExists
        ));
    }

    #[test]
    fn registry_window_not_open_maps_to_internal_not_device_exists() {
        // `WindowNotOpen` should never surface from an admin-side registry
        // call in practice (admin never checks a code), but the mapping must
        // still be safe/explicit rather than silently misclassified as
        // `DeviceExists`.
        assert!(matches!(
            AdminError::from(RegistryError::WindowNotOpen),
            AdminError::Internal(_)
        ));
    }

    fn test_state(pin: &str, relay_url: &str, org: &str) -> crate::AdminState {
        crate::AdminState {
            registry: std::sync::Arc::new(crate::registry::Registry::open_in_memory().unwrap()),
            pin: pin.to_string(),
            relay_url: relay_url.to_string(),
            org: org.to_string(),
        }
    }

    #[tokio::test]
    async fn pairing_info_echoes_the_configured_pin_relay_url_and_org() {
        let state = test_state(
            "dGVzdC1zcGtpLXBpbi1iYXNlNjQ=",
            "https://198.51.100.7:8443",
            "acme",
        );
        let Json(resp) = pairing_info(State(state)).await;
        assert_eq!(resp.pin, "dGVzdC1zcGtpLXBpbi1iYXNlNjQ=");
        assert_eq!(resp.relay_url, "https://198.51.100.7:8443");
        assert_eq!(resp.org, "acme");
    }

    #[tokio::test]
    async fn pairing_info_never_touches_the_registry() {
        // The QR content this response feeds (D12.2 step 3) is entirely
        // static per relay instance -- proving this handler is infallible
        // and side-effect-free, unlike arm_pairing_window/get_device/
        // disconnect which all read or write the device row.
        let state = test_state("pin", "https://127.0.0.1:8443", "org");
        assert!(state.registry.current_device().unwrap().is_none());
        let _ = pairing_info(State(state.clone())).await;
        assert!(state.registry.current_device().unwrap().is_none());
    }
}
