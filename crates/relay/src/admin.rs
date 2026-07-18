//! Admin API (docs/PHASE5.md "admin" module; itrat-console/13 D12.2 step 2,
//! 10): pairing-window arm, paired-device view, disconnect. Served on a
//! SEPARATE listener bound to loopback only (`main.rs` refuses to construct
//! it on anything else, mirroring `config.rs`'s own validation) -- never the
//! public interface the phone talks to.

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
}
