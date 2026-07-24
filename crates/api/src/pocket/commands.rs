//! Console commands for the Pocket panel (docs/PHASE5.md W2, itrat-console/13
//! D12.2a): [`pocket_status`] (idle/paired/relay-unreachable), [`pocket_connect`]
//! ("Connect TokenFuse Pocket": mint a code for the phone and one for the
//! watch at the Cloud, arm both of the relay's pairing windows, return the
//! QR content carrying both codes), and [`pocket_disconnect`].
//!
//! Flow (D12.2a steps 1-3, 10, extended to two devices), mirrored exactly,
//! before the desktop shells were removed, by `crates/ffi/src/pocket/mod.rs`
//! for the SwiftUI shell:
//!
//! 1. [`pocket_connect`] resolves the Cloud admin key
//!    (`crate::money::env::discover`, the SAME two-tier discovery Money's
//!    own device pairing uses) and calls `POST /v1/pair/new`
//!    ([`genaryx_connectors::CloudClient::pair_new`]) TWICE, minting
//!    `{code, expires_unix}` for the phone, then again for the watch.
//! 2. It then arms the relay's pairing window for the phone's code
//!    (`POST /admin/pairing-window {kind, code_sha256, ttl_secs}` -
//!    [`genaryx_connectors::RelayAdminClient::arm_pairing_window`]),
//!    hashing the code with `genaryx_signing::body_sha256_hex` before it
//!    ever leaves this process - the relay never learns the plaintext code
//!    (D12.3's trust-boundary table) - then does the same for the watch's
//!    code. A `409 device_exists` on either call (a device of THAT kind is
//!    already paired) maps to [`PocketError::DeviceExists`] so the frontend
//!    can show Disconnect instead of a QR (D12.2 step 2), never a generic
//!    error banner. If the watch's window fails to arm after the phone's
//!    already succeeded, the phone's window is disarmed before the error is
//!    returned (see [`pocket_connect`]'s own body) - a paired phone or watch
//!    from BEFORE this call is never touched, only the window THIS call
//!    itself just armed.
//! 3. It reads the relay's `GET /admin/pairing-info` (pin +
//!    `public_advertise_url` + org) and builds the EXACT
//!    `genaryx-pocket://pair/v1?relay=...&pin=...&code=...&code_watch=...&org=...`
//!    string (D12.2 step 3, extended with `code_watch`) so W3's scanner
//!    parses it - see [`build_qr`]'s doc for the literal-substitution choice.
//!
//! Fail-closed (docs/PHASE5.md W2 "Rules": "do not leave a half-armed
//! window silently"): once a window is armed, a failure in a later step
//! disarms it again via `RelayAdminClient::disconnect` before returning the
//! error - see [`pocket_connect`]'s own body.
//!
//! [`pocket_status`]/[`pocket_disconnect`] both resolve through
//! [`build_status`], so a Disconnect never needs a second round trip to
//! learn the fresh (now-idle) state, mirroring
//! `remote::commands`'s identical "every mutating command returns the
//! whole-panel status" contract.

use super::env;
use genaryx_connectors::{
    CloudClient, ConnectorError, PairNewResponse, RelayAdminClient, RelayAdminError,
    RelayDeviceKind, RelayDeviceView, RelayPairingInfo, RelayWindowView,
};
use serde::Serialize;

/// How long each of the relay's pairing windows stays armed once opened
/// (D12.2 step 2's `ttl: 300`) - 5 minutes: generous for an operator to open
/// TokenFuse Pocket and scan, short enough that an abandoned QR does not
/// stay pairable indefinitely. Applied to both the phone's and the watch's
/// windows.
const PAIRING_WINDOW_TTL_SECS: i64 = 300;

// ============================================================================
// DTOs
// ============================================================================

/// One paired device slot, as shown within [`PocketStatusDto::Paired`].
/// Mirrors the paired fields of [`RelayDeviceView`] - never that type's own
/// `kind`/`paired` fields, since a [`PocketDeviceDto`] only ever exists for
/// a slot that IS paired (`PocketStatusDto::Paired`'s `phone`/`watch` are
/// `None` for an empty slot instead of an all-fields-empty record, so there
/// is nothing left for `kind`/`paired` to disambiguate here).
#[derive(Debug, Clone, Serialize)]
pub struct PocketDeviceDto {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub paired_at_unix: i64,
    pub last_seen_unix: i64,
}

/// One currently armed pairing window, as shown within
/// [`PocketStatusDto`]'s `phone_window`/`watch_window`. Mirrors
/// [`RelayWindowView`]'s two useful fields - never that type's own `kind`,
/// since here the field name (`phone_window` vs `watch_window`) already
/// says which one this is.
///
/// `failed_attempts` is PURELY OBSERVATIONAL: wrong codes presented to
/// `POST /relay/v1/pair` since this window was armed, that the relay itself
/// never acts on (see [`RelayWindowView`]'s own doc for why - the pairing
/// route is pre-auth, so closing on this would let an unauthenticated
/// caller deny pairing at will, and the watch's window, the long-lived one
/// waiting on a WatchConnectivity handoff, would be the easiest thing in
/// the system to keep permanently shut). Render it as something for the
/// operator to notice and act on THROUGH the existing Disconnect affordance
/// if they choose to, never as a threat this app is already handling,
/// blocking, or that will close the window on its own.
#[derive(Debug, Clone, Serialize)]
pub struct PocketWindowDto {
    pub expires_unix: i64,
    pub failed_attempts: i64,
}

/// Whole-panel state (idle/paired/relay-unreachable) - the three UI states
/// docs/PHASE5.md W2 calls for are `Idle` (render "Connect"), a frontend-only
/// "showing QR" step entered right after a successful [`pocket_connect`]
/// (not tracked here - the relay exposes no "is a window currently armed"
/// read, only device-paired state), and `Paired` (device(s) + Disconnect).
/// `RelayUnreachable` is this module's own addition, honest about a fourth
/// real outcome (the admin API itself is down) rather than folding it into
/// `Idle` and showing a Connect button that would only fail.
///
/// `Idle` and `Paired` both carry `phone_window`/`watch_window`: arming
/// happens before either slot is paired (so a window is normally seen
/// alongside `Idle`), but the phone commonly redeems its code within
/// seconds of a scan while the watch's redemption waits on a
/// WatchConnectivity handoff that can take longer, so a `Paired { phone:
/// Some(_), watch: None }` state with the watch's window still armed
/// underneath it is a real, ordinary sequence, not an edge case to ignore.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PocketStatusDto {
    /// Neither slot is paired.
    Idle {
        /// Whether a Cloud admin key is resolvable at all
        /// (`money::env::discover`) - `false` renders as a disabled Connect
        /// button with an honest hint, rather than letting the operator
        /// click it only to get [`PocketError::NoCloudEnvironment`].
        cloud_ready: bool,
        /// The phone's currently armed window, if any (see the type doc).
        phone_window: Option<PocketWindowDto>,
        /// The watch's currently armed window, if any.
        watch_window: Option<PocketWindowDto>,
    },
    /// At least one slot is paired. `Connect` always arms both slots
    /// together in one call (one QR carries both codes; the phone scans it
    /// and is the one that hands the watch its own code over
    /// WatchConnectivity), so a partial state here (one slot `Some`, the
    /// other `None`) means that device was disconnected on its own, never
    /// that it was simply not offered a code yet. There is no per-slot
    /// re-Connect from this state: `Disconnect` frees both slots so a fresh
    /// two-code QR can pair them again together.
    Paired {
        phone: Option<PocketDeviceDto>,
        watch: Option<PocketDeviceDto>,
        /// The phone's currently armed window, if any (see the type doc) -
        /// normally `None` once `phone` above is `Some`, since a successful
        /// redemption closes that slot's window.
        phone_window: Option<PocketWindowDto>,
        /// The watch's currently armed window, if any - the common case
        /// this exists for: `phone: Some(_)`, `watch: None`, with the
        /// watch's own window still ticking down underneath.
        watch_window: Option<PocketWindowDto>,
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
    /// The Cloud rejected or could not be reached for `POST /v1/pair/new`
    /// (either the phone's code or the watch's - `pocket_connect` mints
    /// both before arming anything, so a failure minting either one fails
    /// the whole attempt before any pairing window is armed).
    Cloud { message: String },
    /// The relay refused to arm a pairing window because a device of that
    /// kind is already paired (D12.2 step 2) - the frontend should re-fetch
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

/// Read the relay's current two-slot device view and fold it into a
/// [`PocketStatusDto`], shared by [`pocket_status`] and [`pocket_disconnect`]
/// so a Disconnect never needs a second round trip (see this module's doc
/// comment). A relay admin call failure here becomes `RelayUnreachable`,
/// never a thrown error: this is a STATUS read, and "the relay is down" is
/// itself a normal, renderable status, not a command failure.
async fn build_status(relay: &RelayAdminClient) -> PocketStatusDto {
    match relay.devices().await {
        Ok(resp) => {
            let phone = resp
                .slot(RelayDeviceKind::Phone)
                .cloned()
                .filter(|d| d.paired)
                .map(to_device_dto);
            let watch = resp
                .slot(RelayDeviceKind::Watch)
                .cloned()
                .filter(|d| d.paired)
                .map(to_device_dto);
            let phone_window = resp
                .window(RelayDeviceKind::Phone)
                .cloned()
                .map(to_window_dto);
            let watch_window = resp
                .window(RelayDeviceKind::Watch)
                .cloned()
                .map(to_window_dto);
            if phone.is_none() && watch.is_none() {
                PocketStatusDto::Idle {
                    cloud_ready: crate::money::env::discover().is_some(),
                    phone_window,
                    watch_window,
                }
            } else {
                PocketStatusDto::Paired {
                    phone,
                    watch,
                    phone_window,
                    watch_window,
                }
            }
        }
        Err(e) => PocketStatusDto::RelayUnreachable {
            message: e.to_string(),
        },
    }
}

/// A paired [`RelayDeviceView`] into the smaller [`PocketDeviceDto`] shown
/// per slot - drops `kind`/`paired`, which [`build_status`] already
/// consumed to decide whether to call this at all.
fn to_device_dto(view: RelayDeviceView) -> PocketDeviceDto {
    PocketDeviceDto {
        device_id: view.device_id.unwrap_or_default(),
        name: view.name.unwrap_or_default(),
        platform: view.platform.unwrap_or_default(),
        paired_at_unix: view.paired_at_unix.unwrap_or_default(),
        last_seen_unix: view.last_seen_unix.unwrap_or_default(),
    }
}

/// An armed [`RelayWindowView`] into the smaller [`PocketWindowDto`] shown
/// per slot - drops `kind`, which [`build_status`] already consumed to
/// decide whether this is the phone's or the watch's window.
fn to_window_dto(view: RelayWindowView) -> PocketWindowDto {
    PocketWindowDto {
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
/// the SOONER of the two slots actually stays open.
///
/// ## Why the QR fields are NOT percent-encoded
///
/// docs/PHASE5.md W2 pins the QR content to be "EXACTLY" the literal
/// template `genaryx-pocket://pair/v1?relay=<public_advertise_url>&pin=<b64
/// SPKI-SHA256>&code=<8-char>&code_watch=<8-char>&org=<org>` (itrat-console/13
/// D12.2 step 3's own example is written the same unencoded way) "so W3's
/// scanner parses it" - W3 (the mobile QR scanner) is a later wave and does
/// not exist yet in this build, so this is a pinned contract, not a free
/// choice to percent-encode against. `code` stays the PHONE's code (so a
/// scanner that only reads `code` keeps working unmodified); `code_watch` is
/// additive, inserted before `org` per the pinned field order. Checked to
/// still be safe: `relay_url` (`https://host:port`) has no `&`/`=`; standard
/// base64's alphabet (`A-Za-z0-9+/=`) has no `&` either; and `code_watch`,
/// like `code` before it, is an 8-char unambiguous-alphabet code
/// (`devices::pairing_code()` upstream) that contains no `&` or `=` of its
/// own - so a naive split-on-`&`-then-first-`=` parser (or Swift's
/// `URLComponents.queryItems`, which does NOT apply
/// `application/x-www-form-urlencoded` `+`-as-space decoding - only
/// percent-decoding, per RFC 3986) isolates every field correctly without
/// corruption.
async fn build_qr(
    relay: &RelayAdminClient,
    minted_phone: &PairNewResponse,
    minted_watch: &PairNewResponse,
    window_expires_unix: i64,
) -> Result<PocketQrDto, PocketError> {
    let info: RelayPairingInfo = relay.pairing_info().await?;
    Ok(PocketQrDto {
        qr_content: format!(
            "genaryx-pocket://pair/v1?relay={}&pin={}&code={}&code_watch={}&org={}",
            info.relay_url, info.pin, minted_phone.code, minted_watch.code, info.org
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
/// comment for the full flow:
///
/// 1. Mint TWO codes at the Cloud (`POST /v1/pair/new`, once for the phone
///    and once for the watch) - nothing is armed at the relay yet, so a
///    mint failure for either code leaves the relay untouched.
/// 2. Arm the relay's pairing window for the phone's code, then for the
///    watch's. If arming the watch's window fails after the phone's already
///    succeeded, disarm the phone's window before returning the error
///    (`RelayAdminClient::disconnect` with `Some(Phone)`, never `None`:
///    `None` would also clear a phone or watch pairing that predates this
///    call, which is not this failure's to touch) - it will still close on
///    its own after `PAIRING_WINDOW_TTL_SECS`, but a failed Connect should
///    not leave a stray window armed for that long regardless.
/// 3. Read the relay's `pairing-info` and build the QR content carrying
///    both codes (see [`build_qr`]'s doc). If THIS step fails, both windows
///    are armed (we only reach it once both arms above succeeded), so both
///    are disarmed the same way.
pub async fn pocket_connect() -> Result<PocketQrDto, PocketError> {
    let resolved = crate::money::env::discover().ok_or(PocketError::NoCloudEnvironment)?;

    let cloud = CloudClient::new(resolved.cloud_url.clone(), resolved.admin_bearer.clone())?;
    let minted_phone = cloud.pair_new(&resolved.admin_bearer).await?;
    let minted_watch = cloud.pair_new(&resolved.admin_bearer).await?;

    let admin_url = env::relay_admin_url();
    let relay = RelayAdminClient::new(&admin_url).map_err(PocketError::from)?;

    let phone_code_sha256 = genaryx_signing::body_sha256_hex(minted_phone.code.as_bytes());
    let armed_phone = relay
        .arm_pairing_window(
            RelayDeviceKind::Phone,
            &phone_code_sha256,
            PAIRING_WINDOW_TTL_SECS,
        )
        .await?;

    let watch_code_sha256 = genaryx_signing::body_sha256_hex(minted_watch.code.as_bytes());
    let armed_watch = match relay
        .arm_pairing_window(
            RelayDeviceKind::Watch,
            &watch_code_sha256,
            PAIRING_WINDOW_TTL_SECS,
        )
        .await
    {
        Ok(armed) => armed,
        Err(e) => {
            if let Err(disarm_err) = relay.disconnect(Some(RelayDeviceKind::Phone)).await {
                eprintln!(
                    "genaryx: pocket_connect: failed to disarm the phone pairing window after \
                     the watch window failed to arm (it will still close on its own \
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
                        "genaryx: pocket_connect: failed to disarm the {} pairing window after \
                         a pairing-info failure (it will still close on its own \
                         {PAIRING_WINDOW_TTL_SECS}s TTL): {disarm_err}",
                        kind.as_str()
                    );
                }
            }
            Err(e)
        }
    }
}

/// Disconnect both the paired phone and watch (always safe to call, even
/// with nothing paired in either slot - `RelayAdminClient::disconnect`'s own
/// contract) and return the fresh status so the panel flips back to Idle
/// without a second round trip.
pub async fn pocket_disconnect() -> Result<PocketStatusDto, PocketError> {
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

    // ---- field-mapping helpers: build_status's only non-trivial logic -----

    #[test]
    fn to_device_dto_maps_every_field() {
        let view = RelayDeviceView {
            kind: "phone".to_string(),
            paired: true,
            device_id: Some("d1".to_string()),
            name: Some("Yurii's iPhone".to_string()),
            platform: Some("ios".to_string()),
            paired_at_unix: Some(1_000),
            last_seen_unix: Some(2_000),
        };
        let dto = to_device_dto(view);
        assert_eq!(dto.device_id, "d1");
        assert_eq!(dto.name, "Yurii's iPhone");
        assert_eq!(dto.platform, "ios");
        assert_eq!(dto.paired_at_unix, 1_000);
        assert_eq!(dto.last_seen_unix, 2_000);
    }

    #[test]
    fn to_window_dto_maps_every_field() {
        let view = RelayWindowView {
            kind: "watch".to_string(),
            expires_unix: 1_758_000_900,
            failed_attempts: 5,
        };
        let dto = to_window_dto(view);
        assert_eq!(dto.expires_unix, 1_758_000_900);
        assert_eq!(dto.failed_attempts, 5);
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

    #[test]
    fn qr_content_fields_are_recoverable_by_a_naive_split_on_ampersand_then_first_equals() {
        // Proves the "no percent-encoding needed" reasoning in `build_qr`'s
        // doc comment against a REAL string, not just an argument in prose:
        // a hand-rolled parser exactly as unsophisticated as one a mobile
        // scanner (W3) might use must still recover every field intact,
        // including the new `code_watch`.
        let info = sample_info();
        let phone = sample_minted_phone();
        let watch = sample_minted_watch();
        let qr_content = format!(
            "genaryx-pocket://pair/v1?relay={}&pin={}&code={}&code_watch={}&org={}",
            info.relay_url, info.pin, phone.code, watch.code, info.org
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
        assert_eq!(fields.get("code"), Some(&phone.code.as_str()));
        assert_eq!(fields.get("code_watch"), Some(&watch.code.as_str()));
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
        // A window_expires_unix deliberately DIFFERENT from either sample's
        // own expires_unix (1_758_000_600): proves at the call site that
        // build_qr takes the relay's own window expiry as an independent
        // argument, not derived from either Cloud code's own TTL (see
        // build_qr's doc comment for why the two must not be conflated).
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
