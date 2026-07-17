//! Tauri commands for the Remote (Distance) view (docs/PHASE4.md W4):
//! `remote_status` plus [`remote_set_environment`] (save the operator's WG
//! peer + SSH target + `wireguard-go` path), the read-only
//! [`remote_hetzner_list`], the WG [`remote_wg_connect`]/
//! [`remote_wg_disconnect`] pair, and the SSH ops
//! [`remote_ssh_check_reachable`]/[`remote_ssh_read_file`]/
//! [`remote_ssh_tail_start`]/[`remote_ssh_tail_stop`].
//!
//! ## Why the console's WG identity survives an environment edit
//!
//! [`remote_set_environment`] replaces the environment, resets the SSH
//! client (a stale pin must never survive pointing the panel at a different
//! box), and resets the tunnel to `Disconnected` (any previous tunnel is
//! torn down - see `state.rs`'s "fail-closed lifecycle" doc). It does NOT
//! touch `console_keypair`: the console's own WG public key is an identity
//! the operator may already have handed to a box admin, independent of which
//! remote box the environment currently points at - regenerating it on every
//! form edit would silently invalidate that handoff. [`remote_wg_connect`]
//! generates it exactly once, lazily, the first time it is needed.
//!
//! ## Why a failed WG bring-up is still `Ok(...)`, never `Err(...)`
//!
//! `remote_wg_connect`'s Rust `Result::Err` means "the command itself could
//! not run" (a task panic, a poisoned mutex) - not "the tunnel failed to come
//! up". A failed `bring_up` is a normal, expected, HONEST outcome (this local
//! box almost certainly lacks tun-device privileges - see this module's
//! `remote_wg_connect` doc), so it is folded into
//! `TunnelStatusDto::Failed{message}` inside a successful
//! `Ok(RemoteStatusDto)`, the same "exit 1 is still a real report, never a
//! command error" shape `drills::commands::drills_run` uses for a mockryx
//! fire-drill gap. The `Failed` state is written into the durable `tunnel`
//! cell BEFORE returning, so a later `remote_status` poll still shows it,
//! never silently reverting to `Disconnected` as if nothing was attempted.
//!
//! ## Streaming the remote tail
//!
//! [`remote_ssh_tail_start`] spawns `ssh ... tail -F` (`SshClient::spawn_tail`)
//! and hands its stdout to a dedicated `std::thread` that emits one
//! `remote:tail-line` Tauri event per line, then a `remote:tail-ended` event
//! on EOF/error - mirrors `live.rs`'s own `std::thread::spawn` +
//! `app_handle.emit(...)` idiom for its `bus:event` feed, adapted from a
//! fixed 2s-tick demo feeder to an unbounded read loop over a real child's
//! pipe. The child itself is kept in the `tail` cell so
//! [`remote_ssh_tail_stop`] (or a fresh `remote_ssh_tail_start`) can kill it
//! (only ever a process this panel itself spawned, never a
//! `ps`/`lsof`-discovered PID).
//!
//! Every blocking connector call (Hetzner's `list_servers` is the one
//! exception - genuinely `async` reqwest I/O, awaited directly like
//! `money::commands`' Cloud calls) runs inside
//! `tauri::async_runtime::spawn_blocking`, mirroring every sibling panel's
//! identical discipline.

use super::state::{
    RemoteClient, RemoteEnvironmentConfig, RemoteInner, RemoteState, TailSession, TunnelState,
};
use genaryx_connectors::{
    HetznerClient, HetznerError, HetznerServer, SshClient, SshError, SshTarget, WgConfig, WgError,
    WgInterfaceAddr, WgKeypair, WgPeer, WgTunnel,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use tauri::Emitter;

// ============================================================================
// DTOs
// ============================================================================

/// Whole-panel state - mirrors `evidence::commands::EvidenceStatusDto`'s
/// shape (no `NoEnvironment` variant: Hetzner/WG/SSH are three independent
/// capabilities, not one all-or-nothing plane - see `state.rs`'s module
/// doc).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RemoteStatusDto {
    Bootstrapping,
    Ready {
        default_wireguard_go_bin: Option<String>,
        /// Boxed only to keep this variant's stack footprint down next to
        /// `Bootstrapping`'s zero bytes (clippy `large_enum_variant`) -
        /// `Box<T>` serializes exactly like `T` (serde's blanket impl is
        /// transparent), so this changes nothing on the wire.
        environment: Option<Box<RemoteEnvironmentDto>>,
        /// The console's own WG public key, once generated (see this
        /// module's doc comment for why it outlives an environment edit) -
        /// `null` until the first `remote_wg_connect` call.
        console_public_b64: Option<String>,
        tunnel: TunnelStatusDto,
        tail: Option<TailStatusDto>,
    },
}

/// Mirrors [`RemoteEnvironmentConfig`] flattened for the wire, `ssh_target`
/// split back into its plain fields (the connector's `SshTarget` has no
/// `Serialize` of its own - it is frozen, W4 scope, `crates/connectors/src/ssh.rs`).
#[derive(Debug, Clone, Serialize)]
pub struct RemoteEnvironmentDto {
    pub name: String,
    pub wireguard_go_bin: String,
    pub wg_peer_public_key_hex: String,
    pub wg_endpoint: String,
    pub wg_allowed_ips: Vec<String>,
    pub wg_persistent_keepalive: Option<u16>,
    pub wg_listen_port: Option<u16>,
    pub wg_local_ip: String,
    pub wg_peer_ip: String,
    pub ssh_host: String,
    pub ssh_port: u16,
    pub ssh_user: String,
    pub ssh_identity_file: String,
    pub ssh_pinned_host_key: String,
}

impl From<&RemoteEnvironmentConfig> for RemoteEnvironmentDto {
    fn from(c: &RemoteEnvironmentConfig) -> Self {
        Self {
            name: c.name.clone(),
            wireguard_go_bin: c.wireguard_go_bin.clone(),
            wg_peer_public_key_hex: c.wg_peer_public_key_hex.clone(),
            wg_endpoint: c.wg_endpoint.clone(),
            wg_allowed_ips: c.wg_allowed_ips.clone(),
            wg_persistent_keepalive: c.wg_persistent_keepalive,
            wg_listen_port: c.wg_listen_port,
            wg_local_ip: c.wg_local_ip.clone(),
            wg_peer_ip: c.wg_peer_ip.clone(),
            ssh_host: c.ssh_target.host.clone(),
            ssh_port: c.ssh_target.port,
            ssh_user: c.ssh_target.user.clone(),
            ssh_identity_file: c.ssh_target.identity_file.display().to_string(),
            ssh_pinned_host_key: c.ssh_target.pinned_host_key.clone(),
        }
    }
}

/// What `remote_set_environment` accepts - plain strings/numbers the
/// frontend form collects, converted into a [`RemoteEnvironmentConfig`] (and
/// the connector's own [`SshTarget`]) after validation.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RemoteEnvironmentRequest {
    pub name: String,
    pub wireguard_go_bin: String,
    pub wg_peer_public_key_hex: String,
    pub wg_endpoint: String,
    pub wg_allowed_ips: Vec<String>,
    pub wg_persistent_keepalive: Option<u16>,
    pub wg_listen_port: Option<u16>,
    pub wg_local_ip: String,
    pub wg_peer_ip: String,
    pub ssh_host: String,
    pub ssh_port: u16,
    pub ssh_user: String,
    pub ssh_identity_file: String,
    pub ssh_pinned_host_key: String,
}

/// The live WG tunnel's state, on the wire - mirrors [`TunnelState`]
/// field-for-field. `Failed` is DURABLE (see this module's doc comment): it
/// persists in `remote_status` until the next Connect/Disconnect, never
/// silently reverting to `Disconnected`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TunnelStatusDto {
    Disconnected,
    Connecting,
    Connected {
        interface: String,
        latest_handshake_secs: Option<u64>,
    },
    Failed {
        message: String,
    },
}

impl From<&TunnelState> for TunnelStatusDto {
    fn from(s: &TunnelState) -> Self {
        match s {
            TunnelState::Disconnected => TunnelStatusDto::Disconnected,
            TunnelState::Connecting => TunnelStatusDto::Connecting,
            TunnelState::Connected(tunnel) => TunnelStatusDto::Connected {
                interface: tunnel.interface().to_string(),
                latest_handshake_secs: tunnel.latest_handshake_secs(),
            },
            TunnelState::Failed(message) => TunnelStatusDto::Failed {
                message: message.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TailStatusDto {
    pub path: String,
    pub running: bool,
}

/// `remote_ssh_read_file`'s return - the remote descriptor's bytes decoded
/// best-effort. `valid_utf8: false` means `content` is a LOSSY decode
/// (replacement characters may appear in place of invalid bytes) - the
/// frontend must render that honestly, never pretend it is exact.
#[derive(Debug, Clone, Serialize)]
pub struct RemoteFileDto {
    pub content: String,
    pub valid_utf8: bool,
    pub size_bytes: usize,
}

impl RemoteFileDto {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        let size_bytes = bytes.len();
        match String::from_utf8(bytes) {
            Ok(content) => Self {
                content,
                valid_utf8: true,
                size_bytes,
            },
            Err(e) => Self {
                content: String::from_utf8_lossy(e.as_bytes()).into_owned(),
                valid_utf8: false,
                size_bytes,
            },
        }
    }
}

/// Every error a remote command can return - mirrors
/// `crypto::commands::CryptoError`'s shape: `WgError`/`SshError`/
/// `HetznerError` carry no HTTP-style status to preserve either, just a
/// message.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoteError {
    Bootstrapping,
    /// No environment saved yet (`remote_set_environment` was never called,
    /// or resolved to `None`) - a normal state, never surfaced as anything
    /// scarier than "save an environment first".
    NoEnvironment,
    /// A `remote_set_environment` request was missing a required field.
    Invalid {
        message: String,
    },
    /// Neither the operator's saved `wireguard_go_bin` nor the auto-discovered
    /// default resolved to anything - Connect fails honestly rather than
    /// guessing a path (docs/PHASE4.md W4: "absent -> Connect fails
    /// honestly").
    NoWireguardGoBinary,
    Wg {
        message: String,
    },
    Ssh {
        message: String,
    },
    Hetzner {
        message: String,
    },
    /// A task-runner/mutex-poisoning failure internal to this panel - never
    /// a domain (Wg/Ssh/Hetzner) failure, see `poisoned`/`join_failed`.
    Internal {
        message: String,
    },
}

impl From<WgError> for RemoteError {
    fn from(e: WgError) -> Self {
        RemoteError::Wg {
            message: e.to_string(),
        }
    }
}

impl From<SshError> for RemoteError {
    fn from(e: SshError) -> Self {
        RemoteError::Ssh {
            message: e.to_string(),
        }
    }
}

impl From<HetznerError> for RemoteError {
    fn from(e: HetznerError) -> Self {
        RemoteError::Hetzner {
            message: e.to_string(),
        }
    }
}

// ============================================================================
// live-tail events
// ============================================================================

/// Tauri event the frontend `listen()`s for while a tail is running; payload
/// is one [`RemoteTailLine`]. Mirrors `live::LIVE_EVENT`'s role for the Bus
/// Explorer feed, scoped to this panel's own remote-tail stream.
pub const TAIL_LINE_EVENT: &str = "remote:tail-line";
/// Emitted once, when the tail reader loop ends (the child exited, the pipe
/// closed, or `remote_ssh_tail_stop`/a replacing `remote_ssh_tail_start`
/// killed it) - payload is one [`RemoteTailEnded`].
pub const TAIL_ENDED_EVENT: &str = "remote:tail-ended";

#[derive(Debug, Clone, Serialize)]
pub struct RemoteTailLine {
    pub path: String,
    pub line: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteTailEnded {
    pub path: String,
    pub reason: String,
}

// ============================================================================
// helpers
// ============================================================================

fn poisoned(what: &'static str) -> RemoteError {
    RemoteError::Internal {
        message: format!("{what} mutex poisoned"),
    }
}

/// `tauri::async_runtime::spawn_blocking(...).await`'s own error type is
/// `tauri::Error` (it wraps the underlying `JoinError`, see
/// `async_runtime.rs`'s `impl Future for JoinHandle`), not
/// `tokio::task::JoinError` directly - this only ever fires on a task panic.
fn join_failed(what: &'static str, e: tauri::Error) -> RemoteError {
    RemoteError::Internal {
        message: format!("{what} task failed to run: {e}"),
    }
}

/// Resolve the current [`RemoteClient`] out of managed state, or the
/// appropriate [`RemoteError`] when the panel is not ready. Only holds the
/// state lock long enough to clone the (cheap, `Arc`-backed) client out -
/// mirrors `memory::commands::ready_client` exactly.
async fn ready_client(state: &tauri::State<'_, RemoteState>) -> Result<RemoteClient, RemoteError> {
    let guard = state.inner.lock().await;
    match &*guard {
        RemoteInner::Ready(client) => Ok(client.clone()),
        RemoteInner::Bootstrapping => Err(RemoteError::Bootstrapping),
    }
}

/// A required text field, trimmed-empty rejected with an honest, named
/// [`RemoteError::Invalid`] - checked BEFORE anything is saved or pinned, so
/// a blank host/key never silently becomes an unusable saved environment.
fn require(field: &'static str, value: &str) -> Result<(), RemoteError> {
    if value.trim().is_empty() {
        return Err(RemoteError::Invalid {
            message: format!("{field} is required"),
        });
    }
    Ok(())
}

/// Which `wireguard-go` to bring the tunnel up with: the environment's own
/// saved override if non-blank, else the auto-discovered `default_bin` -
/// `None` when neither resolves (docs/PHASE4.md W4: "absent -> Connect fails
/// honestly"). Pure, so the fallback rule is unit-tested without any Tauri
/// state.
fn resolve_wireguard_go_bin(
    env: &RemoteEnvironmentConfig,
    default_bin: Option<&Path>,
) -> Option<PathBuf> {
    let overridden = env.wireguard_go_bin.trim();
    if overridden.is_empty() {
        default_bin.map(Path::to_path_buf)
    } else {
        Some(PathBuf::from(overridden))
    }
}

/// Build the `WgConfig`/`WgInterfaceAddr`/interface-name triple
/// `remote_wg_connect` feeds to `WgTunnel::bring_up` from a resolved
/// environment + the console's private key hex. Factored out so the
/// interface-naming rule (`utun` on macOS, a fixed name on Linux -
/// `crates/connectors/src/wg.rs`'s own doc comment) is unit-tested without
/// any Tauri state.
fn build_wg_bring_up_args(
    env: &RemoteEnvironmentConfig,
    private_key_hex: String,
) -> (WgConfig, WgInterfaceAddr, String) {
    let config = WgConfig {
        private_key_hex,
        listen_port: env.wg_listen_port,
        peers: vec![WgPeer {
            public_key_hex: env.wg_peer_public_key_hex.clone(),
            endpoint: env.wg_endpoint.clone(),
            allowed_ips: env.wg_allowed_ips.clone(),
            persistent_keepalive: env.wg_persistent_keepalive,
        }],
    };
    let addr = WgInterfaceAddr {
        local_ip: env.wg_local_ip.clone(),
        peer_ip: env.wg_peer_ip.clone(),
    };
    // wireguard-go picks the real utunN number on macOS; Linux needs an
    // explicit interface name (`crates/connectors/src/wg.rs`'s module doc).
    let interface = if cfg!(target_os = "macos") {
        "utun".to_string()
    } else {
        "genaryx0".to_string()
    };
    (config, addr, interface)
}

/// Read every cell out of `client` into one renderable [`RemoteStatusDto`].
/// Shared by `remote_status` and every mutating command's own return (so the
/// frontend never needs a second round trip after an action - see this
/// module's doc comment). The tunnel cell's read runs inside
/// `spawn_blocking`: `WgTunnel::latest_handshake_secs` is a real UAPI
/// Unix-socket round trip, not a plain enum match.
async fn build_status(client: &RemoteClient) -> RemoteStatusDto {
    let environment = client
        .environment
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|c| Box::new(RemoteEnvironmentDto::from(c))));
    let console_public_b64 = client
        .console_keypair
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(WgKeypair::public_b64));
    let tail = client.tail.lock().ok().and_then(|g| {
        g.as_ref().map(|s| TailStatusDto {
            path: s.path.clone(),
            running: true,
        })
    });

    let tunnel_arc = client.tunnel.clone();
    let tunnel = tauri::async_runtime::spawn_blocking(move || match tunnel_arc.lock() {
        Ok(guard) => TunnelStatusDto::from(&*guard),
        Err(_) => TunnelStatusDto::Failed {
            message: "tunnel state mutex poisoned".to_string(),
        },
    })
    .await
    .unwrap_or(TunnelStatusDto::Failed {
        message: "status task failed to run".to_string(),
    });

    RemoteStatusDto::Ready {
        default_wireguard_go_bin: client
            .default_wireguard_go_bin
            .as_ref()
            .map(|p| p.display().to_string()),
        environment,
        console_public_b64,
        tunnel,
        tail,
    }
}

/// Kill and clear any in-flight tail (best-effort `kill`/`wait` - the SAME
/// discipline `WgTunnel::drop` follows for its own child). A process this
/// panel itself spawned, never a `ps`/`lsof`-discovered PID
/// ([[process-kill-classifier-restriction]]).
fn stop_tail(tail: &Arc<StdMutex<Option<TailSession>>>) -> Result<(), RemoteError> {
    let mut guard = tail.lock().map_err(|_| poisoned("tail session"))?;
    if let Some(mut session) = guard.take() {
        let _ = session.child.kill();
        let _ = session.child.wait();
    }
    Ok(())
}

async fn stop_tail_async(tail: Arc<StdMutex<Option<TailSession>>>) -> Result<(), RemoteError> {
    tauri::async_runtime::spawn_blocking(move || stop_tail(&tail))
        .await
        .map_err(|e| join_failed("stop tail", e))?
}

/// Store the freshly spawned tail child, then hand its stdout to a dedicated
/// reader thread that emits [`TAIL_LINE_EVENT`] per line and
/// [`TAIL_ENDED_EVENT`] once, on EOF/error - see this module's doc comment.
fn spawn_tail_reader(
    mut child: std::process::Child,
    path: String,
    tail: Arc<StdMutex<Option<TailSession>>>,
    app: tauri::AppHandle,
) {
    let stdout = child.stdout.take();
    {
        // Poisoning here would only follow an earlier panic while this exact
        // cell was held - never happens (every lock site in this module is a
        // short, panic-free critical section) - but fail safe rather than
        // dropping the freshly spawned child silently.
        let mut guard = match tail.lock() {
            Ok(g) => g,
            Err(poison) => poison.into_inner(),
        };
        *guard = Some(TailSession {
            child,
            path: path.clone(),
        });
    }
    let Some(stdout) = stdout else {
        let _ = app.emit(
            TAIL_ENDED_EVENT,
            RemoteTailEnded {
                path,
                reason: "the ssh child had no stdout pipe".to_string(),
            },
        );
        return;
    };
    std::thread::spawn(move || {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(text) => {
                    let _ = app.emit(
                        TAIL_LINE_EVENT,
                        RemoteTailLine {
                            path: path.clone(),
                            line: text,
                        },
                    );
                }
                Err(_) => break,
            }
        }
        let _ = app.emit(
            TAIL_ENDED_EVENT,
            RemoteTailEnded {
                path,
                reason: "the remote tail stream ended".to_string(),
            },
        );
    });
}

/// Run a blocking SSH call off the async executor thread: the lock is
/// acquired INSIDE the blocking closure (an `SshClient` cannot be cloned out
/// like `RemoteClient` can - it owns a private temp pin file, see
/// `ssh.rs`'s `Drop`), mirrors `memory::commands::run_blocking` +
/// `lock_engram` combined for that shape.
async fn run_ssh_blocking<T, F>(
    ssh: Arc<StdMutex<Option<SshClient>>>,
    f: F,
) -> Result<T, RemoteError>
where
    F: FnOnce(&SshClient) -> Result<T, SshError> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let guard = ssh.lock().map_err(|_| poisoned("ssh client"))?;
        let client = guard.as_ref().ok_or(RemoteError::NoEnvironment)?;
        f(client).map_err(RemoteError::from)
    })
    .await
    .map_err(|e| join_failed("ssh", e))?
}

// ============================================================================
// commands
// ============================================================================

/// Whole-panel state. Never fails: every outcome of [`super::state::bootstrap`]
/// is a renderable [`RemoteStatusDto`] variant.
#[tauri::command]
pub async fn remote_status(state: tauri::State<'_, RemoteState>) -> Result<RemoteStatusDto, ()> {
    let guard = state.inner.lock().await;
    match &*guard {
        RemoteInner::Bootstrapping => Ok(RemoteStatusDto::Bootstrapping),
        RemoteInner::Ready(client) => Ok(build_status(client).await),
    }
}

/// Save (or replace) the operator-defined remote environment (docs/PHASE4.md
/// W4 position 2). Validates every required field BEFORE touching any cell;
/// on success, pins the new SSH target immediately (local disk I/O, see
/// `state::pin_ssh_target`'s doc), then resets the SSH client, the tunnel
/// (to `Disconnected`), and any in-flight tail - see this module's doc
/// comment for why `console_keypair` is deliberately left untouched.
#[tauri::command(rename_all = "snake_case")]
pub async fn remote_set_environment(
    request: RemoteEnvironmentRequest,
    state: tauri::State<'_, RemoteState>,
) -> Result<RemoteStatusDto, RemoteError> {
    require("name", &request.name)?;
    require("wg_peer_public_key_hex", &request.wg_peer_public_key_hex)?;
    require("wg_endpoint", &request.wg_endpoint)?;
    require("wg_local_ip", &request.wg_local_ip)?;
    require("wg_peer_ip", &request.wg_peer_ip)?;
    require("ssh_host", &request.ssh_host)?;
    require("ssh_user", &request.ssh_user)?;
    require("ssh_identity_file", &request.ssh_identity_file)?;
    require("ssh_pinned_host_key", &request.ssh_pinned_host_key)?;

    let client = ready_client(&state).await?;

    let ssh_target = SshTarget {
        host: request.ssh_host,
        port: request.ssh_port,
        user: request.ssh_user,
        identity_file: PathBuf::from(&request.ssh_identity_file),
        pinned_host_key: request.ssh_pinned_host_key,
    };

    let config = RemoteEnvironmentConfig {
        name: request.name,
        wireguard_go_bin: request.wireguard_go_bin,
        wg_peer_public_key_hex: request.wg_peer_public_key_hex,
        wg_endpoint: request.wg_endpoint,
        wg_allowed_ips: request.wg_allowed_ips,
        wg_persistent_keepalive: request.wg_persistent_keepalive,
        wg_listen_port: request.wg_listen_port,
        wg_local_ip: request.wg_local_ip,
        wg_peer_ip: request.wg_peer_ip,
        ssh_target: ssh_target.clone(),
    };

    let new_ssh =
        tauri::async_runtime::spawn_blocking(move || super::state::pin_ssh_target(ssh_target))
            .await
            .map_err(|e| join_failed("environment save", e))??;

    // Replace the environment, then reset SSH/tunnel/tail so nothing from a
    // PREVIOUS environment lingers (see state.rs's "fail-closed lifecycle"
    // doc) - dropping the old SshClient/WgTunnel/tail child tears each down.
    {
        let mut guard = client
            .environment
            .lock()
            .map_err(|_| poisoned("environment"))?;
        *guard = Some(config);
    }
    {
        let mut guard = client.ssh.lock().map_err(|_| poisoned("ssh client"))?;
        *guard = Some(new_ssh);
    }
    {
        let mut guard = client.tunnel.lock().map_err(|_| poisoned("tunnel state"))?;
        *guard = TunnelState::Disconnected;
    }
    stop_tail_async(client.tail.clone()).await?;

    Ok(build_status(&client).await)
}

/// `GET /v1/servers[?label_selector=...]` (docs/PHASE4.md W4 position 1),
/// STRICTLY READ-ONLY: `HetznerClient` exposes no create/delete/mutate
/// method at all, so there is no way for this command (or this console) to
/// touch Hetzner infrastructure beyond listing it. Stateless by design (the
/// connector holds no persistent connection, `hetzner.rs`'s own doc comment),
/// so no managed state is needed; the token lives only for this one call's
/// duration. A blank `label_selector` uses the campaign default
/// `managed-by=taipan`.
#[tauri::command(rename_all = "snake_case")]
pub async fn remote_hetzner_list(
    token: String,
    label_selector: Option<String>,
) -> Result<Vec<HetznerServer>, RemoteError> {
    require("token", &token)?;
    let selector = label_selector
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "managed-by=taipan".to_string());
    let client = HetznerClient::new(token).map_err(RemoteError::from)?;
    client
        .list_servers(Some(&selector))
        .await
        .map_err(RemoteError::from)
}

/// Bring the WireGuard tunnel up (docs/PHASE4.md W4 position 3): ensures the
/// console's own WG keypair exists (generating it on first use - see this
/// module's doc comment), builds the `WgConfig`/`WgInterfaceAddr`
/// ([`build_wg_bring_up_args`]), marks the tunnel `Connecting`, then runs the
/// SYNC, blocking `WgTunnel::bring_up` inside `spawn_blocking`.
///
/// LOCALLY (as the operator, no privileged helper), `wireguard-go` cannot
/// create a tun device without root, so `bring_up` is expected to fail here
/// with a privilege error - that is shown HONESTLY as `TunnelStatusDto::Failed`
/// (see this module's doc comment for why that is still `Ok(...)`), never a
/// fabricated `Connected`. The live tunnel is exercised on the Hetzner
/// campaign box, not on a plain dev machine (docs/PHASE4.md W4 "Privilege
/// reality").
#[tauri::command]
pub async fn remote_wg_connect(
    state: tauri::State<'_, RemoteState>,
) -> Result<RemoteStatusDto, RemoteError> {
    let client = ready_client(&state).await?;

    let env = {
        let guard = client
            .environment
            .lock()
            .map_err(|_| poisoned("environment"))?;
        guard.clone().ok_or(RemoteError::NoEnvironment)?
    };

    let bin_path = resolve_wireguard_go_bin(&env, client.default_wireguard_go_bin.as_deref())
        .ok_or(RemoteError::NoWireguardGoBinary)?;

    // Ensure a console WG identity exists - generated ONCE, lazily, and
    // reused across reconnects (see this module's doc comment).
    let private_key_hex = {
        let mut guard = client
            .console_keypair
            .lock()
            .map_err(|_| poisoned("console keypair"))?;
        if guard.is_none() {
            *guard = Some(WgKeypair::generate()?);
        }
        guard.as_ref().expect("just ensured present").private_hex()
    };

    let (config, addr, interface) = build_wg_bring_up_args(&env, private_key_hex);

    // Mark Connecting immediately - honest for a concurrent status poll
    // during what can be a multi-second bring-up (crates/connectors/src/wg.rs:
    // up to 5s waiting for the UAPI socket alone).
    {
        let mut guard = client.tunnel.lock().map_err(|_| poisoned("tunnel state"))?;
        *guard = TunnelState::Connecting;
    }

    let tunnel_arc = client.tunnel.clone();
    let bring_up = tauri::async_runtime::spawn_blocking(move || {
        WgTunnel::bring_up(&bin_path, &interface, &config, &addr)
    })
    .await
    .map_err(|e| join_failed("connect", e))?;

    // Fail-closed, ALWAYS recorded: a failed bring-up is `Failed`, never
    // silently reverted to `Disconnected` and never shown as `Connected`
    // (docs/PHASE4.md W4: "never claim a tunnel is up when it is not").
    {
        let mut guard = tunnel_arc.lock().map_err(|_| poisoned("tunnel state"))?;
        *guard = match bring_up {
            Ok(tunnel) => TunnelState::Connected(tunnel),
            Err(e) => TunnelState::Failed(e.to_string()),
        };
    }

    Ok(build_status(&client).await)
}

/// Tear the tunnel down by dropping it (docs/PHASE4.md W4 position 3: "A
/// 'Disconnect' tears the tunnel down (drop it)") - `WgTunnel::drop` kills
/// its `wireguard-go` child. Always safe, even when already disconnected or
/// in a `Failed` state (both simply become `Disconnected`).
#[tauri::command]
pub async fn remote_wg_disconnect(
    state: tauri::State<'_, RemoteState>,
) -> Result<RemoteStatusDto, RemoteError> {
    let client = ready_client(&state).await?;
    let tunnel_arc = client.tunnel.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = tunnel_arc.lock().map_err(|_| poisoned("tunnel state"))?;
        *guard = TunnelState::Disconnected;
        Ok::<(), RemoteError>(())
    })
    .await
    .map_err(|e| join_failed("disconnect", e))??;
    Ok(build_status(&client).await)
}

/// A reachability + host-key-pin + auth probe (docs/PHASE4.md W4 position 4).
#[tauri::command]
pub async fn remote_ssh_check_reachable(
    state: tauri::State<'_, RemoteState>,
) -> Result<(), RemoteError> {
    let client = ready_client(&state).await?;
    run_ssh_blocking(client.ssh, |ssh| ssh.check_reachable()).await
}

/// Read one remote file's bytes (e.g. a taipan descriptor,
/// `~/.taipan/environments/<name>.json` - docs/PHASE4.md W4 position 4).
#[tauri::command(rename_all = "snake_case")]
pub async fn remote_ssh_read_file(
    path: String,
    state: tauri::State<'_, RemoteState>,
) -> Result<RemoteFileDto, RemoteError> {
    require("path", &path)?;
    let client = ready_client(&state).await?;
    let bytes = run_ssh_blocking(client.ssh, move |ssh| ssh.read_remote_file(&path)).await?;
    Ok(RemoteFileDto::from_bytes(bytes))
}

/// Start (replacing any previous) a streaming remote log tail
/// (docs/PHASE4.md W4 position 4) - lines arrive over [`TAIL_LINE_EVENT`],
/// see this module's doc comment.
#[tauri::command(rename_all = "snake_case")]
pub async fn remote_ssh_tail_start(
    path: String,
    from_offset: u64,
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
) -> Result<RemoteStatusDto, RemoteError> {
    require("path", &path)?;
    let client = ready_client(&state).await?;

    // Replace any previous tail first - never two children racing on the
    // same event stream.
    stop_tail_async(client.tail.clone()).await?;

    let ssh_arc = client.ssh.clone();
    let path_for_spawn = path.clone();
    let child = tauri::async_runtime::spawn_blocking(move || {
        let guard = ssh_arc.lock().map_err(|_| poisoned("ssh client"))?;
        let ssh = guard.as_ref().ok_or(RemoteError::NoEnvironment)?;
        ssh.spawn_tail(&path_for_spawn, from_offset)
            .map_err(RemoteError::from)
    })
    .await
    .map_err(|e| join_failed("tail spawn", e))??;

    spawn_tail_reader(child, path, client.tail.clone(), app);

    Ok(build_status(&client).await)
}

/// Stop the in-flight remote tail, if any (always safe to call).
#[tauri::command]
pub async fn remote_ssh_tail_stop(
    state: tauri::State<'_, RemoteState>,
) -> Result<RemoteStatusDto, RemoteError> {
    let client = ready_client(&state).await?;
    stop_tail_async(client.tail.clone()).await?;
    Ok(build_status(&client).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_env() -> RemoteEnvironmentConfig {
        RemoteEnvironmentConfig {
            name: "hetzner-campaign-1".to_string(),
            wireguard_go_bin: String::new(),
            wg_peer_public_key_hex: "bb".repeat(32),
            wg_endpoint: "203.0.113.9:51820".to_string(),
            wg_allowed_ips: vec!["10.9.0.1/32".to_string()],
            wg_persistent_keepalive: Some(25),
            wg_listen_port: None,
            wg_local_ip: "10.9.0.2".to_string(),
            wg_peer_ip: "10.9.0.1".to_string(),
            ssh_target: SshTarget {
                host: "203.0.113.9".to_string(),
                port: 22,
                user: "root".to_string(),
                identity_file: PathBuf::from("/tmp/genaryx-remote-cmd-test-identity"),
                pinned_host_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIabc".to_string(),
            },
        }
    }

    fn empty_client() -> RemoteClient {
        RemoteClient {
            default_wireguard_go_bin: None,
            environment: Arc::new(StdMutex::new(None)),
            console_keypair: Arc::new(StdMutex::new(None)),
            tunnel: Arc::new(StdMutex::new(TunnelState::Disconnected)),
            ssh: Arc::new(StdMutex::new(None)),
            tail: Arc::new(StdMutex::new(None)),
        }
    }

    // ---- require ----

    #[test]
    fn require_rejects_blank_and_whitespace_only() {
        assert!(require("host", "").is_err());
        assert!(require("host", "   ").is_err());
        assert!(require("host", "203.0.113.7").is_ok());
    }

    // ---- resolve_wireguard_go_bin ----

    #[test]
    fn resolve_wireguard_go_bin_prefers_a_non_blank_environment_override() {
        let mut env = sample_env();
        env.wireguard_go_bin = "/custom/wireguard-go".to_string();
        let resolved = resolve_wireguard_go_bin(&env, Some(Path::new("/default/wireguard-go")));
        assert_eq!(resolved, Some(PathBuf::from("/custom/wireguard-go")));
    }

    #[test]
    fn resolve_wireguard_go_bin_falls_back_to_the_default_when_blank() {
        let env = sample_env(); // wireguard_go_bin left blank by sample_env()
        let resolved = resolve_wireguard_go_bin(&env, Some(Path::new("/default/wireguard-go")));
        assert_eq!(resolved, Some(PathBuf::from("/default/wireguard-go")));
    }

    #[test]
    fn resolve_wireguard_go_bin_is_none_when_both_are_absent() {
        let env = sample_env();
        assert_eq!(resolve_wireguard_go_bin(&env, None), None);
    }

    // ---- build_wg_bring_up_args ----

    #[test]
    fn build_wg_bring_up_args_maps_every_field_and_the_platform_interface() {
        let env = sample_env();
        let (config, addr, interface) = build_wg_bring_up_args(&env, "aa".repeat(32));
        assert_eq!(config.private_key_hex, "aa".repeat(32));
        assert_eq!(config.listen_port, None);
        assert_eq!(config.peers.len(), 1);
        assert_eq!(config.peers[0].public_key_hex, "bb".repeat(32));
        assert_eq!(config.peers[0].endpoint, "203.0.113.9:51820");
        assert_eq!(config.peers[0].allowed_ips, vec!["10.9.0.1/32".to_string()]);
        assert_eq!(config.peers[0].persistent_keepalive, Some(25));
        assert_eq!(addr.local_ip, "10.9.0.2");
        assert_eq!(addr.peer_ip, "10.9.0.1");
        if cfg!(target_os = "macos") {
            assert_eq!(interface, "utun");
        } else {
            assert_eq!(interface, "genaryx0");
        }
    }

    // ---- RemoteEnvironmentDto::from ----

    #[test]
    fn remote_environment_dto_flattens_the_ssh_target() {
        let env = sample_env();
        let dto = RemoteEnvironmentDto::from(&env);
        assert_eq!(dto.name, "hetzner-campaign-1");
        assert_eq!(dto.ssh_host, "203.0.113.9");
        assert_eq!(dto.ssh_port, 22);
        assert_eq!(dto.ssh_user, "root");
        assert_eq!(
            dto.ssh_identity_file,
            "/tmp/genaryx-remote-cmd-test-identity"
        );
        assert_eq!(
            dto.ssh_pinned_host_key,
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIabc"
        );
        assert_eq!(dto.wg_peer_public_key_hex, "bb".repeat(32));
    }

    // ---- TunnelStatusDto::from ----

    #[test]
    fn tunnel_status_dto_maps_disconnected_connecting_and_failed() {
        assert!(matches!(
            TunnelStatusDto::from(&TunnelState::Disconnected),
            TunnelStatusDto::Disconnected
        ));
        assert!(matches!(
            TunnelStatusDto::from(&TunnelState::Connecting),
            TunnelStatusDto::Connecting
        ));
        match TunnelStatusDto::from(&TunnelState::Failed(
            "wireguard-go spawn: eperm".to_string(),
        )) {
            TunnelStatusDto::Failed { message } => {
                assert_eq!(message, "wireguard-go spawn: eperm");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    // ---- RemoteFileDto ----

    #[test]
    fn remote_file_dto_decodes_valid_utf8_honestly() {
        let dto = RemoteFileDto::from_bytes(b"{\"name\":\"p1\"}".to_vec());
        assert!(dto.valid_utf8);
        assert_eq!(dto.content, "{\"name\":\"p1\"}");
        assert_eq!(dto.size_bytes, 13);
    }

    #[test]
    fn remote_file_dto_lossy_decodes_invalid_utf8_and_flags_it() {
        let dto = RemoteFileDto::from_bytes(vec![0xff, 0xfe, b'x']);
        assert!(!dto.valid_utf8);
        assert_eq!(dto.size_bytes, 3);
        assert!(dto.content.contains('x'));
    }

    // ---- error mapping ----

    #[test]
    fn remote_error_from_ssh_error_carries_a_message() {
        // A real SshError from a genuine failed connection attempt - same
        // fixture pattern `crypto::commands`'s tests use for `QryxError`.
        // Deliberately a `.invalid` HOSTNAME (RFC 2606, guaranteed to fail
        // DNS resolution immediately), NOT `sample_env()`'s own
        // "203.0.113.9" (a routable-looking TEST-NET-3 IP): `ssh` would
        // attempt a real TCP connect to that and hang until the OS's
        // connection timeout (60s+) - mirrors `ssh.rs`'s own
        // `run_against_an_unresolvable_host_is_fail_closed`'s identical
        // choice of an unresolvable hostname over an unroutable IP.
        let mut target = sample_env().ssh_target;
        target.host = "genaryx-remote-cmd-test.invalid.nonexistent.example".to_string();
        let client = super::super::state::pin_ssh_target(target).expect("pin");
        let err = client
            .check_reachable()
            .expect_err("an unresolvable test host must not actually succeed");
        let mapped = RemoteError::from(err);
        match mapped {
            RemoteError::Ssh { message } => assert!(!message.is_empty()),
            other => panic!("expected a Ssh-shaped RemoteError, got {other:?}"),
        }
    }

    #[test]
    fn remote_error_from_wg_error_carries_a_message() {
        let err = WgTunnel::bring_up(
            Path::new("/nonexistent/wireguard-go-remote-cmd-test"),
            "utun",
            &WgConfig {
                private_key_hex: "aa".repeat(32),
                listen_port: None,
                peers: vec![],
            },
            &WgInterfaceAddr {
                local_ip: "10.9.0.2".to_string(),
                peer_ip: "10.9.0.1".to_string(),
            },
        )
        .expect_err("a nonexistent wireguard-go binary must fail to spawn");
        let mapped = RemoteError::from(err);
        match mapped {
            RemoteError::Wg { message } => assert!(!message.is_empty()),
            other => panic!("expected a Wg-shaped RemoteError, got {other:?}"),
        }
    }

    #[test]
    fn remote_error_from_hetzner_error_carries_a_message() {
        // A 4xx/5xx from a bogus base URL is enough to exercise the mapping
        // without a real network dependency in CI - `hetzner.rs` itself only
        // unit-tests offline JSON parsing, so this stays consistent with
        // that connector's own "no live external API in tests" discipline
        // by not actually awaiting a real call here.
        let err = HetznerError::Api {
            status: 401,
            body: "unauthorized".to_string(),
        };
        let mapped = RemoteError::from(err);
        match mapped {
            RemoteError::Hetzner { message } => {
                assert!(message.contains("401"));
            }
            other => panic!("expected a Hetzner-shaped RemoteError, got {other:?}"),
        }
    }

    // ---- stop_tail ----

    #[test]
    fn stop_tail_on_an_empty_cell_is_a_harmless_no_op() {
        let tail: Arc<StdMutex<Option<TailSession>>> = Arc::new(StdMutex::new(None));
        assert!(stop_tail(&tail).is_ok());
    }

    #[test]
    fn stop_tail_kills_a_real_child_and_clears_the_cell() {
        let child = std::process::Command::new("sleep")
            .arg("30")
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("spawn a real long-lived child");
        let tail: Arc<StdMutex<Option<TailSession>>> = Arc::new(StdMutex::new(Some(TailSession {
            child,
            path: "/tmp/fixture.log".to_string(),
        })));
        assert!(stop_tail(&tail).is_ok());
        assert!(tail.lock().unwrap().is_none());
    }

    // ---- build_status ----

    #[tokio::test]
    async fn build_status_on_a_fresh_client_is_ready_with_nothing_configured() {
        let client = empty_client();
        match build_status(&client).await {
            RemoteStatusDto::Ready {
                default_wireguard_go_bin,
                environment,
                console_public_b64,
                tunnel,
                tail,
            } => {
                assert!(default_wireguard_go_bin.is_none());
                assert!(environment.is_none());
                assert!(console_public_b64.is_none());
                assert!(matches!(tunnel, TunnelStatusDto::Disconnected));
                assert!(tail.is_none());
            }
            RemoteStatusDto::Bootstrapping => panic!("build_status never returns Bootstrapping"),
        }
    }

    #[tokio::test]
    async fn build_status_reflects_a_saved_environment_and_generated_keypair() {
        let client = empty_client();
        *client.environment.lock().unwrap() = Some(sample_env());
        *client.console_keypair.lock().unwrap() = Some(WgKeypair::generate().expect("keygen"));
        *client.tunnel.lock().unwrap() =
            TunnelState::Failed("wireguard-go spawn: eperm".to_string());

        match build_status(&client).await {
            RemoteStatusDto::Ready {
                environment,
                console_public_b64,
                tunnel,
                ..
            } => {
                assert_eq!(
                    environment.expect("environment should be Some").name,
                    "hetzner-campaign-1"
                );
                assert!(console_public_b64.is_some());
                match tunnel {
                    TunnelStatusDto::Failed { message } => {
                        assert_eq!(message, "wireguard-go spawn: eperm");
                    }
                    other => panic!("expected Failed, got {other:?}"),
                }
            }
            RemoteStatusDto::Bootstrapping => panic!("build_status never returns Bootstrapping"),
        }
    }

    // ---- ready_client (via a hand-built RemoteState, no Tauri app needed) ----

    #[tokio::test]
    async fn ready_client_reports_bootstrapping_before_bootstrap_resolves() {
        let state = RemoteState::pending();
        let guard = state.inner.lock().await;
        assert!(matches!(&*guard, RemoteInner::Bootstrapping));
    }
}
