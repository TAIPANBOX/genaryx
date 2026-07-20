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
/// hash -- the relay never learns the code itself until the device presents
/// it (D12.3's trust-boundary table).
///
/// `kind` says which slot this code admits, and is REQUIRED rather than
/// defaulted. A two-code QR arms this endpoint twice, once per kind, and a
/// silent default would mean a desktop that forgot the field quietly armed a
/// phone window for a code it minted for the watch. The device would then be
/// admitted into the wrong slot with a perfectly valid code, which is exactly
/// the confusion `registry.rs` binds kind-to-code to prevent.
#[derive(Debug, Deserialize)]
pub struct ArmPairingWindowRequest {
    pub kind: String,
    pub code_sha256: String,
    pub ttl_secs: i64,
}

#[derive(Debug, Serialize)]
pub struct ArmPairingWindowResponse {
    pub ok: bool,
    pub expires_unix: i64,
}

/// Hard ceiling on how long a pairing window may stay open, whatever the
/// desktop asks for.
///
/// ## Why there is a ceiling at all, and why it is 15 minutes
///
/// An armed window is a LIVE CREDENTIAL sitting on the one listener this
/// process exposes to the internet, and the two-device flow made that worse in
/// a way worth naming: the phone spends its code seconds after the scan, but
/// the watch's code stays unspent until the phone hands it over and the watch
/// app actually runs. The tempting fix is a long TTL so the handoff "always
/// works". That trades a security property for a convenience one, silently.
///
/// So: 15 minutes, and the operator is told what to do instead of the clock
/// being stretched. If the watch has not picked up its code in that time, the
/// honest instruction is "open TokenFuse on your watch and scan again", not a
/// window left open for an hour in case someone gets round to it. `ttl_secs`
/// was previously validated only as positive, so a desktop bug or a careless
/// operator could have armed a window for a week.
pub const MAX_PAIRING_WINDOW_SECS: i64 = 900;

/// Arm one kind's pairing window; refuses with `409 device_exists` while a
/// device of THAT kind is paired (D12.2 step 2: "If a device is already
/// paired, the relay refuses with `device_exists` and the desktop shows the
/// Disconnect button instead"). A paired phone does not block arming the
/// watch, and vice versa.
pub async fn arm_pairing_window(
    State(state): State<crate::AdminState>,
    Json(req): Json<ArmPairingWindowRequest>,
) -> Result<Json<ArmPairingWindowResponse>, AdminError> {
    let Some(kind) = crate::registry::DeviceKind::parse(req.kind.trim()) else {
        return Err(AdminError::BadRequest(
            "kind must be \"phone\" or \"watch\"".to_string(),
        ));
    };
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
    if req.ttl_secs > MAX_PAIRING_WINDOW_SECS {
        return Err(AdminError::BadRequest(format!(
            "ttl_secs must not exceed {MAX_PAIRING_WINDOW_SECS}"
        )));
    }
    let now = crate::exceptions::now_unix();
    let expires_unix = now + req.ttl_secs;
    state
        .registry
        .arm_pairing_window(kind, &req.code_sha256, expires_unix)?;
    Ok(Json(ArmPairingWindowResponse {
        ok: true,
        expires_unix,
    }))
}

/// One pairing slot, as the desktop's Pocket panel renders it (D12.2 step 10:
/// "name, platform, paired_at, last_seen") -- deliberately never includes
/// `device_token` (a bearer secret).
#[derive(Debug, Serialize)]
pub struct DeviceView {
    /// `"phone"` or `"watch"`: which slot this entry describes. Present even
    /// when the slot is empty, so the panel can draw both rows unconditionally.
    pub kind: String,
    pub paired: bool,
    pub device_id: Option<String>,
    pub name: Option<String>,
    pub platform: Option<String>,
    pub paired_at_unix: Option<i64>,
    pub last_seen_unix: Option<i64>,
}

/// `GET /admin/devices`: every slot, always one entry per
/// [`crate::registry::DeviceKind`], paired or not.
///
/// Renamed from the V1 `GET /admin/device` on purpose. The body changed shape
/// (one device became a list of slots), and reusing the old path would let a
/// stale desktop mis-parse the new body in silence; a new path makes the
/// version mismatch a loud 404 instead.
/// One armed pairing window, as the Pocket panel shows it. No secret here: the
/// code hash never leaves the registry.
#[derive(Debug, Serialize)]
pub struct WindowView {
    pub kind: String,
    pub expires_unix: i64,
    /// Wrong codes presented since this window was armed.
    ///
    /// Purely observational. The relay never closes a window over this, and
    /// deliberately so: `POST /relay/v1/pair` is pre-auth, so an unauthenticated
    /// caller could otherwise deny pairing at will, and the watch's window (the
    /// long-lived one, waiting on a WatchConnectivity handoff) would be the
    /// easiest thing in the system to keep permanently shut.
    ///
    /// What it is FOR: this route is silent in normal operation, so a nonzero
    /// value means somebody is probing it. Render it next to the countdown and
    /// let the operator decide, through this same authenticated loopback API,
    /// whether to disarm.
    pub failed_attempts: i64,
}

#[derive(Debug, Serialize)]
pub struct DevicesResponse {
    pub devices: Vec<DeviceView>,
    /// Currently armed windows, if any. Empty in the normal steady state.
    pub windows: Vec<WindowView>,
}

pub async fn get_devices(
    State(state): State<crate::AdminState>,
) -> Result<Json<DevicesResponse>, AdminError> {
    let paired = state.registry.devices()?;
    let windows = state
        .registry
        .pairing_window_states()?
        .into_iter()
        .map(|w| WindowView {
            kind: w.kind.as_str().to_string(),
            expires_unix: w.expires_unix,
            failed_attempts: w.failed_attempts,
        })
        .collect();
    let devices = crate::registry::DeviceKind::ALL
        .into_iter()
        .map(|kind| match paired.iter().find(|d| d.kind == kind) {
            Some(d) => DeviceView {
                kind: kind.as_str().to_string(),
                paired: true,
                device_id: Some(d.device_id.clone()),
                name: Some(d.name.clone()),
                platform: Some(d.platform.clone()),
                paired_at_unix: Some(d.paired_at_unix),
                last_seen_unix: Some(d.last_seen_unix),
            },
            None => DeviceView {
                kind: kind.as_str().to_string(),
                paired: false,
                device_id: None,
                name: None,
                platform: None,
                paired_at_unix: None,
                last_seen_unix: None,
            },
        })
        .collect();
    Ok(Json(DevicesResponse { devices, windows }))
}

/// `POST /admin/disconnect` request. An absent or null `kind` revokes every
/// slot (the V1 "Disconnect" button); naming a kind revokes just that surface,
/// which is why the two devices carry separate tokens: a lost watch must not
/// force the phone to re-pair.
#[derive(Debug, Default, Deserialize)]
pub struct DisconnectRequest {
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DisconnectResponse {
    pub ok: bool,
    /// How many device rows were actually removed (0, 1 or 2).
    pub disconnected: usize,
    /// Kept for the panel's existing "was anything there?" phrasing.
    pub was_paired: bool,
}

/// `POST /admin/disconnect`: deletes the device rows (+ their APNs tokens,
/// same rows). Upstream Cloud-side revocation of the device tokens needs the
/// not-yet-existing `DELETE /v1/devices/{id}` (D12.3 R5 / Appendix item 1) --
/// noted explicitly here rather than silently implied by a 200.
pub async fn disconnect(
    State(state): State<crate::AdminState>,
    body: Option<Json<DisconnectRequest>>,
) -> Result<Json<DisconnectResponse>, AdminError> {
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let kind = match req.kind.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(raw) => match crate::registry::DeviceKind::parse(raw) {
            Some(k) => Some(k),
            None => {
                return Err(AdminError::BadRequest(
                    "kind must be \"phone\", \"watch\", or omitted for all".to_string(),
                ));
            }
        },
    };
    let disconnected = state.registry.disconnect(kind)?;
    if disconnected > 0 {
        eprintln!(
            "genaryx-relay: admin: disconnected {disconnected} device(s) ({}) (Cloud-side \
             token revoke is a later PR, itrat-console/13 D12.3 R5)",
            kind.map(|k| k.as_str()).unwrap_or("all slots")
        );
    }
    Ok(Json(DisconnectResponse {
        ok: true,
        disconnected,
        was_paired: disconnected > 0,
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
    async fn a_pairing_window_cannot_be_armed_for_longer_than_the_ceiling() {
        let state = test_state("pin", "https://127.0.0.1:8443", "acme");

        // At the ceiling: fine.
        let ok = arm_pairing_window(
            State(state.clone()),
            Json(ArmPairingWindowRequest {
                kind: "phone".to_string(),
                code_sha256: "a".repeat(64),
                ttl_secs: MAX_PAIRING_WINDOW_SECS,
            }),
        )
        .await;
        assert!(ok.is_ok(), "the ceiling itself must be allowed");

        // One second past it: refused, rather than quietly leaving a live
        // credential on the public listener for as long as anyone asked.
        let err = arm_pairing_window(
            State(state),
            Json(ArmPairingWindowRequest {
                kind: "watch".to_string(),
                code_sha256: "b".repeat(64),
                ttl_secs: MAX_PAIRING_WINDOW_SECS + 1,
            }),
        )
        .await
        .expect_err("over the ceiling must be refused");
        assert!(matches!(err, AdminError::BadRequest(_)));
    }

    #[tokio::test]
    async fn arming_requires_a_kind_this_build_knows() {
        let state = test_state("pin", "https://127.0.0.1:8443", "acme");
        for raw in ["", "laptop", "Phone", "tablet"] {
            let err = arm_pairing_window(
                State(state.clone()),
                Json(ArmPairingWindowRequest {
                    kind: raw.to_string(),
                    code_sha256: "c".repeat(64),
                    ttl_secs: 300,
                }),
            )
            .await
            .expect_err("an unknown kind must be refused, never defaulted");
            assert!(matches!(err, AdminError::BadRequest(_)), "kind {raw:?}");
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
        // and side-effect-free, unlike arm_pairing_window/get_devices/
        // disconnect which all read or write the device rows.
        let state = test_state("pin", "https://127.0.0.1:8443", "org");
        assert!(state.registry.devices().unwrap().is_empty());
        let _ = pairing_info(State(state.clone())).await;
        assert!(state.registry.devices().unwrap().is_empty());
    }
}
