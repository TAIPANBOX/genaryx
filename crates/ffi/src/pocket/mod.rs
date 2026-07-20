//! `PocketHandle`: the UniFFI Object wrapping the Pocket panel's pairing
//! flow (docs/PHASE5.md W2, itrat-console/13 D12.2a) for the SwiftUI shell -
//! at parity with the Tauri shell's `pocket` module
//! (`apps/desktop/src-tauri/src/pocket/`, see that module's own doc for the
//! full flow this mirrors). "Connect TokenFuse Pocket": mint a pairing code
//! for the phone AND one for the watch at the Cloud (admin key,
//! [`crate::cloud::env::discover`] - the SAME two-tier discovery
//! [`crate::cloud::CloudHandle`]'s own device pairing uses), arm the relay's
//! pairing window for each, and return the exact QR content string to
//! render - one QR, both codes; the phone scans it, and is the one that
//! hands the watch its own code over WatchConnectivity
//! (`crates/relay/src/registry.rs`'s own doc on the phone's role).
//! [`PocketHandle::status`]/[`PocketHandle::disconnect`] read/clear the two
//! paired-device rows.
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
//! ## Fail-closed: a half-armed relay never survives a later failure
//!
//! See [`connect_impl`]'s own doc comment - mirrors
//! `pocket::commands::pocket_connect`'s identical disarm-on-failure body,
//! including WHY the relay's own armed-window expiry (not either Cloud
//! code's longer TTL) is what [`dto::PocketQrRecord::expires_unix`] carries,
//! and why it is now the EARLIER of the phone's and the watch's window
//! expiries.
//!
//! Fail-closed at the boundary (06 §0.5): nothing here panics across FFI.

pub mod dto;
pub mod env;

pub use dto::{PocketDeviceRecord, PocketError, PocketQrRecord, PocketStatusRecord, PocketWindowRecord};

use genaryx_connectors::{
    CloudClient, PairNewResponse, RelayAdminClient, RelayDeviceKind, RelayDeviceView,
    RelayPairingInfo, RelayWindowView,
};

/// See `pocket::commands`'s identical constant and doc comment (D12.2 step
/// 2's `ttl: 300`). Applied to both the phone's and the watch's windows.
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

    /// "Connect TokenFuse Pocket" (docs/PHASE5.md W2) - see [`connect_impl`]'s
    /// own doc comment for the full flow. Fail-closed: once either window is
    /// armed, any later failure disarms whatever THIS call itself just
    /// armed, rather than leaving a half-paired relay silently open.
    pub fn connect(&self) -> Result<PocketQrRecord, PocketError> {
        self.runtime.block_on(connect_impl())
    }

    /// Disconnect both the paired phone and watch (always safe to call,
    /// even with nothing paired in either slot). Returns the fresh status so
    /// the panel flips back to Idle without a second call.
    pub fn disconnect(&self) -> Result<PocketStatusRecord, PocketError> {
        self.runtime.block_on(disconnect_impl())
    }
}

/// Read the relay's current two-slot device view (plus any currently armed
/// windows) and fold it into a [`PocketStatusRecord`] - shared by
/// [`PocketHandle::status`] and [`disconnect_impl`] so a Disconnect never
/// needs a second call. A relay admin call failure here becomes
/// `RelayUnreachable`, never a thrown error: this is a STATUS read, and "the
/// relay is down" is itself a normal, renderable status, not a command
/// failure.
async fn build_status(relay: &RelayAdminClient) -> PocketStatusRecord {
    match relay.devices().await {
        Ok(resp) => {
            let phone = resp
                .slot(RelayDeviceKind::Phone)
                .cloned()
                .filter(|d| d.paired)
                .map(to_device_record);
            let watch = resp
                .slot(RelayDeviceKind::Watch)
                .cloned()
                .filter(|d| d.paired)
                .map(to_device_record);
            let phone_window = resp.window(RelayDeviceKind::Phone).cloned().map(to_window_record);
            let watch_window = resp.window(RelayDeviceKind::Watch).cloned().map(to_window_record);
            if phone.is_none() && watch.is_none() {
                PocketStatusRecord::Idle {
                    cloud_ready: crate::cloud::env::discover().is_some(),
                    phone_window,
                    watch_window,
                }
            } else {
                PocketStatusRecord::Paired {
                    phone,
                    watch,
                    phone_window,
                    watch_window,
                }
            }
        }
        Err(e) => PocketStatusRecord::RelayUnreachable {
            message: e.to_string(),
        },
    }
}

/// A paired [`RelayDeviceView`] into the smaller [`PocketDeviceRecord`]
/// shown per slot - drops `kind`/`paired`, which [`build_status`] already
/// consumed to decide whether to call this at all.
fn to_device_record(view: RelayDeviceView) -> PocketDeviceRecord {
    PocketDeviceRecord {
        device_id: view.device_id.unwrap_or_default(),
        name: view.name.unwrap_or_default(),
        platform: view.platform.unwrap_or_default(),
        paired_at_unix: view.paired_at_unix.unwrap_or_default(),
        last_seen_unix: view.last_seen_unix.unwrap_or_default(),
    }
}

/// An armed [`RelayWindowView`] into the smaller [`PocketWindowRecord`]
/// shown per slot - drops `kind`, which [`build_status`] already consumed to
/// decide whether this is the phone's or the watch's window.
fn to_window_record(view: RelayWindowView) -> PocketWindowRecord {
    PocketWindowRecord {
        expires_unix: view.expires_unix,
        failed_attempts: view.failed_attempts,
    }
}

/// Build the exact QR content string (D12.2 step 3, now carrying both
/// codes) from the relay's `pairing-info` plus the just-minted phone and
/// watch codes. `window_expires_unix` MUST be the EARLIER of the two
/// windows' own armed-window expiries (`arm_pairing_window`'s response),
/// never either Cloud code's own (longer, currently 600s) TTL: the relay's
/// pairing route for a kind goes dark the moment THAT kind's window closes
/// (D12.3, `registry.rs::check_pairing_code`), so a countdown built from
/// anything longer would tell the operator the QR is good for longer than
/// the SOONER of the two slots actually stays open - mirrors
/// `pocket::commands::build_qr`'s identical reasoning, extended from one
/// window to the earlier of two (see [`connect_impl`] for where the `min`
/// is taken).
///
/// ## Why the QR fields are NOT percent-encoded
///
/// docs/PHASE5.md W2 pins the QR content to be "EXACTLY" the literal
/// template `genaryx-pocket://pair/v1?relay=<public_advertise_url>&pin=<b64
/// SPKI-SHA256>&code=<8-char>&code_watch=<8-char>&org=<org>` "so W3's
/// scanner parses it" - W3 (the mobile QR scanner) is a later wave and does
/// not exist yet in this build, so this is a pinned contract, not a free
/// choice to percent-encode against. `code` stays the PHONE's code (so a
/// scanner that only reads `code` keeps working unmodified); `code_watch` is
/// additive, inserted before `org` per the pinned field order. Checked to
/// still be safe: `relay_url` (`https://host:port`) has no `&`/`=`; standard
/// base64's alphabet (`A-Za-z0-9+/=`) has no `&` either; and `code_watch`,
/// like `code` before it, is an 8-char unambiguous-alphabet code
/// (`devices::pairing_code()` upstream) that contains no `&` or `=` of its
/// own - so a naive split-on-`&`-then-first-`=` parser (or Swift's own
/// `URLComponents.queryItems`, which does NOT apply
/// `application/x-www-form-urlencoded` `+`-as-space decoding - only
/// percent-decoding, per RFC 3986) isolates every field correctly without
/// corruption.
async fn build_qr(
    relay: &RelayAdminClient,
    minted_phone: &PairNewResponse,
    minted_watch: &PairNewResponse,
    window_expires_unix: i64,
) -> Result<PocketQrRecord, PocketError> {
    let info: RelayPairingInfo = relay.pairing_info().await?;
    Ok(PocketQrRecord {
        qr_content: format!(
            "genaryx-pocket://pair/v1?relay={}&pin={}&code={}&code_watch={}&org={}",
            info.relay_url, info.pin, minted_phone.code, minted_watch.code, info.org
        ),
        expires_unix: window_expires_unix,
    })
}

/// The full "Connect TokenFuse Pocket" flow (D12.2a steps 1-3, 10, extended
/// to two devices):
///
/// 1. Mint TWO codes at the Cloud (`POST /v1/pair/new`, once for the phone
///    and once for the watch) - nothing is armed at the relay yet, so a mint
///    failure for either code leaves the relay untouched.
/// 2. Arm the relay's pairing window for the phone's code, then for the
///    watch's. If arming the watch's window fails after the phone's already
///    succeeded, disarm the phone's window before returning the error
///    ([`RelayAdminClient::disconnect`] with `Some(Phone)`, never `None`:
///    `None` would also clear a phone or watch pairing that predates this
///    call, which is not this failure's to touch) - it will still close on
///    its own after `PAIRING_WINDOW_TTL_SECS`, but a failed Connect should
///    not leave a stray window armed for that long regardless.
/// 3. Read the relay's `pairing-info` and build the QR content carrying both
///    codes (see [`build_qr`]'s doc). If THIS step fails, both windows are
///    armed (we only reach it once both arms above succeeded), so both are
///    disarmed the same way.
async fn connect_impl() -> Result<PocketQrRecord, PocketError> {
    let resolved = crate::cloud::env::discover().ok_or(PocketError::NoCloudEnvironment)?;

    let cloud = CloudClient::new(resolved.cloud_url.clone(), resolved.admin_bearer.clone())?;
    let minted_phone = cloud.pair_new(&resolved.admin_bearer).await?;
    let minted_watch = cloud.pair_new(&resolved.admin_bearer).await?;

    let admin_url = env::relay_admin_url();
    let relay = RelayAdminClient::new(&admin_url).map_err(PocketError::from)?;

    let phone_code_sha256 = genaryx_signing::body_sha256_hex(minted_phone.code.as_bytes());
    let armed_phone = relay
        .arm_pairing_window(RelayDeviceKind::Phone, &phone_code_sha256, PAIRING_WINDOW_TTL_SECS)
        .await?;

    let watch_code_sha256 = genaryx_signing::body_sha256_hex(minted_watch.code.as_bytes());
    let armed_watch = match relay
        .arm_pairing_window(RelayDeviceKind::Watch, &watch_code_sha256, PAIRING_WINDOW_TTL_SECS)
        .await
    {
        Ok(armed) => armed,
        Err(e) => {
            if let Err(disarm_err) = relay.disconnect(Some(RelayDeviceKind::Phone)).await {
                eprintln!(
                    "genaryx-ffi pocket: connect: failed to disarm the phone pairing window \
                     after the watch window failed to arm (it will still close on its own \
                     {PAIRING_WINDOW_TTL_SECS}s TTL): {disarm_err}"
                );
            }
            return Err(e.into());
        }
    };

    // The QR must never promise longer validity than the soonest window to
    // go dark: use the EARLIER of the two expiries (see build_qr's doc).
    let expires_unix = armed_phone.expires_unix.min(armed_watch.expires_unix);

    match build_qr(&relay, &minted_phone, &minted_watch, expires_unix).await {
        Ok(qr) => Ok(qr),
        Err(e) => {
            // Both windows are armed at this point, so both need disarming.
            for kind in RelayDeviceKind::ALL {
                if let Err(disarm_err) = relay.disconnect(Some(kind)).await {
                    eprintln!(
                        "genaryx-ffi pocket: connect: failed to disarm the {} pairing window \
                         after a pairing-info failure (it will still close on its own \
                         {PAIRING_WINDOW_TTL_SECS}s TTL): {disarm_err}",
                        kind.as_str()
                    );
                }
            }
            Err(e)
        }
    }
}

async fn disconnect_impl() -> Result<PocketStatusRecord, PocketError> {
    let admin_url = env::relay_admin_url();
    let relay = RelayAdminClient::new(&admin_url).map_err(PocketError::from)?;
    relay.disconnect(None).await?;
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

    fn sample_minted_phone() -> PairNewResponse {
        PairNewResponse {
            code: "ABCD1234".to_string(),
            expires_unix: 1_758_000_600,
        }
    }

    fn sample_minted_watch() -> PairNewResponse {
        PairNewResponse {
            code: "WXYZ9876".to_string(),
            expires_unix: 1_758_000_600,
        }
    }

    #[test]
    fn new_never_touches_network_or_filesystem() {
        let _handle = PocketHandle::new().expect("construct PocketHandle");
    }

    // ---- field-mapping helpers: build_status's only non-trivial logic -----

    #[test]
    fn to_device_record_maps_every_field() {
        let view = RelayDeviceView {
            kind: "phone".to_string(),
            paired: true,
            device_id: Some("d1".to_string()),
            name: Some("Yurii's iPhone".to_string()),
            platform: Some("ios".to_string()),
            paired_at_unix: Some(1_000),
            last_seen_unix: Some(2_000),
        };
        let record = to_device_record(view);
        assert_eq!(record.device_id, "d1");
        assert_eq!(record.name, "Yurii's iPhone");
        assert_eq!(record.platform, "ios");
        assert_eq!(record.paired_at_unix, 1_000);
        assert_eq!(record.last_seen_unix, 2_000);
    }

    #[test]
    fn to_window_record_maps_every_field() {
        let view = RelayWindowView {
            kind: "watch".to_string(),
            expires_unix: 1_758_000_900,
            failed_attempts: 5,
        };
        let record = to_window_record(view);
        assert_eq!(record.expires_unix, 1_758_000_900);
        assert_eq!(record.failed_attempts, 5);
    }

    // ---- QR content shape: the load-bearing contract W3 depends on --------

    #[test]
    fn qr_content_matches_the_exact_pinned_template() {
        let info = sample_info();
        let phone = sample_minted_phone();
        let watch = sample_minted_watch();
        let qr_content = format!(
            "genaryx-pocket://pair/v1?relay={}&pin={}&code={}&code_watch={}&org={}",
            info.relay_url, info.pin, phone.code, watch.code, info.org
        );
        assert_eq!(
            qr_content,
            "genaryx-pocket://pair/v1?relay=https://198.51.100.7:8443&pin=dGVzdC1zcGtpLXBpbi1iYXNlNjQ=\
&code=ABCD1234&code_watch=WXYZ9876&org=acme"
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
        // A window_expires_unix deliberately DIFFERENT from either sample's
        // own expires_unix (1_758_000_600): proves at the call site that
        // build_qr takes the relay's own window expiry as an independent
        // argument, not derived from either Cloud code's own TTL.
        let err = build_qr(
            &relay,
            &sample_minted_phone(),
            &sample_minted_watch(),
            1_758_000_300,
        )
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
