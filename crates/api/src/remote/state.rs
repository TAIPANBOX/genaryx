//! Remote-panel console-managed state: the operator-defined remote environment
//! (WG peer + SSH target + `wireguard-go` binary path), the console's own WG
//! identity, and the two LONG-LIVED connections this panel owns for the
//! app's whole lifetime once created - a [`WgTunnel`] and an [`SshClient`]
//! (docs/PHASE4.md W4: "the WG tunnel + SSH client are LONG-LIVED ... like
//! the Memory panel holds its long-lived EngramClient").
//!
//! Unlike every other panel, there is nothing to auto-discover here beyond a
//! best-effort `wireguard-go` DEFAULT (see `env.rs`): the WG peer and SSH
//! target are 100% operator-supplied (docs/PHASE4.md W4 v1 scope position
//! 2), so `bootstrap` never blocks on I/O and there is no `NoEnvironment`/
//! `Unreachable` distinction to make at startup - [`RemoteInner`] has exactly
//! one settled shape, `Ready`, whose `environment` cell is simply `None`
//! until the operator saves one (mirrors `evidence::state`'s "no single
//! Ready/NoEnvironment gate" rationale: Hetzner/WG/SSH are three independent
//! capabilities, not one all-or-nothing plane).
//!
//! ## Interior mutability, not outer-state replacement
//!
//! Every mutable piece (`environment`, `console_keypair`, `tunnel`, `ssh`,
//! `tail`) lives behind its OWN `Arc<std::sync::Mutex<_>>` cell inside one
//! [`RemoteClient`] value that is itself cloned out of the outer
//! `tokio::sync::Mutex<RemoteInner>` exactly once, at bootstrap (mirrors
//! `memory::state`'s `Arc<Mutex<EngramClient>>` pattern, generalized to more
//! than one cell). So `remote::commands::ready_client` clones a handful of
//! `Arc`s once per command call; every mutation (saving an environment,
//! connecting/disconnecting the tunnel, starting/stopping an SSH tail) locks
//! and replaces the CONTENTS of the relevant cell directly, never the outer
//! `RemoteInner` - the outer lock is only ever touched again by
//! `remote_status`'s initial `Bootstrapping`-vs-`Ready` read.
//!
//! `WgTunnel`/`SshClient` methods do blocking I/O (a UAPI Unix-socket round
//! trip, a subprocess spawn+wait) - every call into either happens inside
//! `tokio::task::spawn_blocking`, mirroring `memory::commands`'s identical
//! discipline for `EngramClient`, and each cell is a plain `std::sync::Mutex`
//! (not `tokio::sync::Mutex`) for the same reason `memory::state`'s module
//! doc gives: the lock is only ever taken from a blocking OS thread, never
//! held across an `.await`.
//!
//! ## Fail-closed lifecycle
//!
//! Replacing a cell's contents (`*guard = TunnelState::Disconnected` over a
//! `Connected(WgTunnel)`, or a fresh `SshClient::connect` over an existing
//! one on `remote_set_environment`) drops the OLD value in place - `WgTunnel`
//! kills its `wireguard-go` child and `SshClient` removes its pinned temp
//! `known_hosts` (both connectors' own `Drop` impls,
//! `crates/connectors/src/{wg,ssh}.rs`). So Disconnect and "save a new
//! environment over an old one" both tear down whatever was live before, by
//! construction - there is no code path that leaks a previous tunnel or
//! leaves a stale pin file behind.

use super::env;
use genaryx_connectors::{SshClient, SshError, SshTarget, WgKeypair, WgTunnel};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex as AsyncMutex;

/// The WG peer + SSH target + `wireguard-go` binary path the operator has
/// defined for one remote campaign (docs/PHASE4.md W4 v1 scope position 2).
/// Plain data, cheap to clone; `ssh_target` reuses the connector's own
/// [`SshTarget`] directly rather than duplicating its fields.
#[derive(Debug, Clone)]
pub struct RemoteEnvironmentConfig {
    pub name: String,
    /// Resolved path to `wireguard-go` FOR THIS ENVIRONMENT - the operator's
    /// own override, pre-filled from [`env::discover`] but saved verbatim;
    /// blank falls back to that default at Connect time (see
    /// `commands::resolve_wireguard_go_bin`).
    pub wireguard_go_bin: String,
    pub wg_peer_public_key_hex: String,
    /// `host:port`.
    pub wg_endpoint: String,
    pub wg_allowed_ips: Vec<String>,
    pub wg_persistent_keepalive: Option<u16>,
    pub wg_listen_port: Option<u16>,
    pub wg_local_ip: String,
    pub wg_peer_ip: String,
    pub ssh_target: SshTarget,
}

/// The live WG tunnel's state - see this module's doc comment for why this
/// lives behind its own cell rather than a top-level [`RemoteInner`]
/// variant. Fail-closed by construction: [`Self::Connected`] is the ONLY
/// variant that means the Cloud is reachable, and a half-built tunnel never
/// reaches it (`WgTunnel::bring_up` itself tears down and returns `Err` on
/// any partial failure, `crates/connectors/src/wg.rs`).
pub enum TunnelState {
    Disconnected,
    Connecting,
    Connected(WgTunnel),
    /// The exact `WgError` message from a failed `bring_up` - shown
    /// honestly, DURABLY (survives until the next Connect/Disconnect), never
    /// silently reverted to `Disconnected` (docs/PHASE4.md W4: "a failed WG
    /// bring-up is FAILED, never fake-connected").
    Failed(String),
}

/// One in-flight remote log tail: the owned `ssh ... tail -F` child (killed
/// on stop/replace) plus the remote path it is following, for status
/// display.
pub struct TailSession {
    pub child: std::process::Child,
    pub path: String,
}

/// A live Remote-panel connection: every mutable piece behind its own cell -
/// see this module's doc comment. Cheap to clone (an `Option<PathBuf>` plus
/// five `Arc`s).
#[derive(Clone)]
pub struct RemoteClient {
    /// Best-effort `wireguard-go` default from [`env::discover`], surfaced to
    /// the UI to pre-fill the environment form - never an authority itself
    /// (see [`RemoteEnvironmentConfig::wireguard_go_bin`]'s doc comment).
    pub default_wireguard_go_bin: Option<PathBuf>,
    pub environment: Arc<StdMutex<Option<RemoteEnvironmentConfig>>>,
    /// The console's own WG identity - generated once, lazily, on the FIRST
    /// `remote_wg_connect` call (docs/PHASE4.md W4 position 3), then reused
    /// across reconnects for the rest of the app's life so a public key the
    /// operator already handed to the box admin never silently goes stale.
    /// Never cleared by `remote_set_environment` (the console's identity is
    /// independent of which box it is dialing - see `commands.rs`'s module
    /// doc).
    pub console_keypair: Arc<StdMutex<Option<WgKeypair>>>,
    pub tunnel: Arc<StdMutex<TunnelState>>,
    pub ssh: Arc<StdMutex<Option<SshClient>>>,
    pub tail: Arc<StdMutex<Option<TailSession>>>,
}

/// The Remote panel's whole state machine. Always `Ready` past bootstrap -
/// see this module's doc comment for why there is no `NoEnvironment`/
/// `Unreachable` shape here.
pub enum RemoteInner {
    /// The initial state from [`RemoteState::pending`], until the
    /// background [`bootstrap`] task resolves.
    Bootstrapping,
    Ready(RemoteClient),
}

/// Console-managed state wrapping [`RemoteInner`] in an async mutex, mirroring
/// every other panel's identical shape.
pub struct RemoteState {
    pub inner: AsyncMutex<RemoteInner>,
}

impl RemoteState {
    /// The synchronous, immediately-manageable starting state - `setup`
    /// calls this directly, then spawns [`bootstrap`] in the background,
    /// mirroring every other panel exactly even though (like Crypto/Drills)
    /// today's `bootstrap` body never actually awaits anything either.
    #[must_use]
    pub fn pending() -> Self {
        Self {
            inner: AsyncMutex::new(RemoteInner::Bootstrapping),
        }
    }
}

/// Resolve the `wireguard-go` default and start with a clean, unconfigured
/// panel - no environment, no console identity yet, tunnel disconnected, no
/// SSH client, no tail. Never panics.
pub async fn bootstrap() -> RemoteInner {
    RemoteInner::Ready(RemoteClient {
        default_wireguard_go_bin: env::discover(),
        environment: Arc::new(StdMutex::new(None)),
        console_keypair: Arc::new(StdMutex::new(None)),
        tunnel: Arc::new(StdMutex::new(TunnelState::Disconnected)),
        ssh: Arc::new(StdMutex::new(None)),
        tail: Arc::new(StdMutex::new(None)),
    })
}

/// Construct the pinned [`SshClient`] for `target` - a thin, testable name
/// for the one fallible step `remote_set_environment` performs (writing a
/// pinned temp `known_hosts` is local disk I/O, never network - see
/// `crates/connectors/src/ssh.rs`'s own doc comment for why `connect` itself
/// does not touch the network).
pub fn pin_ssh_target(target: SshTarget) -> Result<SshClient, SshError> {
    SshClient::connect(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_starts_in_the_bootstrapping_state() {
        let state = RemoteState::pending();
        let guard = state
            .inner
            .try_lock()
            .expect("uncontended right after construction");
        assert!(matches!(&*guard, RemoteInner::Bootstrapping));
    }

    #[tokio::test]
    async fn bootstrap_starts_ready_with_nothing_configured_yet() {
        let inner = bootstrap().await;
        match inner {
            RemoteInner::Ready(client) => {
                assert!(client.environment.lock().unwrap().is_none());
                assert!(client.console_keypair.lock().unwrap().is_none());
                assert!(matches!(
                    &*client.tunnel.lock().unwrap(),
                    TunnelState::Disconnected
                ));
                assert!(client.ssh.lock().unwrap().is_none());
                assert!(client.tail.lock().unwrap().is_none());
            }
            RemoteInner::Bootstrapping => {
                panic!("bootstrap must resolve past its own pending state")
            }
        }
    }

    #[test]
    fn remote_client_is_cheap_to_clone_and_shares_the_same_cells() {
        // Proves the `Arc` cells are genuinely SHARED across a clone (the
        // whole point of the interior-mutability design this module's doc
        // comment describes) - a mutation through one clone must be visible
        // through another, the same way `memory::state`'s `Arc<Mutex<...>>`
        // is shared across every `ready_client()` call.
        let a = RemoteClient {
            default_wireguard_go_bin: None,
            environment: Arc::new(StdMutex::new(None)),
            console_keypair: Arc::new(StdMutex::new(None)),
            tunnel: Arc::new(StdMutex::new(TunnelState::Disconnected)),
            ssh: Arc::new(StdMutex::new(None)),
            tail: Arc::new(StdMutex::new(None)),
        };
        let b = a.clone();
        *a.tunnel.lock().unwrap() = TunnelState::Connecting;
        assert!(matches!(
            &*b.tunnel.lock().unwrap(),
            TunnelState::Connecting
        ));
    }

    #[test]
    fn pin_ssh_target_writes_a_pinned_known_hosts_for_a_well_formed_target() {
        let target = SshTarget {
            host: "203.0.113.7".into(),
            port: 2222,
            user: "root".into(),
            identity_file: PathBuf::from("/tmp/genaryx-remote-state-test-identity"),
            pinned_host_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIabc".into(),
        };
        // Only proves the pin write succeeds and yields a usable client -
        // the fail-closed NETWORK path (a mismatched/unreachable host) is
        // `ssh.rs`'s own test (`run_against_an_unresolvable_host_is_fail_closed`).
        let client = pin_ssh_target(target).expect("pin write must succeed");
        drop(client);
    }
}
