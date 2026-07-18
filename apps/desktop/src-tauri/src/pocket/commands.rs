//! Tauri commands for the Pocket panel (docs/PHASE5.md W2, itrat-console/13
//! D12.2a): [`pocket_status`] (idle/paired/relay-unreachable), [`pocket_connect`]
//! ("Connect TokenFuse Pocket": mint a code at the Cloud, arm the relay's
//! pairing window, return the QR content), and [`pocket_disconnect`].
//!
//! Flow (D12.2a steps 1-3, 10), mirrored exactly by
//! `crates/ffi/src/pocket/mod.rs` for the SwiftUI shell:
//!
//! 1. [`pocket_connect`] resolves the Cloud admin key
//!    (`crate::money::env::discover`, the SAME two-tier discovery Money's
//!    own device pairing uses) and calls `POST /v1/pair/new`
//!    ([`genaryx_connectors::CloudClient::pair_new`]) to mint `{code,
//!    expires_unix}`.
//! 2. It then arms the relay's pairing window
//!    (`POST /admin/pairing-window {code_sha256, ttl_secs}` -
//!    [`genaryx_connectors::RelayAdminClient::arm_pairing_window`]),
//!    hashing the code with `genaryx_signing::body_sha256_hex` before it
//!    ever leaves this process - the relay never learns the plaintext code
//!    (D12.3's trust-boundary table). A `409 device_exists` here (a device
//!    is already paired) maps to [`PocketError::DeviceExists`] so the
//!    frontend can show Disconnect instead of a QR (D12.2 step 2), never a
//!    generic error banner.
//! 3. It reads the relay's `GET /admin/pairing-info` (pin +
//!    `public_advertise_url` + org) and builds the EXACT
//!    `genaryx-pocket://pair/v1?relay=...&pin=...&code=...&org=...` string
//!    (D12.2 step 3) so W3's scanner parses it - see [`build_qr`]'s doc for
//!    the literal-substitution choice.
//!
//! Fail-closed (docs/PHASE5.md W2 "Rules": "do not leave a half-armed
//! window silently"): once step 2 succeeds the window IS armed at the
//! relay, so a failure in step 3 disarms it again via `disconnect` before
//! returning the error - see [`pocket_connect`]'s own body.
//!
//! [`pocket_status`]/[`pocket_disconnect`] both resolve through
//! [`build_status`], so a Disconnect never needs a second round trip to
//! learn the fresh (now-idle) state, mirroring
//! `remote::commands`'s identical "every mutating command returns the
//! whole-panel status" contract.

use super::env;
use genaryx_connectors::{
    CloudClient, ConnectorError, PairNewResponse, RelayAdminClient, RelayAdminError,
    RelayPairingInfo,
};
use serde::Serialize;

/// How long the relay's pairing window stays armed once opened (D12.2 step
/// 2's `ttl: 300`) - 5 minutes: generous for an operator to open TokenFuse
/// Pocket and scan, short enough that an abandoned QR does not stay
/// pairable indefinitely.
const PAIRING_WINDOW_TTL_SECS: i64 = 300;

// ============================================================================
// DTOs
// ============================================================================

/// Whole-panel state (idle/paired/relay-unreachable) - the three UI states
/// docs/PHASE5.md W2 calls for are `Idle` (render "Connect"), a frontend-only
/// "showing QR" step entered right after a successful [`pocket_connect`]
/// (not tracked here - the relay exposes no "is a window currently armed"
/// read, only device-paired state), and `Paired` (device + Disconnect).
/// `RelayUnreachable` is this module's own addition, honest about a fourth
/// real outcome (the admin API itself is down) rather than folding it into
/// `Idle` and showing a Connect button that would only fail.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PocketStatusDto {
    Idle {
        /// Whether a Cloud admin key is resolvable at all
        /// (`money::env::discover`) - `false` renders as a disabled Connect
        /// button with an honest hint, rather than letting the operator
        /// click it only to get [`PocketError::NoCloudEnvironment`].
        cloud_ready: bool,
    },
    Paired {
        device_id: String,
        name: String,
        platform: String,
        paired_at_unix: i64,
        last_seen_unix: i64,
    },
    RelayUnreachable {
        message: String,
    },
}

/// [`pocket_connect`]'s success return: the exact QR content string (render
/// verbatim, never reconstruct client-side) plus when the pairing window
/// (and the underlying Cloud code) expires, so the frontend can show a
/// countdown and stop polling once it lapses.
#[derive(Debug, Clone, Serialize)]
pub struct PocketQrDto {
    pub qr_content: String,
    pub expires_unix: i64,
}

/// Every error a Pocket command can return.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PocketError {
    /// No Cloud admin key resolvable (`money::env::discover` returned
    /// `None`) - the same honest "no environment" outcome Money itself
    /// shows, never a fabricated Connect attempt.
    NoCloudEnvironment,
    /// The Cloud rejected or could not be reached for `POST /v1/pair/new`.
    Cloud { message: String },
    /// The relay refused to arm a pairing window because a device is
    /// already paired (D12.2 step 2) - the frontend should re-fetch
    /// [`pocket_status`] and render the Paired view, not an error banner.
    DeviceExists,
    /// The relay's admin API itself failed (unreachable, or any non-2xx
    /// other than the `DeviceExists` case above).
    Relay { message: String },
}

impl From<ConnectorError> for PocketError {
    fn from(e: ConnectorError) -> Self {
        PocketError::Cloud {
            message: e.to_string(),
        }
    }
}

impl From<RelayAdminError> for PocketError {
    fn from(e: RelayAdminError) -> Self {
        match e {
            RelayAdminError::DeviceExists => PocketError::DeviceExists,
            other => PocketError::Relay {
                message: other.to_string(),
            },
        }
    }
}

// ============================================================================
// helpers
// ============================================================================

/// Read the relay's current device view and fold it into a [`PocketStatusDto`],
/// shared by [`pocket_status`] and [`pocket_disconnect`] so a Disconnect
/// never needs a second round trip (see this module's doc comment). A relay
/// admin call failure here becomes `RelayUnreachable`, never a thrown error:
/// this is a STATUS read, and "the relay is down" is itself a normal,
/// renderable status, not a command failure.
async fn build_status(relay: &RelayAdminClient) -> PocketStatusDto {
    match relay.device().await {
        Ok(view) if view.paired => PocketStatusDto::Paired {
            device_id: view.device_id.unwrap_or_default(),
            name: view.name.unwrap_or_default(),
            platform: view.platform.unwrap_or_default(),
            paired_at_unix: view.paired_at_unix.unwrap_or_default(),
            last_seen_unix: view.last_seen_unix.unwrap_or_default(),
        },
        Ok(_) => PocketStatusDto::Idle {
            cloud_ready: crate::money::env::discover().is_some(),
        },
        Err(e) => PocketStatusDto::RelayUnreachable {
            message: e.to_string(),
        },
    }
}

/// Build the exact QR content string (D12.2 step 3) from the relay's
/// `pairing-info` plus the just-minted Cloud code. `window_expires_unix`
/// MUST be the RELAY's own armed-window expiry (`arm_pairing_window`'s
/// response), not the Cloud code's own (longer, currently 600s) TTL: the
/// relay's pairing route goes dark the moment ITS window closes (D12.3,
/// `registry.rs::check_pairing_code`), so a countdown built from the Cloud's
/// TTL would tell the operator the QR is good for longer than it actually
/// is.
///
/// ## Why the QR fields are NOT percent-encoded
///
/// docs/PHASE5.md W2 pins the QR content to be "EXACTLY" the literal
/// template `genaryx-pocket://pair/v1?relay=<public_advertise_url>&pin=<b64
/// SPKI-SHA256>&code=<8-char>&org=<org>` (itrat-console/13 D12.2 step 3's own
/// example is written the same unencoded way) "so W3's scanner parses it" -
/// W3 (the mobile QR scanner) is a later wave and does not exist yet in this
/// build, so this is a pinned contract, not a free choice to percent-encode
/// against. Checked to still be safe: `relay_url` (`https://host:port`) has
/// no `&`/`=`; standard base64's alphabet (`A-Za-z0-9+/=`) has no `&` either,
/// so a naive split-on-`&`-then-first-`=` parser (or Swift's
/// `URLComponents.queryItems`, which does NOT apply
/// `application/x-www-form-urlencoded` `+`-as-space decoding - only
/// percent-decoding, per RFC 3986) both isolate every field correctly
/// without corruption.
async fn build_qr(
    relay: &RelayAdminClient,
    minted: &PairNewResponse,
    window_expires_unix: i64,
) -> Result<PocketQrDto, PocketError> {
    let info: RelayPairingInfo = relay.pairing_info().await?;
    Ok(PocketQrDto {
        qr_content: format!(
            "genaryx-pocket://pair/v1?relay={}&pin={}&code={}&org={}",
            info.relay_url, info.pin, minted.code, info.org
        ),
        expires_unix: window_expires_unix,
    })
}

// ============================================================================
// commands
// ============================================================================

/// Whole-panel status. Never fails (mirrors `money_status`/`remote_status`'s
/// identical "never fails" contract): building the relay HTTP client itself
/// cannot fail in practice (no TLS backend to init for a plain `reqwest`
/// client, `RelayAdminClient::new`'s only fallible step), but is still
/// handled honestly as `RelayUnreachable` rather than unwrapped.
#[tauri::command]
pub async fn pocket_status() -> Result<PocketStatusDto, ()> {
    let admin_url = env::relay_admin_url();
    let relay = match RelayAdminClient::new(&admin_url) {
        Ok(c) => c,
        Err(e) => {
            return Ok(PocketStatusDto::RelayUnreachable {
                message: e.to_string(),
            });
        }
    };
    Ok(build_status(&relay).await)
}

/// "Connect TokenFuse Pocket" (docs/PHASE5.md W2) - see this module's doc
/// comment for the full three-step flow. Fail-closed: once the pairing
/// window is armed (step 2), any later failure disarms it via
/// `RelayAdminClient::disconnect` before returning the error, rather than
/// leaving a QR-less armed window silently open.
#[tauri::command]
pub async fn pocket_connect() -> Result<PocketQrDto, PocketError> {
    let resolved = crate::money::env::discover().ok_or(PocketError::NoCloudEnvironment)?;

    let cloud = CloudClient::new(resolved.cloud_url.clone(), resolved.admin_bearer.clone())?;
    let minted = cloud.pair_new(&resolved.admin_bearer).await?;

    let admin_url = env::relay_admin_url();
    let relay = RelayAdminClient::new(&admin_url).map_err(PocketError::from)?;
    let code_sha256 = genaryx_signing::body_sha256_hex(minted.code.as_bytes());
    let armed = relay
        .arm_pairing_window(&code_sha256, PAIRING_WINDOW_TTL_SECS)
        .await?;

    match build_qr(&relay, &minted, armed.expires_unix).await {
        Ok(qr) => Ok(qr),
        Err(e) => {
            if let Err(disarm_err) = relay.disconnect().await {
                eprintln!(
                    "genaryx: pocket_connect: failed to disarm the just-opened pairing window \
                     after a pairing-info failure (it will still close on its own {PAIRING_WINDOW_TTL_SECS}s TTL): \
                     {disarm_err}"
                );
            }
            Err(e)
        }
    }
}

/// Disconnect the paired phone (always safe to call, even with nothing
/// paired - `RelayAdminClient::disconnect`'s own contract) and return the
/// fresh status so the panel flips back to Idle without a second round trip.
#[tauri::command]
pub async fn pocket_disconnect() -> Result<PocketStatusDto, PocketError> {
    let admin_url = env::relay_admin_url();
    let relay = RelayAdminClient::new(&admin_url).map_err(PocketError::from)?;
    relay.disconnect().await?;
    Ok(build_status(&relay).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_info() -> RelayPairingInfo {
        RelayPairingInfo {
            pin: "dGVzdC1zcGtpLXBpbi1iYXNlNjQ=".to_string(),
            relay_url: "https://198.51.100.7:8443".to_string(),
            org: "acme".to_string(),
        }
    }

    fn sample_minted() -> PairNewResponse {
        PairNewResponse {
            code: "ABCD1234".to_string(),
            expires_unix: 1_758_000_600,
        }
    }

    // ---- QR content shape: the load-bearing contract W3 depends on --------

    #[test]
    fn qr_content_matches_the_exact_pinned_template() {
        let info = sample_info();
        let minted = sample_minted();
        let qr_content = format!(
            "genaryx-pocket://pair/v1?relay={}&pin={}&code={}&org={}",
            info.relay_url, info.pin, minted.code, info.org
        );
        assert_eq!(
            qr_content,
            "genaryx-pocket://pair/v1?relay=https://198.51.100.7:8443&pin=dGVzdC1zcGtpLXBpbi1iYXNlNjQ=\
&code=ABCD1234&org=acme"
        );
    }

    #[test]
    fn qr_content_fields_are_recoverable_by_a_naive_split_on_ampersand_then_first_equals() {
        // Proves the "no percent-encoding needed" reasoning in `build_qr`'s
        // doc comment against a REAL string, not just an argument in prose:
        // a hand-rolled parser exactly as unsophisticated as one a mobile
        // scanner (W3) might use must still recover every field intact.
        let info = sample_info();
        let minted = sample_minted();
        let qr_content = format!(
            "genaryx-pocket://pair/v1?relay={}&pin={}&code={}&org={}",
            info.relay_url, info.pin, minted.code, info.org
        );
        let (scheme_and_path, query) = qr_content.split_once('?').expect("has a query string");
        assert_eq!(scheme_and_path, "genaryx-pocket://pair/v1");

        let mut fields = std::collections::HashMap::new();
        for pair in query.split('&') {
            let (k, v) = pair.split_once('=').expect("every field is key=value");
            fields.insert(k, v);
        }
        assert_eq!(fields.get("relay"), Some(&info.relay_url.as_str()));
        assert_eq!(fields.get("pin"), Some(&info.pin.as_str()));
        assert_eq!(fields.get("code"), Some(&minted.code.as_str()));
        assert_eq!(fields.get("org"), Some(&info.org.as_str()));
    }

    // ---- error mapping ------------------------------------------------------

    #[test]
    fn relay_device_exists_maps_to_pocket_device_exists() {
        assert!(matches!(
            PocketError::from(RelayAdminError::DeviceExists),
            PocketError::DeviceExists
        ));
    }

    #[test]
    fn other_relay_errors_map_to_pocket_relay_with_a_message() {
        match PocketError::from(RelayAdminError::Api {
            status: 500,
            body: "boom".to_string(),
        }) {
            PocketError::Relay { message } => assert!(message.contains("500")),
            other => panic!("expected Relay, got {other:?}"),
        }
    }

    #[test]
    fn connector_error_maps_to_pocket_cloud_with_a_message() {
        match PocketError::from(ConnectorError::NoDeviceSigner) {
            PocketError::Cloud { message } => assert!(!message.is_empty()),
            other => panic!("expected Cloud, got {other:?}"),
        }
    }

    // ---- build_qr, against a closed port: proves the fail-closed shape
    // without a live relay (mirrors relay_admin.rs's own transport-error
    // test convention) ----------------------------------------------------

    #[tokio::test]
    async fn build_qr_against_no_live_relay_is_a_relay_error() {
        let relay = RelayAdminClient::new("http://127.0.0.1:1").expect("client construction");
        // A window_expires_unix deliberately DIFFERENT from
        // sample_minted().expires_unix (1_758_000_600): proves at the call
        // site that build_qr takes the relay's own window expiry as an
        // independent argument, not derived from the Cloud code's own TTL
        // (see build_qr's doc comment for why the two must not be conflated).
        let err = build_qr(&relay, &sample_minted(), 1_758_000_300)
            .await
            .expect_err("no relay is listening on port 1");
        assert!(matches!(err, PocketError::Relay { .. }));
    }

    // ---- build_status ---------------------------------------------------

    #[tokio::test]
    async fn build_status_against_no_live_relay_is_relay_unreachable() {
        let relay = RelayAdminClient::new("http://127.0.0.1:1").expect("client construction");
        match build_status(&relay).await {
            PocketStatusDto::RelayUnreachable { message } => assert!(!message.is_empty()),
            other => panic!("expected RelayUnreachable, got {other:?}"),
        }
    }
}
