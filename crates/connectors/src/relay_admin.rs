//! `RelayAdminClient`: a typed client for `genaryx-relay`'s admin API
//! (Phase 5 W2, docs/PHASE5.md: "desktop Pocket panel"; `crates/relay/src/admin.rs`).
//! This is the desktop-facing half of the Pocket pairing flow
//! (itrat-console/13 D12.2a): `pairing_info` reads the relay's SPKI pin +
//! `public_advertise_url` + `org` to build the QR, `arm_pairing_window` opens
//! the relay's public pairing route for one kind and one code, `devices`
//! renders both pairing slots plus any currently-armed windows, and
//! `disconnect` frees one slot (or every slot at once).
//!
//! ## One operator, two devices
//!
//! The relay now pairs a phone and a watch independently -- at most one
//! device PER KIND, and a paired phone never blocks arming the watch's
//! window, or vice versa (see `crates/relay/src/registry.rs`'s own "One
//! operator, two devices" doc, the authority this client's shapes are
//! transcribed from). Every call that once named "the" device now names a
//! [`DeviceKind`] instead.
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

/// Which pager surface a pairing/disconnect call targets. Mirrors
/// `genaryx-relay`'s own `registry::DeviceKind` spelling exactly
/// (`as_str`/`parse`), but is its own type here rather than a shared one:
/// `genaryx-relay` already depends on THIS crate (it reuses [`PairResponse`]
/// to redeem a code at the Cloud, `crates/relay/src/pairing.rs`), so a
/// dependency back the other way would be circular. Two small,
/// independently tested copies of a two-variant enum is a better trade than
/// restructuring either crate's dependency graph just to share it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    Phone,
    Watch,
}

impl DeviceKind {
    /// The wire spelling, exactly as `crates/relay/src/registry.rs::DeviceKind::as_str`
    /// writes and reads it back.
    pub fn as_str(self) -> &'static str {
        match self {
            DeviceKind::Phone => "phone",
            DeviceKind::Watch => "watch",
        }
    }

    /// Parse the wire spelling. `None` for anything else -- callers never
    /// default a bad string into a kind, mirroring the relay's own
    /// `DeviceKind::parse` contract exactly.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "phone" => Some(DeviceKind::Phone),
            "watch" => Some(DeviceKind::Watch),
            _ => None,
        }
    }

    /// Both kinds, phone first -- the same order `GET /admin/devices` itself
    /// returns them in, for a caller that wants to walk both slots without
    /// hardcoding the pair a second time.
    pub const ALL: [DeviceKind; 2] = [DeviceKind::Phone, DeviceKind::Watch];
}

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
    /// a device of the requested kind is already paired, so the caller
    /// should show Disconnect for that slot instead of arming a new pairing
    /// window (D12.2 step 2). A phone already paired never causes this for
    /// the watch, or vice versa -- the conflict is always about the ONE
    /// kind the caller just asked to arm.
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
    /// Pocket panel folds into the pairing QR verbatim alongside the two
    /// Cloud-minted codes (D12.2 step 3). Infallible relay-side, so a
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
    /// relay's public pairing route for `kind`, for `ttl_secs`, gated on the
    /// SHA-256 hash of the code the desktop already minted at the Cloud for
    /// that kind (`code_sha256`, lowercase hex -- the relay never learns the
    /// plaintext code itself until the device presents it, D12.3's
    /// trust-boundary table). Fails closed with [`RelayAdminError::DeviceExists`]
    /// while a device of `kind` is already paired (D12.2 step 2): the caller
    /// must show Disconnect for that slot instead of a QR in that case,
    /// never silently retry or ignore it. A phone already paired does not
    /// stop a watch window from arming, and vice versa.
    pub async fn arm_pairing_window(
        &self,
        kind: DeviceKind,
        code_sha256: &str,
        ttl_secs: i64,
    ) -> Result<ArmPairingWindowResponse, RelayAdminError> {
        let body = serde_json::to_vec(&ArmPairingWindowRequest {
            kind: kind.as_str(),
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

    /// `GET /admin/devices` (`admin::get_devices`): both pairing slots for
    /// the Pocket panel's device view (name, platform, paired_at, last_seen,
    /// D12.2 step 10) -- always one [`DeviceView`] per [`DeviceKind`], paired
    /// or not (`DeviceView::paired == false` is the normal "nothing in this
    /// slot yet" outcome, never an error). Renamed from V1's `GET
    /// /admin/device` on the relay side because the body shape changed (one
    /// device became a list of slots); this client follows that rename
    /// rather than keeping the old path name for a new shape.
    ///
    /// Also carries [`DevicesResponse::windows`]: any pairing windows
    /// currently armed, purely observational (see [`WindowView`]'s own doc).
    /// Empty in the normal steady state, so most callers never see it.
    pub async fn devices(&self) -> Result<DevicesResponse, RelayAdminError> {
        let resp = self
            .http
            .get(format!("{}/admin/devices", self.base_url))
            .send()
            .await?;
        parse_response(resp).await
    }

    /// `POST /admin/disconnect` (`admin::disconnect`): deletes the paired
    /// device row(s) (+ their APNs tokens). `Some(kind)` frees just that
    /// slot -- the two devices carry separate tokens for exactly this
    /// reason, so losing a watch never forces the phone to re-pair. `None`
    /// frees every slot at once (the panel's whole-device "Disconnect"),
    /// sent as the same plain `{}` body this client already sent back when
    /// there was only ever one slot to clear. Always safe to call, even with
    /// nothing paired in the targeted slot(s) (`disconnected: 0`,
    /// `was_paired: false` reports that honestly rather than erroring).
    pub async fn disconnect(
        &self,
        kind: Option<DeviceKind>,
    ) -> Result<DisconnectResponse, RelayAdminError> {
        let body = match kind {
            Some(k) => serde_json::to_vec(&DisconnectRequest { kind: k.as_str() })?,
            None => b"{}".to_vec(),
        };
        let resp = self
            .http
            .post(format!("{}/admin/disconnect", self.base_url))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
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

/// Mirrors `admin::ArmPairingWindowRequest`. `kind` is required on the wire
/// (the relay 400s without it -- `admin.rs`'s own doc explains why a silent
/// default would be dangerous here: a desktop that forgot the field could
/// arm a phone window for a code it minted for the watch), so
/// [`RelayAdminClient::arm_pairing_window`] takes it as a real argument
/// rather than an optional one.
#[derive(Debug, Serialize)]
struct ArmPairingWindowRequest {
    kind: &'static str,
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
/// `None` -- a normal, renderable "nothing paired in this slot yet" shape,
/// not an error.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceView {
    /// `"phone"` or `"watch"` -- present even when `paired` is `false`, so a
    /// caller can tell the two empty-slot rows apart.
    pub kind: String,
    pub paired: bool,
    pub device_id: Option<String>,
    pub name: Option<String>,
    pub platform: Option<String>,
    pub paired_at_unix: Option<i64>,
    pub last_seen_unix: Option<i64>,
}

/// Mirrors `admin::WindowView`: one pairing window currently armed at the
/// relay. No secret here, the code hash never leaves the registry.
///
/// `failed_attempts` (wrong codes presented to `POST /relay/v1/pair` since
/// this window was armed) is PURELY OBSERVATIONAL. The relay never closes a
/// window over it, and deliberately so: that route is pre-auth, so an
/// unauthenticated caller could otherwise deny pairing at will, and the
/// watch's window (the long-lived one, waiting on a WatchConnectivity
/// handoff) would be the easiest thing in the system to keep permanently
/// shut. A caller must render this as something for a human to notice and
/// decide about through this same authenticated API, never as a threat the
/// app is already handling, blocking, or will time out on its own over.
#[derive(Debug, Clone, Deserialize)]
pub struct WindowView {
    /// `"phone"` or `"watch"`.
    pub kind: String,
    pub expires_unix: i64,
    pub failed_attempts: i64,
}

/// Mirrors `admin::DevicesResponse`: always exactly one [`DeviceView`] per
/// [`DeviceKind`], phone then watch, whether or not each slot is filled.
/// Replaces the V1 single-`DeviceView` response now that `GET /admin/device`
/// is `GET /admin/devices`.
#[derive(Debug, Clone, Deserialize)]
pub struct DevicesResponse {
    pub devices: Vec<DeviceView>,
    /// Currently armed windows, if any; empty in the normal steady state.
    /// `#[serde(default)]` so a relay from before this field existed still
    /// deserializes (as no windows reported -- the safe, quiet default,
    /// matching what "nothing armed" already renders as).
    #[serde(default)]
    pub windows: Vec<WindowView>,
}

impl DevicesResponse {
    /// The slot for `kind`, if the relay reported one. Callers ask for "the
    /// phone" and "the watch" individually far more often than they walk
    /// `devices` as a list, so this is the primary way this type gets read.
    /// `None` only if the relay itself omitted a kind it is contractually
    /// supposed to always include (`admin::get_devices`'s own doc) -- a
    /// caller should treat that the same as an unpaired slot, never panic.
    pub fn slot(&self, kind: DeviceKind) -> Option<&DeviceView> {
        self.devices.iter().find(|d| d.kind == kind.as_str())
    }

    /// The currently armed window for `kind`, if any. Mirrors [`Self::slot`]'s
    /// find-by-kind shape; `None` is the normal steady state (nothing armed
    /// for that kind right now), not an error.
    pub fn window(&self, kind: DeviceKind) -> Option<&WindowView> {
        self.windows.iter().find(|w| w.kind == kind.as_str())
    }
}

/// Mirrors `admin::DisconnectRequest`'s one field. Only ever constructed for
/// the `Some(kind)` case: [`RelayAdminClient::disconnect`] sends the plain
/// `{}` body for `None` (revoke everything) instead, matching what this
/// client already sent back when there was no `kind` to name at all.
#[derive(Debug, Serialize)]
struct DisconnectRequest {
    kind: &'static str,
}

/// Mirrors `admin::DisconnectResponse`.
#[derive(Debug, Clone, Deserialize)]
pub struct DisconnectResponse {
    pub ok: bool,
    /// How many device rows were actually removed (0, 1, or 2).
    pub disconnected: usize,
    /// Kept for the panel's existing "was anything there?" phrasing --
    /// equivalent to `disconnected > 0`.
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

    // ---- DeviceKind ---------------------------------------------------------

    #[test]
    fn device_kind_round_trips_and_rejects_anything_else() {
        for kind in DeviceKind::ALL {
            assert_eq!(DeviceKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(DeviceKind::parse("laptop"), None);
        assert_eq!(DeviceKind::parse("Phone"), None, "spelling is exact");
        assert_eq!(DeviceKind::parse(""), None);
    }

    // ---- DTO deserialization/serialization: real shapes, transcribed from
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
    fn arm_pairing_window_request_serializes_the_required_kind_field() {
        let body = serde_json::to_string(&ArmPairingWindowRequest {
            kind: DeviceKind::Watch.as_str(),
            code_sha256: "deadbeef".to_string(),
            ttl_secs: 300,
        })
        .expect("serializes");
        assert_eq!(
            body,
            r#"{"kind":"watch","code_sha256":"deadbeef","ttl_secs":300}"#
        );
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
            r#"{"kind":"phone","paired":true,"device_id":"d1","name":"Yurii's iPhone","platform":"ios",
                 "paired_at_unix":1000,"last_seen_unix":2000}"#,
        )
        .expect("valid paired DeviceView json");
        assert_eq!(paired.kind, "phone");
        assert!(paired.paired);
        assert_eq!(paired.device_id.as_deref(), Some("d1"));

        let unpaired: DeviceView = serde_json::from_str(
            r#"{"kind":"watch","paired":false,"device_id":null,"name":null,"platform":null,
                 "paired_at_unix":null,"last_seen_unix":null}"#,
        )
        .expect("valid unpaired DeviceView json");
        assert_eq!(unpaired.kind, "watch");
        assert!(!unpaired.paired);
        assert!(unpaired.device_id.is_none());
    }

    #[test]
    fn devices_response_slot_finds_each_kind_and_nothing_else() {
        let resp: DevicesResponse = serde_json::from_str(
            r#"{"devices":[
                {"kind":"phone","paired":true,"device_id":"d1","name":"iPhone","platform":"ios",
                 "paired_at_unix":1000,"last_seen_unix":2000},
                {"kind":"watch","paired":false,"device_id":null,"name":null,"platform":null,
                 "paired_at_unix":null,"last_seen_unix":null}
            ]}"#,
        )
        .expect("valid DevicesResponse json");

        let phone = resp.slot(DeviceKind::Phone).expect("phone slot present");
        assert!(phone.paired);
        assert_eq!(phone.device_id.as_deref(), Some("d1"));

        let watch = resp.slot(DeviceKind::Watch).expect("watch slot present");
        assert!(!watch.paired);
    }

    #[test]
    fn devices_response_deserializes_without_a_windows_key_for_an_older_relay() {
        // No `"windows"` key at all -- what a relay from before this field
        // existed sends. `#[serde(default)]` must still produce an empty
        // Vec, not fail the whole response.
        let resp: DevicesResponse = serde_json::from_str(
            r#"{"devices":[
                {"kind":"phone","paired":false,"device_id":null,"name":null,"platform":null,
                 "paired_at_unix":null,"last_seen_unix":null},
                {"kind":"watch","paired":false,"device_id":null,"name":null,"platform":null,
                 "paired_at_unix":null,"last_seen_unix":null}
            ]}"#,
        )
        .expect("valid DevicesResponse json even without a windows key");
        assert!(resp.windows.is_empty());
        assert!(resp.window(DeviceKind::Phone).is_none());
    }

    #[test]
    fn window_view_deserializes() {
        let w: WindowView =
            serde_json::from_str(r#"{"kind":"watch","expires_unix":1758000900,"failed_attempts":3}"#)
                .expect("valid WindowView json");
        assert_eq!(w.kind, "watch");
        assert_eq!(w.expires_unix, 1_758_000_900);
        assert_eq!(w.failed_attempts, 3);
    }

    #[test]
    fn devices_response_window_finds_the_armed_kind_and_nothing_else() {
        let resp: DevicesResponse = serde_json::from_str(
            r#"{"devices":[
                {"kind":"phone","paired":false,"device_id":null,"name":null,"platform":null,
                 "paired_at_unix":null,"last_seen_unix":null},
                {"kind":"watch","paired":false,"device_id":null,"name":null,"platform":null,
                 "paired_at_unix":null,"last_seen_unix":null}
            ],"windows":[
                {"kind":"watch","expires_unix":1758000900,"failed_attempts":7}
            ]}"#,
        )
        .expect("valid DevicesResponse json");

        assert!(
            resp.window(DeviceKind::Phone).is_none(),
            "phone has no armed window"
        );
        let watch_window = resp.window(DeviceKind::Watch).expect("watch window present");
        assert_eq!(watch_window.expires_unix, 1_758_000_900);
        assert_eq!(watch_window.failed_attempts, 7);
    }

    #[test]
    fn devices_response_windows_is_empty_in_the_normal_steady_state() {
        let resp: DevicesResponse = serde_json::from_str(
            r#"{"devices":[
                {"kind":"phone","paired":true,"device_id":"d1","name":"iPhone","platform":"ios",
                 "paired_at_unix":1000,"last_seen_unix":2000},
                {"kind":"watch","paired":true,"device_id":"d2","name":"Watch","platform":"watchos",
                 "paired_at_unix":1000,"last_seen_unix":2000}
            ],"windows":[]}"#,
        )
        .expect("valid DevicesResponse json");
        assert!(resp.windows.is_empty());
    }

    #[test]
    fn disconnect_request_serializes_the_named_kind() {
        let body = serde_json::to_string(&DisconnectRequest {
            kind: DeviceKind::Phone.as_str(),
        })
        .expect("serializes");
        assert_eq!(body, r#"{"kind":"phone"}"#);
    }

    #[test]
    fn disconnect_response_deserializes() {
        let resp: DisconnectResponse =
            serde_json::from_str(r#"{"ok":true,"disconnected":2,"was_paired":true}"#)
                .expect("valid DisconnectResponse json");
        assert!(resp.ok);
        assert_eq!(resp.disconnected, 2);
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
        match client
            .arm_pairing_window(DeviceKind::Phone, "deadbeef", 300)
            .await
        {
            Err(RelayAdminError::Transport(_)) => {}
            other => panic!("expected Transport against a closed port, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn devices_against_no_live_relay_is_a_transport_error() {
        let client = RelayAdminClient::new("http://127.0.0.1:1").expect("client");
        match client.devices().await {
            Err(RelayAdminError::Transport(_)) => {}
            other => panic!("expected Transport against a closed port, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn disconnect_against_no_live_relay_is_a_transport_error() {
        let client = RelayAdminClient::new("http://127.0.0.1:1").expect("client");
        match client.disconnect(None).await {
            Err(RelayAdminError::Transport(_)) => {}
            other => panic!("expected Transport against a closed port, got {other:?}"),
        }
    }
}
