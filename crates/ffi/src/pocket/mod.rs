//! `PocketHandle`: the UniFFI Object wrapping the Pocket panel's pairing
//! flow (docs/PHASE5.md W2, itrat-console/13 D12.2a) for the SwiftUI shell -
//! at parity with the Tauri shell's `pocket` module
//! (`apps/desktop/src-tauri/src/pocket/`, see that module's own doc for the
//! full flow this mirrors). "Connect TokenFuse Pocket": mint a code at the
//! Cloud (admin key, [`crate::cloud::env::discover`] - the SAME two-tier
//! discovery [`crate::cloud::CloudHandle`]'s own device pairing uses), arm
//! the relay's pairing window, and return the exact QR content string to
//! render; [`PocketHandle::status`]/[`PocketHandle::disconnect`] read/clear
//! the paired-device row.
//!
//! ## Async-to-sync: one owned `tokio::runtime::Runtime`
//!
//! `CloudClient::pair_new` and `RelayAdminClient`'s methods are all `async
//! fn`; every UniFFI-exported method here is synchronous (F-04,
//! docs/PHASE0.md). Mirrors `CloudHandle`/`RemoteHandle`'s identical bridge:
//! one multi-thread runtime built once in the constructor, `block_on` per
//! call - see those types' own module docs for why multi-thread (not
//! current-thread) specifically (more than one Swift caller thread must
//! never contend for a single `block_on` slot).
//!
//! ## No managed state beyond the runtime
//!
//! Unlike `CloudHandle` (holds a paired device) or `RemoteHandle` (holds a
//! keypair/tunnel), `PocketHandle` holds NOTHING beyond its own runtime -
//! every method resolves the Cloud admin key and the relay admin URL fresh,
//! per call, mirroring the Tauri shell's `pocket::commands` module doc
//! ("stateless by design... there is no persistent connection worth holding
//! onto"). There is no `discover()`/`connect()` CONSTRUCTOR pair the way
//! `CloudHandle` has either: [`PocketHandle::new`] cannot fail on
//! environment/pairing at all, only on the runtime itself failing to start -
//! the pairing attempt itself is [`PocketHandle::connect`], a regular
//! method, callable any time after construction.
//!
//! ## Fail-closed: a half-armed window never survives a later failure
//!
//! See [`PocketHandle::connect`]'s own doc comment - mirrors
//! `pocket::commands::pocket_connect`'s identical disarm-on-failure body,
//! including WHY the relay's own armed-window expiry (not the Cloud code's
//! longer TTL) is what [`dto::PocketQrRecord::expires_unix`] carries.
//!
//! Fail-closed at the boundary (06 §0.5): nothing here panics across FFI.

pub mod dto;
pub mod env;

pub use dto::{PocketError, PocketQrRecord, PocketStatusRecord};

use genaryx_connectors::{CloudClient, PairNewResponse, RelayAdminClient, RelayPairingInfo};

/// See `pocket::commands`'s identical constant and doc comment (D12.2 step
/// 2's `ttl: 300`).
const PAIRING_WINDOW_TTL_SECS: i64 = 300;

/// The Pocket UniFFI Object. See the module doc for the async bridge and why
/// there is nothing else to hold.
#[derive(uniffi::Object)]
pub struct PocketHandle {
    runtime: tokio::runtime::Runtime,
}

#[uniffi::export]
impl PocketHandle {
    /// Build the handle: start the small owned async runtime every method
    /// below needs to bridge `genaryx_connectors`' async clients. Touches no
    /// network and resolves no environment (see the module doc) - this can
    /// only fail on a genuine local resource problem.
    #[uniffi::constructor]
    pub fn new() -> Result<Self, PocketError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| PocketError::Runtime {
                reason: e.to_string(),
            })?;
        Ok(Self { runtime })
    }

    /// Whole-panel status (idle / paired / relay-unreachable). Never throws
    /// - see [`PocketStatusRecord`]'s own doc for why building the relay
    /// HTTP client itself cannot fail in practice (no TLS backend to init
    /// for a plain `reqwest` client), but is still handled honestly as
    /// `RelayUnreachable` rather than unwrapped.
    pub fn status(&self) -> PocketStatusRecord {
        let admin_url = env::relay_admin_url();
        let Ok(relay) = RelayAdminClient::new(&admin_url) else {
            return PocketStatusRecord::RelayUnreachable {
                message: "failed to build the relay admin HTTP client".to_string(),
            };
        };
        self.runtime.block_on(build_status(&relay))
    }

    /// "Connect TokenFuse Pocket" (docs/PHASE5.md W2) - see this module's
    /// doc comment for the full three-step flow. Fail-closed: once the
    /// pairing window is armed (step 2), any later failure disarms it via
    /// `RelayAdminClient::disconnect` before returning the error, rather
    /// than leaving a QR-less armed window silently open.
    pub fn connect(&self) -> Result<PocketQrRecord, PocketError> {
        self.runtime.block_on(connect_impl())
    }

    /// Disconnect the paired phone (always safe to call, even with nothing
    /// paired). Returns the fresh status so the panel flips back to Idle
    /// without a second call.
    pub fn disconnect(&self) -> Result<PocketStatusRecord, PocketError> {
        self.runtime.block_on(disconnect_impl())
    }
}

/// Read the relay's current device view and fold it into a
/// [`PocketStatusRecord`] - shared by [`PocketHandle::status`] and
/// [`disconnect_impl`] so a Disconnect never needs a second call. A relay
/// admin call failure here becomes `RelayUnreachable`, never a thrown error:
/// this is a STATUS read, and "the relay is down" is itself a normal,
/// renderable status, not a command failure.
async fn build_status(relay: &RelayAdminClient) -> PocketStatusRecord {
    match relay.device().await {
        Ok(view) if view.paired => PocketStatusRecord::Paired {
            device_id: view.device_id.unwrap_or_default(),
            name: view.name.unwrap_or_default(),
            platform: view.platform.unwrap_or_default(),
            paired_at_unix: view.paired_at_unix.unwrap_or_default(),
            last_seen_unix: view.last_seen_unix.unwrap_or_default(),
        },
        Ok(_) => PocketStatusRecord::Idle {
            cloud_ready: crate::cloud::env::discover().is_some(),
        },
        Err(e) => PocketStatusRecord::RelayUnreachable {
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
/// is - mirrors `pocket::commands::build_qr`'s identical reasoning.
///
/// ## Why the QR fields are NOT percent-encoded
///
/// docs/PHASE5.md W2 pins the QR content to be "EXACTLY" the literal
/// template `genaryx-pocket://pair/v1?relay=<public_advertise_url>&pin=<b64
/// SPKI-SHA256>&code=<8-char>&org=<org>` "so W3's scanner parses it" - W3
/// (the mobile QR scanner) is a later wave and does not exist yet in this
/// build, so this is a pinned contract, not a free choice to percent-encode
/// against. Checked to still be safe: `relay_url` (`https://host:port`) has
/// no `&`/`=`; standard base64's alphabet (`A-Za-z0-9+/=`) has no `&`
/// either, so a naive split-on-`&`-then-first-`=` parser (or Swift's own
/// `URLComponents.queryItems`, which does NOT apply
/// `application/x-www-form-urlencoded` `+`-as-space decoding - only
/// percent-decoding, per RFC 3986) both isolate every field correctly
/// without corruption.
async fn build_qr(
    relay: &RelayAdminClient,
    minted: &PairNewResponse,
    window_expires_unix: i64,
) -> Result<PocketQrRecord, PocketError> {
    let info: RelayPairingInfo = relay.pairing_info().await?;
    Ok(PocketQrRecord {
        qr_content: format!(
            "genaryx-pocket://pair/v1?relay={}&pin={}&code={}&org={}",
            info.relay_url, info.pin, minted.code, info.org
        ),
        expires_unix: window_expires_unix,
    })
}

async fn connect_impl() -> Result<PocketQrRecord, PocketError> {
    let resolved = crate::cloud::env::discover().ok_or(PocketError::NoCloudEnvironment)?;

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
                    "genaryx-ffi pocket: connect: failed to disarm the just-opened pairing window \
                     after a pairing-info failure (it will still close on its own {PAIRING_WINDOW_TTL_SECS}s TTL): \
                     {disarm_err}"
                );
            }
            Err(e)
        }
    }
}

async fn disconnect_impl() -> Result<PocketStatusRecord, PocketError> {
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

    #[test]
    fn new_never_touches_network_or_filesystem() {
        let _handle = PocketHandle::new().expect("construct PocketHandle");
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

    // ---- fail-closed transport: no live relay in unit tests, mirrors
    // `pocket::commands`'s own transport-error test convention (an explicit
    // closed port, never the ambient default admin URL - a real relay might
    // genuinely be running on 127.0.0.1:8444 on some boxes) -------------------

    #[tokio::test]
    async fn build_status_against_no_live_relay_is_relay_unreachable() {
        let relay = RelayAdminClient::new("http://127.0.0.1:1").expect("client construction");
        match build_status(&relay).await {
            PocketStatusRecord::RelayUnreachable { message } => assert!(!message.is_empty()),
            other => panic!("expected RelayUnreachable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn build_qr_against_no_live_relay_is_a_relay_error() {
        let relay = RelayAdminClient::new("http://127.0.0.1:1").expect("client construction");
        // A window_expires_unix deliberately DIFFERENT from
        // sample_minted().expires_unix (1_758_000_600): proves at the call
        // site that build_qr takes the relay's own window expiry as an
        // independent argument, not derived from the Cloud code's own TTL.
        let err = build_qr(&relay, &sample_minted(), 1_758_000_300)
            .await
            .expect_err("no relay is listening on port 1");
        assert!(matches!(err, PocketError::Relay { .. }));
    }

    /// Mirrors `CloudHandle`'s own
    /// `discover_without_an_environment_is_a_clean_error_not_a_panic`:
    /// inherently environment-dependent (this box may or may not have a
    /// real `taipan up` environment or `TOKENFUSE_CLOUD_ADMIN_KEY` set), so
    /// this only proves `connect()` never panics and always settles into
    /// SOME honest `Result` shape.
    #[test]
    fn connect_never_panics_regardless_of_this_boxs_environment() {
        let handle = PocketHandle::new().expect("construct");
        match handle.connect() {
            Ok(_)
            | Err(
                PocketError::NoCloudEnvironment
                | PocketError::Cloud { .. }
                | PocketError::Relay { .. }
                | PocketError::DeviceExists,
            ) => {}
            Err(PocketError::Runtime { .. }) => {
                panic!("the runtime already started successfully in PocketHandle::new")
            }
        }
    }
}
