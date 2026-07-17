//! Wire DTOs and error taxonomy for [`super::RemoteHandle`] (docs/PHASE4.md
//! W4, decision D11): UniFFI `Record`/`Enum`/`Error` mirrors of
//! `genaryx_connectors::{HetznerServer, WgPeer, WgConfig, WgInterfaceAddr,
//! SshTarget}`, plus the FFI-only [`WgStatusRecord`] verdict and
//! [`RemoteError`] taxonomy.
//!
//! ## `BTreeMap<String, String>` labels cross FFI as `Vec<LabelEntry>`
//!
//! Mirrors the map-flattening convention [`crate::drills::dto`]'s own module
//! doc establishes for `HeaderEntry` (itself following
//! [`crate::crypto::dto`]'s `CountEntry`): a plain `(key, value)` Record,
//! `Vec`-collected in the map's own `BTreeMap` (alphabetical) order.
//!
//! ## `WgStatusRecord` is a plain `Enum`, never wrapped in a `Result`
//!
//! [`super::RemoteHandle::connect_tunnel`]/[`super::RemoteHandle::tunnel_status`]
//! return this directly, not `Result<WgStatusRecord, RemoteError>`: EVERY
//! outcome a WireGuard bring-up can have - including the `wireguard-go`
//! privilege failure LOCAL testing always hits (docs/PHASE4.md W4's own
//! "Privilege reality" section: "`wireguard-go` needs root to create a tun,
//! so LOCALLY connect will FAIL with a privilege error - show it honestly as
//! FAILED, never fake-connected") - is a normal, renderable verdict, never an
//! exceptional program error. This mirrors the established "a gap verdict is
//! not an error" contract [`crate::drills::dto::DrillsError`]'s own doc calls
//! out by name for a Mockryx exit `1`, just for WireGuard's own three-state
//! verdict instead of Drills' pass/gap one. The one `WgError` failure mode
//! that genuinely CANNOT happen inside `connect_tunnel` - `WgError::KeyGen`,
//! which only [`super::RemoteHandle::wg_generate_keypair`] can ever produce,
//! before a `WgConfig` even exists - is the sole WireGuard case
//! [`RemoteError`] still carries, as [`RemoteError::WgKeyGen`].
//!
//! ## The WireGuard private key never crosses this boundary
//!
//! [`WgKeypairRecord`] carries only the PUBLIC half (`public_b64`/
//! `public_hex`) - by construction, there is no private-key field on this
//! type to leak. [`super::RemoteHandle`] generates the session keypair,
//! holds it (behind its own `Mutex`), and consumes it directly on the next
//! `connect_tunnel` call; see that handle's own module doc for the full
//! rationale.

use genaryx_connectors::{
    HetznerError as ConnHetznerError, HetznerServer as ConnHetznerServer, SshError as ConnSshError,
};
use std::collections::BTreeMap;

// ============================================================================
// map -> Vec<LabelEntry>
// ============================================================================

/// One `(key, value)` Hetzner label pair - see the module doc.
#[derive(Debug, Clone, uniffi::Record)]
pub struct LabelEntry {
    pub key: String,
    pub value: String,
}

fn labels_from(map: &BTreeMap<String, String>) -> Vec<LabelEntry> {
    map.iter()
        .map(|(key, value)| LabelEntry {
            key: key.clone(),
            value: value.clone(),
        })
        .collect()
}

// ============================================================================
// Hetzner
// ============================================================================

/// One inventory row - exact field set of `genaryx_connectors::HetznerServer`
/// (itself already read-only by construction; see that type's own module
/// doc: "there is no POST/PUT/DELETE method on this type at all"), `labels`
/// flattened per the module doc.
#[derive(Debug, Clone, uniffi::Record)]
pub struct HetznerServerRecord {
    pub id: i64,
    pub name: String,
    /// `running` | `off` | `starting` | `stopping` | `initializing` |
    /// `migrating` | `rebuilding` | `deleting` | `unknown`.
    pub status: String,
    /// The primary public IPv4, or `None` if the server has none attached.
    pub ipv4: Option<String>,
    /// The server type name, e.g. `cpx62`.
    pub server_type: String,
    pub cores: i64,
    /// RAM in GB.
    pub memory_gb: f64,
    /// The datacenter location, e.g. `nbg1`.
    pub location: String,
    /// Net hourly price in EUR for this server's location, best-effort
    /// (`None` if the per-location price row could not be found).
    pub price_hourly_eur: Option<f64>,
    pub labels: Vec<LabelEntry>,
    /// ISO-8601 creation time.
    pub created: String,
}

impl From<&ConnHetznerServer> for HetznerServerRecord {
    fn from(s: &ConnHetznerServer) -> Self {
        Self {
            id: s.id,
            name: s.name.clone(),
            status: s.status.clone(),
            ipv4: s.ipv4.clone(),
            server_type: s.server_type.clone(),
            cores: s.cores,
            memory_gb: s.memory_gb,
            location: s.location.clone(),
            price_hourly_eur: s.price_hourly_eur,
            labels: labels_from(&s.labels),
            created: s.created.clone(),
        }
    }
}

// ============================================================================
// WireGuard
// ============================================================================

/// The console's freshly generated session keypair's PUBLIC half only - see
/// the module doc's "the WireGuard private key never crosses this boundary".
#[derive(Debug, Clone, uniffi::Record)]
pub struct WgKeypairRecord {
    /// The `wg`/`.conf` encoding - what the box admin pastes into their peer
    /// config's `PublicKey =` line.
    pub public_b64: String,
    /// The UAPI hex encoding, shown alongside for an admin instead
    /// hand-editing a UAPI `set=1` block.
    pub public_hex: String,
}

/// Everything [`super::RemoteHandle::connect_tunnel`] needs beyond the
/// already-generated console keypair: the box's own WG peer (public key,
/// endpoint, allowed IPs, keepalive), the tunnel's point-to-point address
/// pair, the interface name, and the resolved `wireguard-go` binary. One
/// bundled Record (an "inputs" struct, matching
/// `crate::cloud::evidence::EvidenceBuildInputs`'s own precedent for a
/// multi-field mutation argument) rather than eight loose parameters.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ConnectTunnelInputs {
    pub wireguard_go_bin: String,
    pub interface: String,
    /// The client-hosted Cloud's WG peer public key, hex.
    pub peer_public_key_hex: String,
    /// `host:port` the peer listens on.
    pub endpoint: String,
    pub allowed_ips: Vec<String>,
    pub persistent_keepalive: Option<u16>,
    /// Almost always `None` (ephemeral) - see
    /// `genaryx_connectors::WgConfig::listen_port`'s own doc; carried through
    /// for completeness rather than hidden.
    pub listen_port: Option<u16>,
    pub local_ip: String,
    pub peer_ip: String,
}

/// The WireGuard tunnel's honest, renderable verdict - see the module doc's
/// "`WgStatusRecord` is a plain `Enum`" for why this is never wrapped in a
/// `Result`. `Connected` covers BOTH "the tunnel process is up, no handshake
/// yet" (`handshake_secs_ago: None`) and "a peer handshake has landed"
/// (`Some(secs)`) - the Remote panel's badge text branches on that field,
/// never on a fourth Rust-side variant (docs/PHASE4.md W4:
/// "connected-with-handshake" is a LABEL over this one state, not a separate
/// one).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum WgStatusRecord {
    /// No tunnel is up (the initial state, or after
    /// [`super::RemoteHandle::disconnect_tunnel`]).
    Disconnected,
    /// `wireguard-go` came up, the UAPI accepted the peer config, and the
    /// interface address was set. `handshake_secs_ago` is the UAPI's own
    /// `last_handshake_time_sec`, re-derived fresh on every read - `None`
    /// until the peer actually responds (a real, honest "not yet reachable"
    /// state, distinct from `Failed`: the LOCAL half of the channel is up).
    Connected {
        interface: String,
        handshake_secs_ago: Option<u64>,
    },
    /// Bring-up itself did not complete - ANY `WgError` (spawn/privilege
    /// failure, UAPI timeout, peer config rejected, address/route failed)
    /// folds into this ONE honest verdict, verbatim in `reason`. No tunnel
    /// exists; there is nothing to tear down.
    Failed { reason: String },
}

// ============================================================================
// SSH
// ============================================================================

/// One SSH target - exact field set of `genaryx_connectors::SshTarget`
/// (`identity_file` flattened from `PathBuf` to `String`, since UniFFI
/// Records only carry FFI-safe scalar types). Every field is
/// operator-entered; this crate generates no keys and resolves no defaults
/// for any of them (mirrors that connector's own "never generates, rotates,
/// or deletes any key" guarantee - `crates/connectors/src/ssh.rs`'s own
/// module doc).
#[derive(Debug, Clone, uniffi::Record)]
pub struct SshTargetRecord {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Path to the private-key identity file (never the key material
    /// itself).
    pub identity_file: String,
    /// The pinned host public key, `"<keytype> <base64>"`.
    pub pinned_host_key: String,
}

// ============================================================================
// error taxonomy
// ============================================================================

/// Every failure mode a [`super::RemoteHandle`] call can surface, fail-closed
/// throughout (06 §0.5: no panics/unwraps cross the FFI boundary). Collapsed
/// from `genaryx_connectors::{HetznerError, SshError}`'s variants (verbatim,
/// `Hetzner`-/`Ssh`-prefixed so the two connectors' otherwise-similar shapes -
/// both carry a transport/spawn-ish variant - stay unambiguous in one flat
/// error type), plus two ffi-layer-only additions with no connector-level
/// equivalent: [`Self::Runtime`] (this handle's own `tokio::runtime::Runtime`
/// failed to start) and [`Self::InvalidTarget`] (a [`SshTargetRecord`] with a
/// blank required field, rejected BEFORE ever shelling `ssh` - defense in
/// depth, an honest and specific message instead of letting OpenSSH itself
/// fail confusingly on e.g. `user@` with no host).
///
/// `WgError` is DELIBERATELY absent here except [`Self::WgKeyGen`] - see
/// [`WgStatusRecord`]'s own doc for why every OTHER WireGuard failure mode is
/// a [`WgStatusRecord::Failed`] verdict, never a thrown error.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum RemoteError {
    #[error("could not start the local async runtime: {reason}")]
    Runtime { reason: String },
    #[error("could not build the hetzner client: {reason}")]
    HetznerBuild { reason: String },
    #[error("hetzner returned HTTP {status}: {body}")]
    HetznerApi { status: u16, body: String },
    #[error("could not reach hetzner: {reason}")]
    HetznerTransport { reason: String },
    #[error("unexpected response shape from hetzner: {reason}")]
    HetznerJson { reason: String },
    #[error("could not generate a WireGuard keypair: {reason}")]
    WgKeyGen { reason: String },
    #[error("invalid SSH target: {reason}")]
    InvalidTarget { reason: String },
    #[error("could not pin the SSH host key: {reason}")]
    SshPin { reason: String },
    #[error("could not run ssh: {reason}")]
    SshSpawn { reason: String },
    #[error("ssh exited {code}: {stderr}")]
    SshRemote { code: i32, stderr: String },
}

impl From<ConnHetznerError> for RemoteError {
    fn from(e: ConnHetznerError) -> Self {
        match e {
            ConnHetznerError::Build(reason) => RemoteError::HetznerBuild { reason },
            ConnHetznerError::Transport(err) => RemoteError::HetznerTransport {
                reason: err.to_string(),
            },
            ConnHetznerError::Api { status, body } => RemoteError::HetznerApi { status, body },
            ConnHetznerError::Json(err) => RemoteError::HetznerJson {
                reason: err.to_string(),
            },
        }
    }
}

impl From<ConnSshError> for RemoteError {
    fn from(e: ConnSshError) -> Self {
        match e {
            ConnSshError::Pin { path, source } => RemoteError::SshPin {
                reason: format!("{path}: {source}"),
            },
            ConnSshError::Spawn(err) => RemoteError::SshSpawn {
                reason: err.to_string(),
            },
            ConnSshError::Remote { code, stderr } => RemoteError::SshRemote { code, stderr },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_server() -> ConnHetznerServer {
        let mut labels = BTreeMap::new();
        labels.insert("managed-by".to_string(), "taipan".to_string());
        labels.insert("campaign".to_string(), "w4".to_string());
        ConnHetznerServer {
            id: 42,
            name: "taipan-live-1".to_string(),
            status: "running".to_string(),
            ipv4: Some("203.0.113.7".to_string()),
            server_type: "cpx62".to_string(),
            cores: 16,
            memory_gb: 32.0,
            location: "nbg1".to_string(),
            price_hourly_eur: Some(0.05),
            labels,
            created: "2026-07-17T10:00:00+00:00".to_string(),
        }
    }

    #[test]
    fn hetzner_server_record_flattens_labels_and_preserves_every_field() {
        let record = HetznerServerRecord::from(&sample_server());
        assert_eq!(record.id, 42);
        assert_eq!(record.name, "taipan-live-1");
        assert_eq!(record.status, "running");
        assert_eq!(record.ipv4.as_deref(), Some("203.0.113.7"));
        assert_eq!(record.server_type, "cpx62");
        assert_eq!(record.cores, 16);
        assert_eq!(record.memory_gb, 32.0);
        assert_eq!(record.location, "nbg1");
        assert_eq!(record.price_hourly_eur, Some(0.05));
        assert_eq!(record.labels.len(), 2);
        assert!(
            record
                .labels
                .iter()
                .any(|e| e.key == "managed-by" && e.value == "taipan")
        );
        assert!(
            record
                .labels
                .iter()
                .any(|e| e.key == "campaign" && e.value == "w4")
        );
        assert_eq!(record.created, "2026-07-17T10:00:00+00:00");
    }

    #[test]
    fn hetzner_server_record_with_no_ip_and_no_price_stays_honest_none() {
        let server = ConnHetznerServer {
            id: 43,
            name: "no-ip-box".to_string(),
            status: "off".to_string(),
            ipv4: None,
            server_type: "cx11".to_string(),
            cores: 1,
            memory_gb: 2.0,
            location: "hel1".to_string(),
            price_hourly_eur: None,
            labels: BTreeMap::new(),
            created: String::new(),
        };
        let record = HetznerServerRecord::from(&server);
        assert!(record.ipv4.is_none());
        assert!(record.price_hourly_eur.is_none());
        assert!(record.labels.is_empty());
    }

    #[test]
    fn hetzner_build_error_maps_to_remote_error_hetzner_build() {
        match RemoteError::from(ConnHetznerError::Build("tls init failed".to_string())) {
            RemoteError::HetznerBuild { reason } => assert_eq!(reason, "tls init failed"),
            other => panic!("expected HetznerBuild, got {other:?}"),
        }
    }

    #[test]
    fn hetzner_api_error_maps_verbatim() {
        match RemoteError::from(ConnHetznerError::Api {
            status: 401,
            body: "unauthorized".to_string(),
        }) {
            RemoteError::HetznerApi { status, body } => {
                assert_eq!(status, 401);
                assert_eq!(body, "unauthorized");
            }
            other => panic!("expected HetznerApi, got {other:?}"),
        }
    }

    #[test]
    fn hetzner_json_error_maps_to_remote_error_hetzner_json() {
        let json_err = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        match RemoteError::from(ConnHetznerError::Json(json_err)) {
            RemoteError::HetznerJson { reason } => assert!(!reason.is_empty()),
            other => panic!("expected HetznerJson, got {other:?}"),
        }
    }

    #[test]
    fn ssh_remote_error_maps_verbatim() {
        match RemoteError::from(ConnSshError::Remote {
            code: 255,
            stderr: "Host key verification failed".to_string(),
        }) {
            RemoteError::SshRemote { code, stderr } => {
                assert_eq!(code, 255);
                assert_eq!(stderr, "Host key verification failed");
            }
            other => panic!("expected SshRemote, got {other:?}"),
        }
    }

    #[test]
    fn ssh_spawn_error_maps_to_remote_error_ssh_spawn() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");
        match RemoteError::from(ConnSshError::Spawn(io_err)) {
            RemoteError::SshSpawn { reason } => assert!(reason.contains("no such file")),
            other => panic!("expected SshSpawn, got {other:?}"),
        }
    }

    #[test]
    fn ssh_pin_error_maps_to_remote_error_ssh_pin_naming_the_path() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        match RemoteError::from(ConnSshError::Pin {
            path: "/tmp/genaryx-ssh-known-1".to_string(),
            source: io_err,
        }) {
            RemoteError::SshPin { reason } => {
                assert!(reason.contains("/tmp/genaryx-ssh-known-1"));
                assert!(reason.contains("denied"));
            }
            other => panic!("expected SshPin, got {other:?}"),
        }
    }
}
