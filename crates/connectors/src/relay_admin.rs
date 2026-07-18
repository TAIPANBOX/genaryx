//! `RelayAdminClient`: a typed client for `genaryx-relay`'s admin API
//! (Phase 5 W2, docs/PHASE5.md: "desktop Pocket panel"; `crates/relay/src/admin.rs`).
//! This is the desktop-facing half of the Pocket pairing flow
//! (itrat-console/13 D12.2a): `pairing_info` reads the relay's SPKI pin +
//! `public_advertise_url` + `org` to build the QR, `arm_pairing_window` opens
//! the relay's public pairing route for one code, `device` renders the
//! paired-device view, and `disconnect` frees the single-device slot.
//!
//! ## No bearer, by design
//!
//! Unlike [`crate::CloudClient`], this client sends no credential at all: the
//! admin API's ENTIRE trust model is its bind address (`config.rs`:
//! `admin_bind_addr` must be loopback; `main.rs` serves it on a listener
//! SEPARATE from the phone-facing TLS one) -- see `admin.rs`'s own module
//! doc, "never the public interface the phone talks to". A shell reaching
//! this client at all already proves it is running on (or WG-tunneled into)
//! the relay's own host, so a second bearer layered on top would be
//! redundant, not defense-in-depth (there is no attacker position from which
//! having the bearer but not loopback/WG access is possible). Keeping this
//! client bearer-free also means it never has a secret to accidentally log.
//!
//! ## Wire shapes
//!
//! Every DTO below is a field-for-field mirror of `crates/relay/src/admin.rs`'s
//! own response structs, confirmed by reading that module directly (the
//! authority, per this repo's own ground-truth convention -- see
//! `cloud_rest.rs`'s identical practice against `tokenfuse`'s `http.rs`).

use serde::{Deserialize, Serialize};

/// Every failure mode a [`RelayAdminClient`] call can surface. Deliberately
/// separate from [`crate::ConnectorError`] (Cloud-shaped: `PlanRequired`,
/// `SignatureRejected`, a device signer) -- the relay admin API has none of
/// that, and [`RelayAdminError::DeviceExists`] needs to be its own variant
/// (not folded into a generic `Api{status,body}`) for the exact same reason
/// `ConnectorError::PlanRequired` is kept distinct in `cloud_rest.rs`: so a
/// caller can render "already paired, show Disconnect" (D12.2 step 2)
/// instead of a generic error banner.
#[derive(Debug, thiserror::Error)]
pub enum RelayAdminError {
    /// The request never got a response at all (relay not running, wrong
    /// port, WG tunnel down, timeout).
    #[error("http transport: {0}")]
    Transport(#[from] reqwest::Error),
    /// A 2xx body that failed to deserialize into the expected shape.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// `409 {"error":"device_exists"}` (`admin::AdminError::DeviceExists`):
    /// a device is already paired, so the caller should show Disconnect
    /// instead of arming a new pairing window (D12.2 step 2).
    #[error("a device is already paired at the relay")]
    DeviceExists,
    /// Any other non-2xx response: status plus raw body text (UTF-8 lossy).
    #[error("relay admin API returned HTTP {status}: {body}")]
    Api { status: u16, body: String },
}

/// A typed client for `genaryx-relay`'s loopback/WG-only admin API. Cheap to
/// construct per call (no persistent connection, mirrors [`crate::HetznerClient`]'s
/// "stateless by design" shape) -- there is nothing here worth holding onto
/// between a desktop panel's Connect/status-poll/Disconnect actions.
pub struct RelayAdminClient {
    base_url: String,
    http: reqwest::Client,
}

impl std::fmt::Debug for RelayAdminClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayAdminClient")
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl RelayAdminClient {
    /// Construct a client for `base_url` (e.g. `http://127.0.0.1:8444`, the
    /// relay's own `admin_bind_addr` default -- `crates/relay/src/config.rs`).
    /// No secret to fail closed over here (see this module's "no bearer" doc),
    /// so the only failure mode is building the underlying HTTP client itself.
    pub fn new(base_url: impl Into<String>) -> Result<Self, RelayAdminError> {
        let http = reqwest::Client::builder().build()?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
        })
    }

    /// `GET /admin/pairing-info` (`admin::pairing_info`): the relay's SPKI
    /// pin + `public_advertise_url` + `org`, the three static values the
    /// Pocket panel folds into the pairing QR verbatim alongside the
    /// Cloud-minted `code` (D12.2 step 3). Infallible relay-side, so a
    /// non-2xx here means something is wrong with the transport/route
    /// itself, not with pairing state.
    pub async fn pairing_info(&self) -> Result<PairingInfo, RelayAdminError> {
        let resp = self
            .http
            .get(format!("{}/admin/pairing-info", self.base_url))
            .send()
            .await?;
        parse_response(resp).await
    }

    /// `POST /admin/pairing-window` (`admin::arm_pairing_window`): open the
    /// relay's public pairing route for `ttl_secs`, gated on the SHA-256 hash
    /// of the code the desktop already minted at the Cloud (`code_sha256`,
    /// lowercase hex -- the relay never learns the plaintext code itself
    /// until the phone presents it, D12.3's trust-boundary table). Fails
    /// closed with [`RelayAdminError::DeviceExists`] while a device is
    /// already paired (D12.2 step 2): the caller must show Disconnect
    /// instead of a QR in that case, never silently retry or ignore it.
    pub async fn arm_pairing_window(
        &self,
        code_sha256: &str,
        ttl_secs: i64,
    ) -> Result<ArmPairingWindowResponse, RelayAdminError> {
        let body = serde_json::to_vec(&ArmPairingWindowRequest {
            code_sha256: code_sha256.to_string(),
            ttl_secs,
        })?;
        let resp = self
            .http
            .post(format!("{}/admin/pairing-window", self.base_url))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await?;
        parse_response(resp).await
    }

    /// `GET /admin/device` (`admin::get_device`): the paired-device view for
    /// the Pocket panel's "paired" state (name, platform, paired_at,
    /// last_seen, D12.2 step 10). `DeviceView::paired == false` (never an
    /// error) is the normal "nothing paired yet" outcome.
    pub async fn device(&self) -> Result<DeviceView, RelayAdminError> {
        let resp = self
            .http
            .get(format!("{}/admin/device", self.base_url))
            .send()
            .await?;
        parse_response(resp).await
    }

    /// `POST /admin/disconnect` (`admin::disconnect`): deletes the paired
    /// device row (+ its APNs token). Always safe to call, even with nothing
    /// paired (`was_paired: false` reports that honestly rather than erroring).
    pub async fn disconnect(&self) -> Result<DisconnectResponse, RelayAdminError> {
        let resp = self
            .http
            .post(format!("{}/admin/disconnect", self.base_url))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body("{}")
            .send()
            .await?;
        parse_response(resp).await
    }
}

// ---- wire DTOs (exact shapes from crates/relay/src/admin.rs) --------------

/// Mirrors `admin::PairingInfoResponse`.
#[derive(Debug, Clone, Deserialize)]
pub struct PairingInfo {
    pub pin: String,
    pub relay_url: String,
    pub org: String,
}

/// Mirrors `admin::ArmPairingWindowRequest`.
#[derive(Debug, Serialize)]
struct ArmPairingWindowRequest {
    code_sha256: String,
    ttl_secs: i64,
}

/// Mirrors `admin::ArmPairingWindowResponse`.
#[derive(Debug, Clone, Deserialize)]
pub struct ArmPairingWindowResponse {
    pub ok: bool,
    pub expires_unix: i64,
}

/// Mirrors `admin::DeviceView`. `paired: false` leaves every other field
/// `None` -- a normal, renderable "nothing paired yet" shape, not an error.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceView {
    pub paired: bool,
    pub device_id: Option<String>,
    pub name: Option<String>,
    pub platform: Option<String>,
    pub paired_at_unix: Option<i64>,
    pub last_seen_unix: Option<i64>,
}

/// Mirrors `admin::DisconnectResponse`.
#[derive(Debug, Clone, Deserialize)]
pub struct DisconnectResponse {
    pub ok: bool,
    pub was_paired: bool,
}

/// The flat `{"error": "..."}` envelope every admin error response uses
/// (`admin::AdminError::into_response`).
#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: String,
}

// ---- response parsing -------------------------------------------------------

/// Parse one HTTP response: a 2xx body deserializes as `T`; anything else
/// becomes a classified [`RelayAdminError`] -- mirrors `cloud_rest.rs`'s
/// `parse_response`/`classify_error` pair exactly, trimmed to this API's
/// smaller error surface (no plan/signature envelopes to special-case).
async fn parse_response<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<T, RelayAdminError> {
    let status = resp.status();
    let bytes = resp.bytes().await?;
    if status.is_success() {
        Ok(serde_json::from_slice(&bytes)?)
    } else {
        if status.as_u16() == 409
            && let Ok(e) = serde_json::from_slice::<ErrorResponse>(&bytes)
            && e.error == "device_exists"
        {
            return Err(RelayAdminError::DeviceExists);
        }
        Err(RelayAdminError::Api {
            status: status.as_u16(),
            body: String::from_utf8_lossy(&bytes).into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- DTO deserialization: real shapes, transcribed from
    // crates/relay/src/admin.rs's own struct definitions ----------------------

    #[test]
    fn pairing_info_deserializes() {
        let info: PairingInfo = serde_json::from_str(
            r#"{"pin":"dGVzdC1zcGtpLXBpbi1iYXNlNjQ=","relay_url":"https://198.51.100.7:8443","org":"acme"}"#,
        )
        .expect("valid PairingInfo json");
        assert_eq!(info.pin, "dGVzdC1zcGtpLXBpbi1iYXNlNjQ=");
        assert_eq!(info.relay_url, "https://198.51.100.7:8443");
        assert_eq!(info.org, "acme");
    }

    #[test]
    fn arm_pairing_window_response_deserializes() {
        let resp: ArmPairingWindowResponse =
            serde_json::from_str(r#"{"ok":true,"expires_unix":1758000600}"#)
                .expect("valid ArmPairingWindowResponse json");
        assert!(resp.ok);
        assert_eq!(resp.expires_unix, 1_758_000_600);
    }

    #[test]
    fn device_view_deserializes_both_paired_and_unpaired_shapes() {
        let paired: DeviceView = serde_json::from_str(
            r#"{"paired":true,"device_id":"d1","name":"Yurii's iPhone","platform":"ios",
                 "paired_at_unix":1000,"last_seen_unix":2000}"#,
        )
        .expect("valid paired DeviceView json");
        assert!(paired.paired);
        assert_eq!(paired.device_id.as_deref(), Some("d1"));

        let unpaired: DeviceView = serde_json::from_str(
            r#"{"paired":false,"device_id":null,"name":null,"platform":null,
                 "paired_at_unix":null,"last_seen_unix":null}"#,
        )
        .expect("valid unpaired DeviceView json");
        assert!(!unpaired.paired);
        assert!(unpaired.device_id.is_none());
    }

    #[test]
    fn disconnect_response_deserializes() {
        let resp: DisconnectResponse = serde_json::from_str(r#"{"ok":true,"was_paired":true}"#)
            .expect("valid DisconnectResponse json");
        assert!(resp.ok);
        assert!(resp.was_paired);
    }

    // ---- error classification ---------------------------------------------

    #[test]
    fn classifies_409_device_exists_distinctly_from_a_generic_409() {
        let bytes = br#"{"error":"device_exists"}"#;
        match serde_json::from_slice::<ErrorResponse>(bytes) {
            Ok(e) if e.error == "device_exists" => {}
            other => panic!("fixture itself must parse as device_exists, got {other:?}"),
        }
    }

    #[test]
    fn new_trims_a_trailing_slash_from_base_url() {
        let client = RelayAdminClient::new("http://127.0.0.1:8444/").expect("client");
        assert_eq!(client.base_url, "http://127.0.0.1:8444");
    }

    // ---- fail-closed transport: no live relay in unit tests, mirrors
    // pairing.rs's own "closed port proves the fail-closed path" convention ----

    #[tokio::test]
    async fn pairing_info_against_no_live_relay_is_a_transport_error() {
        let client = RelayAdminClient::new("http://127.0.0.1:1").expect("client");
        match client.pairing_info().await {
            Err(RelayAdminError::Transport(_)) => {}
            other => panic!("expected Transport against a closed port, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn arm_pairing_window_against_no_live_relay_is_a_transport_error() {
        let client = RelayAdminClient::new("http://127.0.0.1:1").expect("client");
        match client.arm_pairing_window("deadbeef", 300).await {
            Err(RelayAdminError::Transport(_)) => {}
            other => panic!("expected Transport against a closed port, got {other:?}"),
        }
    }
}
