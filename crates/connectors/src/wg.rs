//! WireGuard transport: the PRIMARY console<->Cloud channel (docs/PHASE4.md W4,
//! decision D11). Genaryx brings up its OWN userspace WireGuard peer (a bundled
//! `wireguard-go`, cross-platform, no kernel extension) to a client-hosted
//! Cloud, which is then reachable ONLY inside the tunnel - never exposed to the
//! internet.
//!
//! ## Layering (do not confuse): WG is the channel, signing is the message
//!
//! WireGuard authenticates + encrypts WHO CAN CONNECT AT ALL (the channel). The
//! existing ES256 device-signing (`CloudClient`) stays the MESSAGE layer (WHO
//! SIGNED WHICH ACTION - non-repudiation + audit). WG is ADDITIVE
//! defense-in-depth; it never replaces signing, and a mutation still carries its
//! signature inside the tunnel. This module does ONLY the channel.
//!
//! ## Fail-closed is the whole point
//!
//! The security guarantee is that the Cloud is reachable ONLY through the
//! tunnel. So [`WgTunnel::bring_up`] is fail-closed at every step: if
//! `wireguard-go` will not start, the UAPI socket never appears, the peer is
//! rejected, the address/route cannot be set, or (optionally) no handshake
//! completes in time, it tears the half-built tunnel DOWN and returns an error.
//! There is NO code path in this module that reaches the Cloud without the
//! tunnel - no "direct" fallback, no plaintext path. If bring-up fails, the
//! caller gets nothing to connect through, by construction.
//!
//! ## Keys
//!
//! The console peer's Curve25519 keypair is generated in-process
//! ([`WgKeypair::generate`]) from the OS CSPRNG, clamped exactly as `wg genkey`
//! does. This module GENERATES a session keypair (necessary for the tunnel) but
//! DELETES nothing on its own initiative; a keypair lives only as long as the
//! `WgKeypair`/`WgConfig` value ([[never-delete-keys-on-own-initiative]]). Only
//! the DH key derivation is Rust's; the tunnel data path is entirely
//! `wireguard-go`'s.
//!
//! ## What is unit-tested here vs live
//!
//! Keypair generation, UAPI config rendering, and the address/route command
//! construction are pure and unit-tested. The actual bring-up (spawning
//! `wireguard-go`, creating the tun, the UAPI handshake) needs the
//! `wireguard-go` binary + privileges to create a tun device, so a live
//! loopback test is skip-gracefully and the real end-to-end validation is the
//! W4 Hetzner campaign (the box has both). The fail-closed spawn path IS
//! unit-tested (a bogus binary path -> error, no lingering process).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

// ---- error -----------------------------------------------------------------

/// Every failure mode bringing up / driving a WG tunnel can surface.
/// Fail-closed: each is a hard error that leaves NO usable channel, never a
/// silent fallback.
#[derive(Debug, thiserror::Error)]
pub enum WgError {
    /// Generating the Curve25519 keypair failed (OS entropy).
    #[error("wg keygen: {0}")]
    KeyGen(String),

    /// `wireguard-go` could not be spawned (binary missing / not executable /
    /// insufficient privilege to create a tun).
    #[error("wireguard-go spawn: {0}")]
    Spawn(#[source] std::io::Error),

    /// The UAPI control socket never appeared within the timeout (wireguard-go
    /// failed to create the interface). The tunnel has been torn down.
    #[error("wireguard-go UAPI socket did not appear within {0:?}")]
    UapiTimeout(Duration),

    /// Configuring the interface over the UAPI failed (peer rejected, I/O). The
    /// tunnel has been torn down.
    #[error("wireguard-go UAPI configure: {0}")]
    Configure(String),

    /// Setting the tunnel-local address or route failed. The tunnel has been
    /// torn down.
    #[error("wg interface address/route: {0}")]
    Network(String),

    /// No handshake completed within the timeout - the peer is unreachable, so
    /// the channel is NOT actually up. The tunnel has been torn down (never
    /// left half-open pretending to be connected).
    #[error("wg handshake did not complete within {0:?}")]
    NoHandshake(Duration),
}

// ---- keypair (Curve25519, wg genkey compatible) ----------------------------

/// A WireGuard Curve25519 keypair. The private key is clamped exactly as
/// `wg genkey` produces, so the public key derivation matches the standard tool.
#[derive(Clone)]
pub struct WgKeypair {
    private: [u8; 32],
    public: [u8; 32],
}

// Never print the private key.
impl std::fmt::Debug for WgKeypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WgKeypair")
            .field("private", &"<redacted>")
            .field("public_b64", &self.public_b64())
            .finish()
    }
}

impl WgKeypair {
    /// Generate a fresh keypair from the OS CSPRNG, clamped as WireGuard
    /// requires.
    pub fn generate() -> Result<Self, WgError> {
        let mut private = [0u8; 32];
        getrandom::getrandom(&mut private).map_err(|e| WgError::KeyGen(e.to_string()))?;
        clamp(&mut private);
        Ok(Self::from_private(private))
    }

    /// Build from an existing clamped private key (e.g. one persisted for a
    /// stable console identity). Re-clamps defensively.
    pub fn from_private(mut private: [u8; 32]) -> Self {
        clamp(&mut private);
        let secret = x25519_dalek::StaticSecret::from(private);
        let public = x25519_dalek::PublicKey::from(&secret).to_bytes();
        Self { private, public }
    }

    /// The private key as lowercase hex (the UAPI encoding).
    pub fn private_hex(&self) -> String {
        to_hex(&self.private)
    }
    /// The public key as lowercase hex.
    pub fn public_hex(&self) -> String {
        to_hex(&self.public)
    }
    /// The public key as standard base64 (the `wg`/.conf encoding, and what a
    /// peer's config lists).
    pub fn public_b64(&self) -> String {
        b64(&self.public)
    }
}

/// The standard Curve25519 clamp (RFC 7748), matching `wg genkey`.
fn clamp(k: &mut [u8; 32]) {
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;
}

// ---- config ----------------------------------------------------------------

/// One WireGuard peer (the client-hosted Cloud's WG endpoint).
#[derive(Debug, Clone)]
pub struct WgPeer {
    /// The peer's public key, hex (32 bytes -> 64 hex chars).
    pub public_key_hex: String,
    /// `host:port` the peer listens on (the ONLY internet-facing element; the
    /// Cloud's own API stays inside the tunnel).
    pub endpoint: String,
    /// The tunnel CIDRs routed to this peer, e.g. `["10.9.0.1/32"]`.
    pub allowed_ips: Vec<String>,
    /// Keepalive seconds for NAT traversal (typically 25), or `None`.
    pub persistent_keepalive: Option<u16>,
}

/// The interface configuration pushed to `wireguard-go` over the UAPI.
#[derive(Debug, Clone)]
pub struct WgConfig {
    /// This peer's private key, hex.
    pub private_key_hex: String,
    /// Optional fixed listen port (usually `None` -> ephemeral).
    pub listen_port: Option<u16>,
    pub peers: Vec<WgPeer>,
}

impl WgConfig {
    /// Render the `wireguard-go` UAPI `set=1` operation body (newline-delimited
    /// `key=value`, HEX keys, terminated by a blank line). This is exactly the
    /// bytes written to the UAPI socket to configure the interface;
    /// `replace_peers`/`replace_allowed_ips` make it an authoritative set (no
    /// stale peer survives a reconfigure).
    pub fn render_uapi(&self) -> String {
        let mut s = String::new();
        s.push_str("set=1\n");
        s.push_str(&format!("private_key={}\n", self.private_key_hex));
        if let Some(port) = self.listen_port {
            s.push_str(&format!("listen_port={port}\n"));
        }
        s.push_str("replace_peers=true\n");
        for p in &self.peers {
            s.push_str(&format!("public_key={}\n", p.public_key_hex));
            s.push_str(&format!("endpoint={}\n", p.endpoint));
            if let Some(k) = p.persistent_keepalive {
                s.push_str(&format!("persistent_keepalive_interval={k}\n"));
            }
            s.push_str("replace_allowed_ips=true\n");
            for cidr in &p.allowed_ips {
                s.push_str(&format!("allowed_ip={cidr}\n"));
            }
        }
        s.push('\n'); // blank line terminates the operation
        s
    }
}

/// The tunnel-local address to assign the interface + the peer (gateway)
/// address to route through it.
#[derive(Debug, Clone)]
pub struct WgInterfaceAddr {
    /// This side's tunnel IP, e.g. `10.9.0.2`.
    pub local_ip: String,
    /// The peer side's tunnel IP (the point-to-point destination), e.g.
    /// `10.9.0.1`.
    pub peer_ip: String,
}

/// The platform command to assign `addr` to `interface` (macOS `ifconfig`
/// point-to-point vs Linux `ip addr`). Pure (constructs args, runs nothing) so
/// it is unit-testable. Returns `(program, args)`.
fn addr_command(interface: &str, addr: &WgInterfaceAddr) -> (String, Vec<String>) {
    if cfg!(target_os = "macos") {
        // A host netmask (255.255.255.255) is REQUIRED on a macOS `utun`
        // point-to-point interface: `ifconfig utunN inet LOCAL PEER alias`
        // returns success but silently leaves the interface without an inet
        // address (utunN comes UP with no IP), so no data flows and reachability
        // never completes. `netmask 255.255.255.255` is what actually assigns
        // the address. Verified live on the Phase-4 Hetzner campaign
        // (2026-07-18): the `alias` form left `utun6` address-less; the netmask
        // form brought the tunnel data-path up.
        (
            "ifconfig".to_string(),
            vec![
                interface.to_string(),
                "inet".to_string(),
                addr.local_ip.clone(),
                addr.peer_ip.clone(),
                "netmask".to_string(),
                "255.255.255.255".to_string(),
            ],
        )
    } else {
        (
            "ip".to_string(),
            vec![
                "address".to_string(),
                "add".to_string(),
                format!("{}/32", addr.local_ip),
                "peer".to_string(),
                format!("{}/32", addr.peer_ip),
                "dev".to_string(),
                interface.to_string(),
            ],
        )
    }
}

// ---- tunnel (live; bring-up + fail-closed teardown) ------------------------

const UAPI_DIR: &str = "/var/run/wireguard";
const SOCKET_WAIT: Duration = Duration::from_secs(5);

/// A live WG tunnel: the owned `wireguard-go` child + its interface name. On
/// [`Drop`] the child is killed and the interface torn down, so a tunnel never
/// outlives its handle (fail-closed lifecycle).
#[derive(Debug)]
pub struct WgTunnel {
    child: Child,
    /// The real interface name: the `utunN` wireguard-go chose on macOS, or the
    /// requested name on Linux.
    real_name: String,
    tun_name_file: Option<PathBuf>,
}

impl WgTunnel {
    /// Bring the tunnel up, fail-closed. `wireguard_go_bin` is the resolved
    /// (bundled) binary; `interface` is `utun` on macOS (kernel picks the
    /// number) or a chosen name on Linux; `config` is pushed over the UAPI;
    /// `addr` assigns the tunnel-local address. Any failure tears everything
    /// down and returns an error - the caller never gets a half-open tunnel.
    ///
    /// Needs privileges to create a tun device; without them `wireguard-go`
    /// exits and this returns fail-closed (no tunnel), which is the correct
    /// outcome, not a fallback.
    pub fn bring_up(
        wireguard_go_bin: &Path,
        interface: &str,
        config: &WgConfig,
        addr: &WgInterfaceAddr,
    ) -> Result<Self, WgError> {
        // wireguard-go writes the real interface name here (macOS utun number).
        let tun_name_file = std::env::temp_dir().join(format!(
            "genaryx-wg-name-{}-{}",
            std::process::id(),
            interface
        ));
        let _ = std::fs::remove_file(&tun_name_file);

        // Foreground so we OWN the child (default daemonizes). stderr piped for
        // diagnostics; the data path is entirely inside wireguard-go.
        let child = Command::new(wireguard_go_bin)
            .arg(interface)
            .env("WG_PROCESS_FOREGROUND", "1")
            .env("WG_TUN_NAME_FILE", &tun_name_file)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(WgError::Spawn)?;

        // From here on, ANY error must kill the child (fail-closed). Hold it in
        // a guard that kills on early return, disarmed only on full success.
        let guard = ChildGuard { child: Some(child) };

        // Any `?` below returns early, dropping `guard` -> the child is killed
        // (fail-closed teardown of a half-built tunnel). `disarm` on full
        // success hands the live child to the returned WgTunnel.
        let real_name = wait_for_real_name(&tun_name_file, interface)?;
        let socket = PathBuf::from(UAPI_DIR).join(format!("{real_name}.sock"));
        wait_for_socket(&socket)?;
        uapi_set(&socket, &config.render_uapi())?;
        set_addr(&real_name, addr)?;

        let child = guard.disarm();
        Ok(Self {
            child,
            real_name,
            tun_name_file: Some(tun_name_file),
        })
    }

    /// The seconds-since-epoch of the latest handshake with the first peer, via
    /// the UAPI `get`, or `None` if none yet. `Some(t>0)` means the channel is
    /// genuinely up.
    pub fn latest_handshake_secs(&self) -> Option<u64> {
        let socket = PathBuf::from(UAPI_DIR).join(format!("{}.sock", self.real_name));
        let resp = uapi_get(&socket).ok()?;
        for line in resp.lines() {
            if let Some(v) = line.strip_prefix("last_handshake_time_sec=") {
                return v.parse::<u64>().ok();
            }
        }
        None
    }

    /// The resolved interface name (the real `utunN` on macOS).
    pub fn interface(&self) -> &str {
        &self.real_name
    }
}

impl Drop for WgTunnel {
    fn drop(&mut self) {
        // Kill OUR OWN wireguard-go child (a process we spawned - allowed; not a
        // ps/lsof-discovered PID, and a tunnel process, never a key or a
        // server). Killing it removes the tun + UAPI socket.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(f) = &self.tun_name_file {
            let _ = std::fs::remove_file(f);
        }
    }
}

/// Kills the held child on drop unless [`Self::disarm`]ed - the fail-closed
/// teardown for a bring-up that errors partway.
struct ChildGuard {
    child: Option<Child>,
}
impl ChildGuard {
    fn disarm(mut self) -> Child {
        self.child.take().expect("child present")
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn wait_for_real_name(tun_name_file: &Path, interface: &str) -> Result<String, WgError> {
    // On Linux wireguard-go uses the given name; on macOS it writes the real
    // utun name to WG_TUN_NAME_FILE. Wait briefly for that file on macOS.
    if !cfg!(target_os = "macos") {
        return Ok(interface.to_string());
    }
    let deadline = Instant::now() + SOCKET_WAIT;
    loop {
        if let Ok(name) = std::fs::read_to_string(tun_name_file) {
            let name = name.trim().to_string();
            if !name.is_empty() {
                return Ok(name);
            }
        }
        if Instant::now() >= deadline {
            return Err(WgError::UapiTimeout(SOCKET_WAIT));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_socket(socket: &Path) -> Result<(), WgError> {
    let deadline = Instant::now() + SOCKET_WAIT;
    loop {
        if socket.exists() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(WgError::UapiTimeout(SOCKET_WAIT));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn uapi_set(socket: &Path, config: &str) -> Result<(), WgError> {
    let mut stream = std::os::unix::net::UnixStream::connect(socket)
        .map_err(|e| WgError::Configure(format!("connect uapi: {e}")))?;
    stream
        .write_all(config.as_bytes())
        .map_err(|e| WgError::Configure(format!("write uapi: {e}")))?;
    let mut resp = String::new();
    stream
        .read_to_string(&mut resp)
        .map_err(|e| WgError::Configure(format!("read uapi: {e}")))?;
    // The UAPI answers `errno=0` on success.
    if resp.lines().any(|l| l == "errno=0") {
        Ok(())
    } else {
        Err(WgError::Configure(format!(
            "uapi rejected config: {}",
            resp.trim()
        )))
    }
}

fn uapi_get(socket: &Path) -> Result<String, WgError> {
    let mut stream = std::os::unix::net::UnixStream::connect(socket)
        .map_err(|e| WgError::Configure(format!("connect uapi: {e}")))?;
    stream
        .write_all(b"get=1\n\n")
        .map_err(|e| WgError::Configure(format!("write uapi get: {e}")))?;
    let mut resp = String::new();
    stream
        .read_to_string(&mut resp)
        .map_err(|e| WgError::Configure(format!("read uapi get: {e}")))?;
    Ok(resp)
}

fn set_addr(interface: &str, addr: &WgInterfaceAddr) -> Result<(), WgError> {
    let (prog, args) = addr_command(interface, addr);
    let out = Command::new(&prog)
        .args(&args)
        .output()
        .map_err(|e| WgError::Network(format!("spawn {prog}: {e}")))?;
    if !out.status.success() {
        return Err(WgError::Network(format!(
            "{prog} exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    // Linux also needs the link brought up.
    if !cfg!(target_os = "macos") {
        let up = Command::new("ip")
            .args(["link", "set", "up", "dev", interface])
            .output()
            .map_err(|e| WgError::Network(format!("ip link set up: {e}")))?;
        if !up.status.success() {
            return Err(WgError::Network(format!(
                "ip link set up exited {}: {}",
                up.status,
                String::from_utf8_lossy(&up.stderr).trim()
            )));
        }
    }
    Ok(())
}

// ---- encoding helpers ------------------------------------------------------

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keypair_is_clamped_and_public_derives_deterministically() {
        let kp = WgKeypair::generate().expect("keygen");
        // Clamp invariants (RFC 7748): low 3 bits of byte0 clear, top 2 bits of
        // byte31 are 0 then 1.
        assert_eq!(kp.private[0] & 0b0000_0111, 0);
        assert_eq!(kp.private[31] & 0b1100_0000, 0b0100_0000);
        // hex/b64 lengths.
        assert_eq!(kp.private_hex().len(), 64);
        assert_eq!(kp.public_hex().len(), 64);
        // Deterministic: same private -> same public.
        let again = WgKeypair::from_private(kp.private);
        assert_eq!(again.public_hex(), kp.public_hex());
        // Debug never leaks the private key.
        assert!(!format!("{kp:?}").contains(&kp.private_hex()));
    }

    #[test]
    fn public_key_derivation_matches_the_rfc_7748_x25519_vector() {
        // RFC 7748 §6.1 (the same X25519 curve WireGuard uses). Alice's private
        // scalar (NOT pre-clamped on the wire) derives the given public; our
        // derivation clamps exactly as X25519 does, so the public must match
        // byte-for-byte. This pins that the crypto is correct, not just that
        // clamping happens - a wrong pubkey would silently break every tunnel.
        let alice_private: [u8; 32] = [
            0x77, 0x07, 0x6d, 0x0a, 0x73, 0x18, 0xa5, 0x7d, 0x3c, 0x16, 0xc1, 0x72, 0x51, 0xb2,
            0x66, 0x45, 0xdf, 0x4c, 0x2f, 0x87, 0xeb, 0xc0, 0x99, 0x2a, 0xb1, 0x77, 0xfb, 0xa5,
            0x1d, 0xb9, 0x2c, 0x2a,
        ];
        let expected_public_hex =
            "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a";
        let kp = WgKeypair::from_private(alice_private);
        assert_eq!(kp.public_hex(), expected_public_hex);
        // And from_private re-clamps a non-clamped input's stored scalar.
        assert_eq!(kp.private[0] & 0b0000_0111, 0);
        assert_eq!(kp.private[31] & 0b1100_0000, 0b0100_0000);
    }

    #[test]
    fn render_uapi_has_hex_keys_replace_flags_and_peer() {
        let cfg = WgConfig {
            private_key_hex: "aa".repeat(32),
            listen_port: None,
            peers: vec![WgPeer {
                public_key_hex: "bb".repeat(32),
                endpoint: "203.0.113.9:51820".into(),
                allowed_ips: vec!["10.9.0.1/32".into()],
                persistent_keepalive: Some(25),
            }],
        };
        let u = cfg.render_uapi();
        assert!(u.starts_with("set=1\n"));
        assert!(u.contains(&format!("private_key={}\n", "aa".repeat(32))));
        assert!(u.contains("replace_peers=true\n"));
        assert!(u.contains(&format!("public_key={}\n", "bb".repeat(32))));
        assert!(u.contains("endpoint=203.0.113.9:51820\n"));
        assert!(u.contains("persistent_keepalive_interval=25\n"));
        assert!(u.contains("replace_allowed_ips=true\n"));
        assert!(u.contains("allowed_ip=10.9.0.1/32\n"));
        assert!(u.ends_with("\n\n"), "blank line terminates the op");
    }

    #[test]
    fn addr_command_is_platform_appropriate() {
        let addr = WgInterfaceAddr {
            local_ip: "10.9.0.2".into(),
            peer_ip: "10.9.0.1".into(),
        };
        let (prog, args) = addr_command("utun7", &addr);
        let joined = format!("{prog} {}", args.join(" "));
        if cfg!(target_os = "macos") {
            assert_eq!(prog, "ifconfig");
            // Host netmask (not `alias`) is what actually assigns the address on
            // a macOS utun point-to-point interface - see addr_command's note.
            assert!(joined.contains("utun7 inet 10.9.0.2 10.9.0.1 netmask 255.255.255.255"));
        } else {
            assert_eq!(prog, "ip");
            assert!(joined.contains("address add 10.9.0.2/32 peer 10.9.0.1/32 dev utun7"));
        }
    }

    #[test]
    fn bring_up_with_a_bogus_binary_is_fail_closed_no_lingering() {
        let cfg = WgConfig {
            private_key_hex: "aa".repeat(32),
            listen_port: None,
            peers: vec![],
        };
        let addr = WgInterfaceAddr {
            local_ip: "10.9.0.2".into(),
            peer_ip: "10.9.0.1".into(),
        };
        let r = WgTunnel::bring_up(
            Path::new("/nonexistent/wireguard-go-xyz"),
            "utun",
            &cfg,
            &addr,
        );
        match r {
            Err(WgError::Spawn(_)) => {}
            other => panic!("expected fail-closed Spawn error, got {other:?}"),
        }
        // No WgTunnel returned -> nothing to leak; the ChildGuard never held a
        // real child (spawn failed before it was constructed).
    }
}
