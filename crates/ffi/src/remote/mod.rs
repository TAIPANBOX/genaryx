//! `RemoteHandle`: the UniFFI Object wrapping
//! `genaryx_connectors::{HetznerClient, WgTunnel, SshClient}` for the SwiftUI
//! Remote (Distance) surface (docs/PHASE4.md W4, decision D11). Manages the
//! TRANSPORT to a client-hosted stack, not the stack itself: Hetzner
//! read-only inventory, the WireGuard tunnel (D11's PRIMARY console<->Cloud
//! channel), and host-key-pinned SSH ops (the SECONDARY, ops-focused
//! transport). No other plane re-points through this handle - the full
//! "every plane reaches the remote Cloud through the tunnel" wiring is the
//! LIVE Hetzner exit-gate campaign, out of this v1's scope.
//!
//! ## No `taipan up`-style environment to discover
//!
//! Every OTHER discovery-based handle in this crate resolves ONE environment
//! at construction (`CloudHandle::discover`'s `taipan up` descriptor,
//! `IdryxHandle::discover`'s `IDRYX_URL`, `MemoryHandle::discover`'s
//! `engram-mcp`...). Remote has no such single thing: a Hetzner token, a WG
//! peer config, and an SSH target are each independently operator-entered
//! per docs/PHASE4.md W4's own field list - none of them come from a
//! descriptor. So [`RemoteHandle::new`] does not discover/connect/pair
//! anything; it only starts the small owned async runtime Hetzner reads need
//! (see below) and can fail only on a genuine local resource problem. See
//! `env`'s own module doc for the "operator can see/set it, never enforced"
//! pre-filled defaults this handle exposes instead of a `source()`/
//! `*EnvSource` pair.
//!
//! ## Async-to-sync: one `tokio::runtime::Runtime`, for Hetzner only
//!
//! `HetznerClient::list_servers` is `async` (it wraps `reqwest`), so this
//! handle owns one multi-thread `tokio::runtime::Runtime` and
//! `block_on`s it per call - the same bridge `CloudHandle`/`IdryxHandle` use
//! for their own async connectors (see those types' own module docs). The
//! WireGuard tunnel and every SSH op are already synchronous/blocking on the
//! connector side (`WgTunnel::bring_up`, `SshClient`'s methods all shell a
//! subprocess and block), so the runtime exists ONLY for Hetzner - unlike
//! `CloudHandle`, there is no `block_on` anywhere in the WireGuard/SSH
//! methods below.
//!
//! ## The WireGuard keypair: generated once, held server-side, never re-crosses
//!
//! [`RemoteHandle::wg_generate_keypair`] generates a fresh session keypair and
//! stores it behind `Mutex<Option<WgKeypair>>`, returning ONLY the public
//! half ([`dto::WgKeypairRecord`]) for the operator to hand the box admin.
//! [`RemoteHandle::connect_tunnel`] then consumes the HELD keypair directly -
//! the private half never crosses the FFI boundary at all, in either
//! direction. This is a deliberate security choice (docs/PHASE4.md's own
//! review discipline: "anything touching keys is hand-written by the
//! orchestrator, never delegated"), consistent with how every OTHER secret
//! this codebase generates (`CloudHandle`'s paired `SoftwareSigner`,
//! `SshClient`'s pinned host key) also never round-trips its private half
//! through a caller. Calling `wg_generate_keypair` again before connecting
//! simply replaces the held keypair - there is never more than one "pending"
//! console identity at a time.
//!
//! ## The WireGuard tunnel: `Mutex<Option<WgTunnel>>`, Drop-closed, fail-closed
//!
//! [`RemoteHandle::connect_tunnel`] holds the live [`WgTunnel`] (when one is
//! up) behind `Mutex<Option<WgTunnel>>` - the same interior-mutability shape
//! [`crate::memory::MemoryHandle`] uses for its own long-lived
//! `EngramClient`. A fresh `connect_tunnel` call always tears down whatever
//! this handle already held FIRST (so at most one tunnel is ever live under
//! one handle), then attempts a new `WgTunnel::bring_up`.
//! [`RemoteHandle::disconnect_tunnel`] just clears the `Mutex` - `WgTunnel`'s
//! own `Drop` (`crates/connectors/src/wg.rs`) kills the `wireguard-go` child
//! and removes its scratch tun-name file, so a tunnel never outlives being
//! explicitly disconnected OR this handle itself being dropped.
//!
//! `connect_tunnel`/`tunnel_status` return [`dto::WgStatusRecord`] directly,
//! never `Result` - see that type's own module doc for why EVERY bring-up
//! failure (privilege, spawn, timeout, rejected config) is an honest
//! `Failed { reason }` verdict, never a thrown [`dto::RemoteError`]. This is
//! where docs/PHASE4.md W4's "Privilege reality" note becomes concrete
//! behavior: `wireguard-go` needs root to create a tun device, so a LOCAL
//! `connect_tunnel` call on this box, run un-elevated, is EXPECTED to return
//! `Failed { reason: "wireguard-go spawn: ..." }` - that is the correct,
//! honest v1 outcome, not a bug to work around. This handle adds no
//! sudo/helper escalation of any kind (a later packaging task; production
//! ships a privileged helper) - see docs/PHASE4.md W4's own "do NOT add a
//! sudo/helper flow" instruction.
//!
//! ## SSH: a fresh [`SshClient`] per call, and one lifecycle subtlety
//!
//! Every SSH method here builds a fresh `SshClient` per call (it is cheap -
//! just a pinned temp `known_hosts` file - and the operator may edit the
//! target between calls), mirroring `CryptoHandle`/`DrillsHandle`'s own
//! "re-run the underlying tool fresh every call, nothing to cache" choice.
//! [`SshClient::connect`] writes a PRIVATE temp `known_hosts` file that its
//! own `Drop` removes (`crates/connectors/src/ssh.rs`). For `ssh_check`/
//! `ssh_read_descriptor` (both wrap a BLOCKING `Command::output()` call under
//! the hood - `SshClient::run`), chaining
//! `SshClient::connect(target)?.check_reachable()` is safe: the whole
//! synchronous call completes before the temporary `SshClient` drops at the
//! end of the statement. [`RemoteHandle::ssh_tail_once`] is different -
//! `SshClient::spawn_tail` hands back a LIVE child immediately - so that
//! method explicitly BINDS its `SshClient` to a local variable and keeps it
//! alive for the child's entire probe window; dropping it any earlier would
//! race-delete the pin file while `ssh` might still be mid-handshake. See
//! that method's own doc for the full explanation.
//!
//! Fail-closed at the boundary (06 §0.5): nothing here panics across FFI.

pub mod dto;
pub mod env;

pub use dto::{
    ConnectTunnelInputs, HetznerServerRecord, LabelEntry, RemoteError, SshTargetRecord,
    WgKeypairRecord, WgStatusRecord,
};

use genaryx_connectors::{
    HetznerClient, SshClient, SshTarget, WgConfig, WgInterfaceAddr, WgKeypair, WgPeer, WgTunnel,
};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Bounded window [`RemoteHandle::ssh_tail_once`] waits for new bytes before
/// returning - long enough to catch a burst of freshly-tailed log lines,
/// short enough that the FFI call itself always returns promptly (never an
/// indefinite block - 06 §0.5). Also long enough to let a DNS/connect
/// failure surface as a real nonzero exit within the window on the common
/// case (see that method's own doc).
const TAIL_POLL_WINDOW: Duration = Duration::from_millis(1000);

/// Lock a poisoned-or-not mutex without panicking: a poisoned guard only
/// means some other call died mid-hold, and both values this handle guards
/// (an `Option<WgKeypair>`, an `Option<WgTunnel>`) stay perfectly usable in
/// that case. Mirrors `crate::relock`/`crate::memory::relock`'s own
/// independent copies (this crate's established "independent evolution over
/// a shared cross-module abstraction" choice - see `crate::wardryx::env`'s
/// own doc for the same rationale).
fn relock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The Remote (Distance) UniFFI Object. See the module doc for the async
/// bridge, the keypair-hygiene rule, and the tunnel's Drop-closed lifecycle.
#[derive(uniffi::Object)]
pub struct RemoteHandle {
    /// Hetzner reads are async - `block_on` per call. WireGuard bring-up and
    /// every SSH op are already synchronous on the connector side, so this
    /// runtime exists ONLY for Hetzner (see the module doc).
    runtime: tokio::runtime::Runtime,
    /// The console's own WG session keypair, generated by
    /// [`Self::wg_generate_keypair`] and consumed by [`Self::connect_tunnel`].
    /// `None` until the operator generates one this session.
    keypair: Mutex<Option<WgKeypair>>,
    /// The live tunnel, if one is up. `None` means disconnected - the ONLY
    /// state [`Self::tunnel_status`] needs to distinguish `Connected` from
    /// `Disconnected`; a bring-up that FAILED never reaches this field at all
    /// (see [`Self::connect_tunnel`]).
    tunnel: Mutex<Option<WgTunnel>>,
}

#[uniffi::export]
impl RemoteHandle {
    /// Build the handle: start the small owned async runtime Hetzner reads
    /// need. Touches no network, spawns no subprocess, and resolves no
    /// environment (see the module doc's "no `taipan up`-style environment to
    /// discover") - this can only fail on a genuine local resource problem.
    #[uniffi::constructor]
    pub fn new() -> Result<Self, RemoteError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| RemoteError::Runtime {
                reason: e.to_string(),
            })?;
        Ok(Self {
            runtime,
            keypair: Mutex::new(None),
            tunnel: Mutex::new(None),
        })
    }

    // ---- pre-filled defaults (operator can see/set them, never enforced -
    // see `env`'s own module doc and `crate::crypto::default_scan_target`'s
    // precedent) ----------------------------------------------------------

    pub fn default_wireguard_go_bin(&self) -> Option<String> {
        env::default_wireguard_go_bin().map(|p| p.display().to_string())
    }

    pub fn default_interface(&self) -> String {
        env::default_interface().to_string()
    }

    pub fn default_hetzner_label_selector(&self) -> String {
        env::default_hetzner_label_selector().to_string()
    }

    pub fn default_hetzner_token(&self) -> Option<String> {
        env::default_hetzner_token()
    }

    pub fn default_tunnel_local_ip(&self) -> String {
        env::default_tunnel_local_ip().to_string()
    }

    pub fn default_tunnel_peer_ip(&self) -> String {
        env::default_tunnel_peer_ip().to_string()
    }

    pub fn default_persistent_keepalive(&self) -> Option<u16> {
        env::default_persistent_keepalive()
    }

    pub fn default_ssh_port(&self) -> u16 {
        env::default_ssh_port()
    }

    // ---- Hetzner (read-only inventory) ------------------------------------

    /// `GET /v1/servers[?label_selector=...]` through a fresh, throwaway
    /// `HetznerClient` - the token is taken PER CALL, never stored on this
    /// handle (an operator-pasted credential lives only as long as this one
    /// call's stack frame). Read-only by construction on the connector side
    /// (`crates/connectors/src/hetzner.rs`'s own module doc: "there is no
    /// POST/PUT/DELETE method on this type at all") - this method can only
    /// ever list, never mutate or delete a box.
    pub fn list_hetzner(
        &self,
        token: String,
        label_selector: Option<String>,
    ) -> Result<Vec<HetznerServerRecord>, RemoteError> {
        let client = HetznerClient::new(token)?;
        let servers = self
            .runtime
            .block_on(client.list_servers(label_selector.as_deref()))?;
        Ok(servers.iter().map(HetznerServerRecord::from).collect())
    }

    // ---- WireGuard tunnel (D11: the primary console<->Cloud channel) ------

    /// Generate a fresh console session keypair (Curve25519, OS CSPRNG) and
    /// hold it for the next [`Self::connect_tunnel`] call; returns only the
    /// PUBLIC half for the operator to hand the box admin (paste into the
    /// box's WG peer config). See the module doc's "the WireGuard keypair"
    /// section.
    pub fn wg_generate_keypair(&self) -> Result<WgKeypairRecord, RemoteError> {
        let kp = WgKeypair::generate().map_err(|e| RemoteError::WgKeyGen {
            reason: e.to_string(),
        })?;
        let record = WgKeypairRecord {
            public_b64: kp.public_b64(),
            public_hex: kp.public_hex(),
        };
        *relock(&self.keypair) = Some(kp);
        Ok(record)
    }

    /// Bring the tunnel up against `inputs`' peer config, using the keypair
    /// [`Self::wg_generate_keypair`] already generated. ALWAYS returns a
    /// verdict, never throws for a bring-up failure - see
    /// [`WgStatusRecord`]'s own module doc, and this handle's own module doc
    /// on "Privilege reality". A fresh call always supersedes whatever
    /// tunnel this handle already held (torn down first, before the new
    /// bring-up is attempted).
    pub fn connect_tunnel(&self, inputs: ConnectTunnelInputs) -> WgStatusRecord {
        if let Err(reason) = validate_connect_inputs(&inputs) {
            return WgStatusRecord::Failed { reason };
        }
        let Some(kp) = relock(&self.keypair).clone() else {
            return WgStatusRecord::Failed {
                reason: "no console keypair generated yet - call wgGenerateKeypair() first"
                    .to_string(),
            };
        };

        let config = WgConfig {
            private_key_hex: kp.private_hex(),
            listen_port: inputs.listen_port,
            peers: vec![WgPeer {
                public_key_hex: inputs.peer_public_key_hex,
                endpoint: inputs.endpoint,
                allowed_ips: inputs.allowed_ips,
                persistent_keepalive: inputs.persistent_keepalive,
            }],
        };
        let addr = WgInterfaceAddr {
            local_ip: inputs.local_ip,
            peer_ip: inputs.peer_ip,
        };

        // A fresh Connect always supersedes whatever this handle was already
        // holding: tear it down FIRST (Drop kills the old wireguard-go child
        // and removes its tun-name scratch file) so two tunnels/interfaces
        // are never stacked under one handle.
        *relock(&self.tunnel) = None;

        match WgTunnel::bring_up(
            Path::new(&inputs.wireguard_go_bin),
            &inputs.interface,
            &config,
            &addr,
        ) {
            Ok(tunnel) => {
                let interface = tunnel.interface().to_string();
                let handshake_secs_ago = tunnel.latest_handshake_secs();
                *relock(&self.tunnel) = Some(tunnel);
                WgStatusRecord::Connected {
                    interface,
                    handshake_secs_ago,
                }
            }
            // EVERY WgError (privilege/spawn failure, UAPI timeout, peer
            // config rejected, address/route failed) folds into ONE honest
            // verdict - see WgStatusRecord's own module doc. `bring_up`
            // itself already tore down any half-built child before
            // returning Err, so there is nothing left here to clean up.
            Err(e) => WgStatusRecord::Failed {
                reason: e.to_string(),
            },
        }
    }

    /// Re-derive the tunnel's current status from the live UAPI (fresh
    /// `latest_handshake_secs` every call) - for a Swift-side poll loop that
    /// keeps a "handshake Ns ago" badge honestly up to date without a second
    /// `connect_tunnel` attempt.
    pub fn tunnel_status(&self) -> WgStatusRecord {
        match relock(&self.tunnel).as_ref() {
            None => WgStatusRecord::Disconnected,
            Some(t) => WgStatusRecord::Connected {
                interface: t.interface().to_string(),
                handshake_secs_ago: t.latest_handshake_secs(),
            },
        }
    }

    /// Tear the tunnel down (kills the `wireguard-go` child, removes its
    /// scratch tun-name file - `WgTunnel`'s own `Drop`). A safe no-op, not an
    /// error, when nothing is connected.
    pub fn disconnect_tunnel(&self) {
        *relock(&self.tunnel) = None;
    }

    // ---- SSH ops (secondary to WireGuard - D11) ---------------------------

    /// A reachability + host-key-pin + auth probe. `Ok(())` means the pinned
    /// box authenticated the operator's identity file; an `Err` distinguishes
    /// a pin/auth failure from an unreachable host (via its message).
    pub fn ssh_check(&self, target: SshTargetRecord) -> Result<(), RemoteError> {
        let target = ssh_target_from(target)?;
        SshClient::connect(target)?.check_reachable()?;
        Ok(())
    }

    /// Read one remote file's bytes (the taipan environment descriptor, or
    /// any other path the operator names), host-key-pinned.
    pub fn ssh_read_descriptor(
        &self,
        target: SshTargetRecord,
        path: String,
    ) -> Result<Vec<u8>, RemoteError> {
        let target = ssh_target_from(target)?;
        let bytes = SshClient::connect(target)?.read_remote_file(&path)?;
        Ok(bytes)
    }

    /// One bounded poll of a remote file's tail, starting at `from_offset`
    /// (`tail -c +offset+1 -F` under the hood, host-key-pinned): collects
    /// whatever bytes arrive within a short fixed window, then tears the
    /// probe down and returns them.
    ///
    /// This is deliberately NOT a live-streaming tail:
    /// `SshClient::spawn_tail` hands back a live, indefinitely-running
    /// child, and wiring THAT across UniFFI would need a new push-callback
    /// channel (like [`crate::EventListener`]) plus a background thread per
    /// tail session - a bigger design the v1 scope's own SSH-ops list (check
    /// / read descriptor / tail) does not call for. Instead the Swift side
    /// calls this repeatedly (a manual refresh, or its own short poll loop),
    /// each time passing `from_offset + <bytes it received last time>`,
    /// which reproduces a "tail" experience at the UI layer without a
    /// persistent Rust-side stream. An empty `Vec` (the child still running,
    /// nothing new arrived within the window) is a normal outcome, never an
    /// error - but a child that has ALREADY EXITED by the time the window
    /// closes means the ssh connection itself failed before ever reaching
    /// the tail (DNS, refused, auth, host-key mismatch), which this method
    /// surfaces as a real [`RemoteError::SshRemote`] with ssh's own stderr,
    /// never silently folded into an empty "nothing new" poll.
    pub fn ssh_tail_once(
        &self,
        target: SshTargetRecord,
        path: String,
        from_offset: u64,
    ) -> Result<Vec<u8>, RemoteError> {
        let target = ssh_target_from(target)?;
        // `client` MUST outlive the spawned child below - see this crate's
        // module doc ("SSH: a fresh SshClient per call, and one lifecycle
        // subtlety") for why dropping it any earlier would race-delete the
        // pinned known_hosts file a live ssh child may still be reading.
        let client = SshClient::connect(target)?;
        let mut child = client.spawn_tail(&path, from_offset)?;
        let data = drain_stdout_for(&mut child, TAIL_POLL_WINDOW);

        match child.try_wait() {
            Ok(Some(status)) if !status.success() => {
                let mut stderr_bytes = Vec::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = stderr.read_to_end(&mut stderr_bytes);
                }
                let _ = child.wait();
                Err(RemoteError::SshRemote {
                    code: status.code().unwrap_or(-1),
                    stderr: String::from_utf8_lossy(&stderr_bytes).trim().to_string(),
                })
            }
            _ => {
                // Still running (the normal, expected outcome for a live
                // `tail -F`) or exited cleanly - either way this is one
                // bounded poll, so the probe child is torn down regardless.
                let _ = child.kill();
                let _ = child.wait();
                Ok(data)
            }
        }
    }
}

// ---- private helpers (not exported over FFI) -------------------------------

/// Reject an obviously-incomplete [`ConnectTunnelInputs`] BEFORE ever
/// spawning `wireguard-go` - defense in depth, an honest and specific
/// [`WgStatusRecord::Failed`] reason instead of a confusing UAPI/spawn error
/// over empty args (06 §0.5: no silent fail-open on empty/odd input in a
/// privileged path).
fn validate_connect_inputs(inputs: &ConnectTunnelInputs) -> Result<(), String> {
    if inputs.wireguard_go_bin.trim().is_empty() {
        return Err("no wireguard-go binary path given".to_string());
    }
    if inputs.interface.trim().is_empty() {
        return Err("no interface name given".to_string());
    }
    if inputs.peer_public_key_hex.trim().is_empty() {
        return Err("no peer public key given".to_string());
    }
    if inputs.endpoint.trim().is_empty() {
        return Err("no peer endpoint given".to_string());
    }
    if inputs.allowed_ips.is_empty() {
        return Err("no allowed IPs given".to_string());
    }
    if inputs.local_ip.trim().is_empty() || inputs.peer_ip.trim().is_empty() {
        return Err("tunnel local/peer address is required".to_string());
    }
    Ok(())
}

/// Validate + convert an [`SshTargetRecord`] into a connector
/// [`SshTarget`], rejecting a blank required field up front - see
/// [`RemoteError::InvalidTarget`]'s own doc.
fn ssh_target_from(record: SshTargetRecord) -> Result<SshTarget, RemoteError> {
    let host = record.host.trim().to_string();
    let user = record.user.trim().to_string();
    let identity_file = record.identity_file.trim().to_string();
    let pinned_host_key = record.pinned_host_key.trim().to_string();
    if host.is_empty() || user.is_empty() || identity_file.is_empty() || pinned_host_key.is_empty()
    {
        return Err(RemoteError::InvalidTarget {
            reason: "host, user, identity file, and pinned host key are all required".to_string(),
        });
    }
    Ok(SshTarget {
        host,
        port: record.port,
        user,
        identity_file: PathBuf::from(identity_file),
        pinned_host_key,
    })
}

/// Collect whatever `child`'s stdout produces within `timeout`, then return -
/// see [`RemoteHandle::ssh_tail_once`]'s own doc for why this is a bounded
/// poll, not a live stream. A background reader thread does the blocking
/// `read()` calls so this function itself never blocks past `timeout`; the
/// thread is intentionally not joined here (it exits on its own once the
/// caller kills `child` and the pipe closes - joining would reintroduce the
/// unbounded wait this function exists to avoid). Never panics: a spawn
/// failure or an already-taken stdout both yield an honest empty `Vec`.
fn drain_stdout_for(child: &mut Child, timeout: Duration) -> Vec<u8> {
    let Some(mut stdout) = child.stdout.take() else {
        return Vec::new();
    };
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let spawned = std::thread::Builder::new()
        .name("genaryx-ffi-ssh-tail-reader".into())
        .spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match stdout.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    if spawned.is_err() {
        return Vec::new();
    }

    let mut collected = Vec::new();
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match rx.recv_timeout(remaining) {
            Ok(chunk) => collected.extend(chunk),
            Err(_) => break, // timeout, or the reader thread exited
        }
    }
    collected
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_inputs() -> ConnectTunnelInputs {
        ConnectTunnelInputs {
            wireguard_go_bin: "/usr/local/bin/wireguard-go".to_string(),
            interface: "utun".to_string(),
            peer_public_key_hex: "b".repeat(64),
            endpoint: "203.0.113.9:51820".to_string(),
            allowed_ips: vec!["10.9.0.1/32".to_string()],
            persistent_keepalive: Some(25),
            listen_port: None,
            local_ip: "10.9.0.2".to_string(),
            peer_ip: "10.9.0.1".to_string(),
        }
    }

    fn unresolvable_target() -> SshTargetRecord {
        // `.invalid` is an RFC 2606 TLD reserved to never resolve - mirrors
        // `crates/connectors/src/ssh.rs`'s own
        // `run_against_an_unresolvable_host_is_fail_closed` test, so this
        // fails fast and deterministically without depending on real network
        // reachability.
        SshTargetRecord {
            host: "genaryx.invalid.nonexistent.example".to_string(),
            port: 22,
            user: "root".to_string(),
            identity_file: "/tmp/genaryx-ffi-remote-test-key".to_string(),
            pinned_host_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIabc".to_string(),
        }
    }

    #[test]
    fn new_never_touches_network_or_filesystem_and_defaults_never_panic() {
        let handle = RemoteHandle::new().expect("construct RemoteHandle");
        let _ = handle.default_wireguard_go_bin();
        assert!(!handle.default_interface().is_empty());
        assert!(!handle.default_hetzner_label_selector().is_empty());
        assert!(!handle.default_tunnel_local_ip().is_empty());
        assert!(!handle.default_tunnel_peer_ip().is_empty());
        assert_eq!(handle.default_ssh_port(), 22);
    }

    #[test]
    fn wg_generate_keypair_returns_a_real_public_key_and_never_the_private_half() {
        let handle = RemoteHandle::new().expect("construct");
        let record = handle.wg_generate_keypair().expect("keygen");
        assert_eq!(record.public_hex.len(), 64, "32 raw bytes as hex");
        assert!(!record.public_b64.is_empty());
        // WgKeypairRecord's own field set is the compile-time proof there is
        // no private-key field to leak at all - see the module doc.

        // Regenerating replaces the held keypair with a genuinely different
        // one (new CSPRNG draw), never silently reusing the first.
        let second = handle.wg_generate_keypair().expect("regenerate");
        assert_ne!(record.public_hex, second.public_hex);
    }

    #[test]
    fn connect_tunnel_without_a_generated_keypair_is_an_honest_failed_verdict_not_a_panic() {
        let handle = RemoteHandle::new().expect("construct");
        match handle.connect_tunnel(sample_inputs()) {
            WgStatusRecord::Failed { reason } => {
                assert!(reason.contains("keypair"), "reason: {reason}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn connect_tunnel_with_a_bogus_binary_is_an_honest_failed_verdict_never_fake_connected() {
        let handle = RemoteHandle::new().expect("construct");
        handle.wg_generate_keypair().expect("keygen");
        let mut inputs = sample_inputs();
        inputs.wireguard_go_bin = "/nonexistent/wireguard-go-xyz".to_string();
        let status = handle.connect_tunnel(inputs);
        assert!(
            matches!(status, WgStatusRecord::Failed { .. }),
            "a missing/unprivileged wireguard-go must be an honest FAILED verdict: {status:?}"
        );
        // And tunnel_status agrees: no tunnel was ever stored.
        assert_eq!(handle.tunnel_status(), WgStatusRecord::Disconnected);
    }

    #[test]
    fn connect_tunnel_rejects_blank_required_fields_before_attempting_bring_up() {
        let handle = RemoteHandle::new().expect("construct");
        handle.wg_generate_keypair().expect("keygen");
        let mut inputs = sample_inputs();
        inputs.endpoint = String::new();
        match handle.connect_tunnel(inputs) {
            WgStatusRecord::Failed { reason } => assert!(reason.contains("endpoint")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn tunnel_status_with_nothing_connected_is_disconnected() {
        let handle = RemoteHandle::new().expect("construct");
        assert_eq!(handle.tunnel_status(), WgStatusRecord::Disconnected);
    }

    #[test]
    fn disconnect_tunnel_when_nothing_is_connected_is_a_safe_no_op() {
        let handle = RemoteHandle::new().expect("construct");
        handle.disconnect_tunnel(); // must not panic
        assert_eq!(handle.tunnel_status(), WgStatusRecord::Disconnected);
    }

    #[test]
    fn ssh_check_against_blank_target_fields_is_invalid_target_not_a_panic() {
        let handle = RemoteHandle::new().expect("construct");
        let target = SshTargetRecord {
            host: String::new(),
            port: 22,
            user: "root".to_string(),
            identity_file: "/tmp/key".to_string(),
            pinned_host_key: "ssh-ed25519 AAAA".to_string(),
        };
        match handle.ssh_check(target) {
            Err(RemoteError::InvalidTarget { .. }) => {}
            other => panic!("expected InvalidTarget, got {other:?}"),
        }
    }

    #[test]
    fn ssh_check_against_an_unresolvable_host_is_fail_closed_not_a_panic() {
        let handle = RemoteHandle::new().expect("construct");
        match handle.ssh_check(unresolvable_target()) {
            Err(RemoteError::SshRemote { .. } | RemoteError::SshSpawn { .. }) => {}
            other => panic!("expected a fail-closed SshRemote/SshSpawn error, got {other:?}"),
        }
    }

    #[test]
    fn ssh_read_descriptor_against_an_unresolvable_host_is_fail_closed_not_a_panic() {
        let handle = RemoteHandle::new().expect("construct");
        match handle.ssh_read_descriptor(unresolvable_target(), "/etc/hostname".to_string()) {
            Err(RemoteError::SshRemote { .. } | RemoteError::SshSpawn { .. }) => {}
            other => panic!("expected a fail-closed SshRemote/SshSpawn error, got {other:?}"),
        }
    }

    #[test]
    fn ssh_tail_once_against_an_unresolvable_host_is_fail_closed_not_a_panic() {
        let handle = RemoteHandle::new().expect("construct");
        match handle.ssh_tail_once(unresolvable_target(), "/var/log/taipan.log".to_string(), 0) {
            Err(RemoteError::SshRemote { .. } | RemoteError::SshSpawn { .. }) => {}
            other => panic!("expected a fail-closed SshRemote/SshSpawn error, got {other:?}"),
        }
    }

    #[test]
    fn ssh_tail_once_against_blank_target_fields_is_invalid_target_not_a_panic() {
        let handle = RemoteHandle::new().expect("construct");
        let target = SshTargetRecord {
            host: "example.com".to_string(),
            port: 22,
            user: String::new(),
            identity_file: "/tmp/key".to_string(),
            pinned_host_key: "ssh-ed25519 AAAA".to_string(),
        };
        match handle.ssh_tail_once(target, "/var/log/taipan.log".to_string(), 0) {
            Err(RemoteError::InvalidTarget { .. }) => {}
            other => panic!("expected InvalidTarget, got {other:?}"),
        }
    }
}
