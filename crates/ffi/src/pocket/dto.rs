//! Wire Records/Enum/Error for [`super::PocketHandle`] (docs/PHASE5.md W2).
//! Mirrors the Tauri shell's `apps/desktop/src-tauri/src/pocket/commands.rs`
//! DTOs field-for-field (same shell-parity convention `remote/dto.rs`'s own
//! module doc follows against `remote::commands`).

use genaryx_connectors::{ConnectorError, RelayAdminError};

/// Mirrors `pocket::commands::PocketStatusDto`. A plain Enum, never wrapped
/// in a `Result` - see [`super::PocketHandle::status`]'s own doc for why
/// every outcome (including the relay being unreachable) is a normal,
/// renderable verdict, the same "a verdict, not an exceptional program
/// error" contract `remote::dto::WgStatusRecord`'s own module doc
/// establishes for the WireGuard tunnel.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum PocketStatusRecord {
    Idle {
        /// Whether a Cloud admin key is resolvable at all
        /// (`crate::cloud::env::discover`) - `false` should render as a
        /// disabled Connect affordance with an honest hint, rather than
        /// letting the operator tap it only to get
        /// [`PocketError::NoCloudEnvironment`].
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

/// Mirrors `pocket::commands::PocketQrDto` - [`super::PocketHandle::connect`]'s
/// success return. `qr_content` is the EXACT `genaryx-pocket://pair/v1?...`
/// string (docs/PHASE5.md W2); render it verbatim, never reconstruct it in
/// Swift.
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
    /// The Cloud rejected or could not be reached for `POST /v1/pair/new`.
    #[error("cloud error: {message}")]
    Cloud { message: String },
    /// The relay refused to arm a pairing window because a device is
    /// already paired (D12.2 step 2) - the Swift side should re-fetch
    /// [`super::PocketHandle::status`] and render the Paired view, not an
    /// error banner.
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
