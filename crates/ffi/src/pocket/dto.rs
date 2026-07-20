//! Wire Records/Enum/Error for [`super::PocketHandle`] (docs/PHASE5.md W2).
//! Mirrors the Tauri shell's `apps/desktop/src-tauri/src/pocket/commands.rs`
//! DTOs field-for-field (same shell-parity convention `remote/dto.rs`'s own
//! module doc follows against `remote::commands`).

use genaryx_connectors::{ConnectorError, RelayAdminError};

/// One paired device slot, as shown within [`PocketStatusRecord::Paired`].
/// Mirrors the paired fields of `genaryx_connectors::RelayDeviceView` --
/// never that type's own `kind`/`paired` fields, since a
/// [`PocketDeviceRecord`] only ever exists for a slot that IS paired
/// ([`PocketStatusRecord::Paired`]'s `phone`/`watch` are `None` for an empty
/// slot instead of an all-fields-empty record, so there is nothing left for
/// `kind`/`paired` to disambiguate here).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PocketDeviceRecord {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub paired_at_unix: i64,
    pub last_seen_unix: i64,
}

/// One currently armed pairing window, as shown within
/// [`PocketStatusRecord`]'s `phone_window`/`watch_window`. Mirrors
/// `genaryx_connectors::RelayWindowView`'s two useful fields -- never that
/// type's own `kind`, since here the field name (`phone_window` vs
/// `watch_window`) already says which one this is.
///
/// `failed_attempts` is PURELY OBSERVATIONAL: wrong codes presented to
/// `POST /relay/v1/pair` since this window was armed, that the relay itself
/// never acts on (see `RelayWindowView`'s own doc for why - the pairing
/// route is pre-auth, so closing on this would let an unauthenticated
/// caller deny pairing at will, and the watch's window, the long-lived one
/// waiting on a WatchConnectivity handoff, would be the easiest thing in
/// the system to keep permanently shut). Render it as something for the
/// operator to notice and act on THROUGH the existing Disconnect affordance
/// if they choose to, never as a threat this app is already handling,
/// blocking, or that will close the window on its own.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PocketWindowRecord {
    pub expires_unix: i64,
    pub failed_attempts: i64,
}

/// Mirrors `pocket::commands::PocketStatusDto`. A plain Enum, never wrapped
/// in a `Result` - see [`super::PocketHandle::status`]'s own doc for why
/// every outcome (including the relay being unreachable) is a normal,
/// renderable verdict, the same "a verdict, not an exceptional program
/// error" contract `remote::dto::WgStatusRecord`'s own module doc
/// establishes for the WireGuard tunnel.
///
/// `Idle` and `Paired` both carry `phone_window`/`watch_window`: arming
/// happens before either slot is paired (so a window is normally seen
/// alongside `Idle`), but the phone commonly redeems its code within
/// seconds of a scan while the watch's redemption waits on a
/// WatchConnectivity handoff that can take longer, so a `Paired { phone:
/// Some(_), watch: None }` state with the watch's window still armed
/// underneath it is a real, ordinary sequence, not an edge case to ignore.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum PocketStatusRecord {
    /// Neither slot is paired.
    Idle {
        /// Whether a Cloud admin key is resolvable at all
        /// (`crate::cloud::env::discover`) - `false` should render as a
        /// disabled Connect affordance with an honest hint, rather than
        /// letting the operator tap it only to get
        /// [`PocketError::NoCloudEnvironment`].
        cloud_ready: bool,
        /// The phone's currently armed window, if any (see the type doc).
        phone_window: Option<PocketWindowRecord>,
        /// The watch's currently armed window, if any.
        watch_window: Option<PocketWindowRecord>,
    },
    /// At least one slot is paired. `Connect` always arms both slots
    /// together in one call (one QR carries both codes; the phone scans it
    /// and is the one that hands the watch its own code over
    /// WatchConnectivity - see [`super::connect_impl`]'s doc), so a partial
    /// state here (one slot `Some`, the other `None`) means that device was
    /// disconnected on its own, never that it was simply not offered a code
    /// yet. There is no per-slot re-Connect from this state: `Disconnect`
    /// frees both slots so a fresh two-code QR can pair them again together.
    Paired {
        phone: Option<PocketDeviceRecord>,
        watch: Option<PocketDeviceRecord>,
        /// The phone's currently armed window, if any (see the type doc) -
        /// normally `None` once `phone` above is `Some`, since a successful
        /// redemption closes that slot's window.
        phone_window: Option<PocketWindowRecord>,
        /// The watch's currently armed window, if any - the common case
        /// this exists for: `phone: Some(_)`, `watch: None`, with the
        /// watch's own window still ticking down underneath.
        watch_window: Option<PocketWindowRecord>,
    },
    RelayUnreachable {
        message: String,
    },
}

/// Mirrors `pocket::commands::PocketQrDto` - [`super::PocketHandle::connect`]'s
/// success return. `qr_content` is the EXACT `genaryx-pocket://pair/v1?...`
/// string (docs/PHASE5.md W2), now carrying both the phone's and the
/// watch's codes; render it verbatim, never reconstruct it in Swift.
#[derive(Debug, Clone, uniffi::Record)]
pub struct PocketQrRecord {
    pub qr_content: String,
    pub expires_unix: i64,
}

/// Every failure mode a [`super::PocketHandle`] call can surface. Mirrors
/// `pocket::commands::PocketError`, plus [`Self::Runtime`] (this handle's
/// own `tokio::runtime::Runtime` failed to start - an ffi-layer-only
/// addition with no Tauri-side equivalent, the same shape as
/// `remote::dto::RemoteError::Runtime`).
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum PocketError {
    #[error("could not start the local async runtime: {reason}")]
    Runtime { reason: String },
    /// No Cloud admin key resolvable (`crate::cloud::env::discover` returned
    /// `None`) - the same honest "no environment" outcome `CloudHandle`
    /// itself surfaces, never a fabricated Connect attempt.
    #[error("no TokenFuse Cloud environment found")]
    NoCloudEnvironment,
    /// The Cloud rejected or could not be reached for `POST /v1/pair/new`
    /// (either the phone's code or the watch's - `connect_impl` mints both
    /// before arming anything, so a failure minting either one fails the
    /// whole attempt before any pairing window is armed).
    #[error("cloud error: {message}")]
    Cloud { message: String },
    /// The relay refused to arm a pairing window because a device of that
    /// kind is already paired (D12.2 step 2) - the Swift side should
    /// re-fetch [`super::PocketHandle::status`] and render the Paired view,
    /// not an error banner.
    #[error("a device is already paired at the relay")]
    DeviceExists,
    /// The relay's admin API itself failed (unreachable, or any non-2xx
    /// other than the `DeviceExists` case above).
    #[error("relay error: {message}")]
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
