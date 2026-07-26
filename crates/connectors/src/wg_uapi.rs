//! WireGuard UAPI client: manage peers on an interface this process does NOT
//! own, over the userspace API socket `wireguard-go` exposes.
//!
//! ## Why this exists next to [`crate::wg`]
//!
//! [`crate::wg`] owns a tunnel: it spawns its own `wireguard-go`, configures
//! the whole interface authoritatively (`replace_peers=true`) and kills the
//! child on drop. That is the console DIALLING OUT, and it is right for that.
//!
//! This module is the opposite posture. The interface already exists, someone
//! else brought it up, and this process is a guest that may only add or remove
//! one peer at a time without disturbing the others. Reusing
//! [`crate::wg::WgConfig::render_uapi`] here would be a bug rather than reuse:
//! its `replace_peers=true` is exactly what must NOT happen when a second
//! operator device is issued while the first is connected.
//!
//! ## Why UAPI rather than shelling `wg`
//!
//! The console runs in a container with no `NET_ADMIN`, no `/dev/net/tun` and
//! no `wg` binary, so `wg set <iface> peer ...` cannot work there and says so
//! ([`crate::wg`]'s sibling `wg_operator` detects it and refuses honestly).
//! The UAPI socket crosses that boundary as a plain file: a sidecar holds the
//! privileges and the tunnel, this process holds a unix socket, and peer
//! management needs no capability at all. The same code path then works
//! unchanged on a host install, where the socket is simply the local one.
//!
//! ## Protocol
//!
//! Newline-delimited `key=value`, one operation per connection, terminated by
//! a blank line. The daemon answers with the same shape, ending in `errno=N`
//! where 0 is success. Keys are lowercase hex, NOT the base64 that `wg show`
//! prints - conversion lives in [`hex_to_b64`]/[`b64_to_hex`] because the
//! operator-facing `.conf` and QR must carry base64 while the wire carries
//! hex. Reference: <https://www.wireguard.com/xplatform/>.

use std::io::{BufReader, Read as _, Write as _};
use std::net::{TcpStream, ToSocketAddrs as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;

/// Where `wireguard-go` places its UAPI sockets, on every platform it targets.
/// Same constant as [`crate::wg`]'s, deliberately not shared: these two agree
/// today because the daemon dictates the path, not because one module owns it.
const UAPI_DIR: &str = "/var/run/wireguard";

/// A single UAPI exchange is local I/O against a live daemon. It either
/// answers immediately or something is wrong; a hung read here would hang a
/// console request, so both directions are bounded.
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// `exists()` is a pre-check, not an operation: it must not add five seconds
/// to every refusal, so it gets its own shorter budget.
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
pub enum WgUapiError {
    /// No socket at the expected path: the interface does not exist, or the
    /// sidecar holding it is not running. Distinguished from a connect failure
    /// because the remedies differ (start the tunnel vs fix permissions).
    #[error("no wireguard UAPI at {0}: the interface is not up")]
    NoSocket(String),

    /// The socket exists but will not talk: almost always filesystem
    /// permissions on a shared volume, which is worth naming explicitly
    /// because the fix is a mount option rather than anything in this code.
    #[error("wireguard UAPI at {endpoint}: {source}")]
    Io {
        endpoint: String,
        #[source]
        source: std::io::Error,
    },

    /// The daemon answered `errno=N`, N != 0. UAPI reports negative errno
    /// values, so this keeps the raw number rather than guessing a message.
    #[error("wireguard UAPI refused the operation: errno={0}")]
    Errno(i64),

    /// A well-formed exchange whose body we could not make sense of.
    #[error("wireguard UAPI response malformed: {0}")]
    Malformed(&'static str),

    /// The endpoint is reachable in principle but the exchange failed: TLS,
    /// resolution, or the pinned certificate. Separate from `Io` because none
    /// of these are a filesystem problem and the remedies differ.
    #[error("wireguard UAPI at {endpoint}: {message}")]
    Transport { endpoint: String, message: String },

    /// A key was not the 64 lowercase hex characters or 44 base64 characters
    /// a Curve25519 key must be. Rejected before it reaches the wire, because
    /// a malformed key in a `set` is accepted as a DIFFERENT peer rather than
    /// refused, which would silently strand the device being issued.
    #[error("not a valid wireguard key: {0}")]
    BadKey(String),
}

// ============================================================================
// key encodings
// ============================================================================

/// Base64 (what `.conf` files, QR codes and `wg show` use) from lowercase hex
/// (what the UAPI wire uses).
pub fn hex_to_b64(hex: &str) -> Result<String, WgUapiError> {
    let bytes = decode_hex32(hex)?;
    Ok(B64.encode(bytes))
}

/// Lowercase hex (UAPI wire) from base64 (`.conf`/`wg show`).
pub fn b64_to_hex(b64: &str) -> Result<String, WgUapiError> {
    let bytes = B64
        .decode(b64.trim())
        .map_err(|_| WgUapiError::BadKey(format!("not base64: {b64}")))?;
    if bytes.len() != 32 {
        return Err(WgUapiError::BadKey(format!(
            "a wireguard key is 32 bytes, got {}",
            bytes.len()
        )));
    }
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn decode_hex32(hex: &str) -> Result<[u8; 32], WgUapiError> {
    let hex = hex.trim();
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(WgUapiError::BadKey(format!(
            "expected 64 hex characters, got {:?}",
            &hex[..hex.len().min(16)]
        )));
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).expect("hex is ascii");
        out[i] = u8::from_str_radix(s, 16).expect("validated as hex above");
    }
    Ok(out)
}

// ============================================================================
// wire rendering (pure)
// ============================================================================

/// The `get=1` request body.
pub fn render_get() -> &'static str {
    "get=1\n\n"
}

/// A `set=1` that ADDS or UPDATES exactly one peer and leaves every other peer
/// untouched. Deliberately no `replace_peers`: issuing a second device must
/// not disconnect the first.
///
/// `replace_allowed_ips=true` is scoped to THIS peer and is what makes a
/// re-issue to the same key idempotent rather than accumulating stale CIDRs.
pub fn render_add_peer(public_key_hex: &str, allowed_ips: &[String]) -> Result<String, WgUapiError> {
    // Validated rather than trusted: an invalid key here would be accepted by
    // the daemon as some other peer entirely.
    decode_hex32(public_key_hex)?;
    let mut s = String::from("set=1\n");
    s.push_str(&format!("public_key={}\n", public_key_hex.trim()));
    s.push_str("replace_allowed_ips=true\n");
    for cidr in allowed_ips {
        s.push_str(&format!("allowed_ip={cidr}\n"));
    }
    s.push('\n');
    Ok(s)
}

/// A `set=1` that REMOVES exactly one peer, leaving the rest alone. This is
/// the revocation primitive: the device's key stops being able to complete a
/// handshake the moment the daemon applies it.
pub fn render_remove_peer(public_key_hex: &str) -> Result<String, WgUapiError> {
    decode_hex32(public_key_hex)?;
    let mut s = String::from("set=1\n");
    s.push_str(&format!("public_key={}\n", public_key_hex.trim()));
    s.push_str("remove=true\n");
    s.push('\n');
    Ok(s)
}

// ============================================================================
// wire parsing (pure)
// ============================================================================

/// One peer as the daemon reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerState {
    /// Lowercase hex, as it appears on the wire.
    pub public_key_hex: String,
    /// The CIDRs routed to this peer. For an operator device this is its
    /// single `/32`, which is also how [`taken_host_octets`] finds free
    /// addresses.
    pub allowed_ips: Vec<String>,
    /// Unix seconds of the last completed handshake, or `None` if the peer has
    /// never connected. `0` from the wire means never, and is normalised to
    /// `None` here so callers cannot mistake the epoch for a real time.
    pub last_handshake_unix: Option<u64>,
    /// `host:port` the peer was last seen at, when the daemon knows it.
    pub endpoint: Option<String>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// The interface as the daemon reports it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InterfaceState {
    /// The SERVER's own public key, hex. Read live rather than remembered, so
    /// a rotated key is reflected in the very next issued config.
    pub public_key_hex: Option<String>,
    pub listen_port: Option<u16>,
    pub peers: Vec<PeerState>,
}

impl InterfaceState {
    /// The server's public key in the base64 form a client `.conf` needs.
    pub fn public_key_b64(&self) -> Option<String> {
        self.public_key_hex.as_deref().and_then(|h| hex_to_b64(h).ok())
    }

    /// Whether a peer with this public key (hex) is currently configured.
    pub fn has_peer(&self, public_key_hex: &str) -> bool {
        let want = public_key_hex.trim();
        self.peers.iter().any(|p| p.public_key_hex == want)
    }
}

/// Parse a `get=1` response body into [`InterfaceState`].
///
/// The wire is a flat stream of `key=value` lines where `public_key=` starts a
/// new peer section and every later key belongs to that peer until the next
/// one. Interface-level keys appear BEFORE the first peer, which is what makes
/// a single pass with a "current peer" cursor correct.
pub fn parse_get(response: &str) -> Result<InterfaceState, WgUapiError> {
    let mut state = InterfaceState::default();
    let mut current: Option<PeerState> = None;
    let mut saw_errno = false;

    for line in response.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(WgUapiError::Malformed("a line carried no '='"));
        };
        match key {
            "errno" => {
                saw_errno = true;
                let n: i64 = value
                    .parse()
                    .map_err(|_| WgUapiError::Malformed("errno was not a number"))?;
                if n != 0 {
                    return Err(WgUapiError::Errno(n));
                }
            }
            "private_key" => {
                // The daemon reports the interface's PRIVATE key and never its
                // public one, so the public key has to be derived here or the
                // server identity comes out empty and every issued config
                // names no peer to connect to.
                //
                // The private half is derived from and immediately dropped: it
                // is never stored on the struct, so it cannot reach a DTO, a
                // log line or a panic message by accident. The public half is
                // not a secret and is exactly what a client config needs.
                if state.public_key_hex.is_none() {
                    if let Ok(bytes) = decode_hex32(value) {
                        state.public_key_hex = Some(crate::wg::WgKeypair::from_private(bytes).public_hex());
                    }
                }
            }
            // What a FILTERING relay sends instead of `private_key`, so the
            // server's private half never crosses a network. The daemon
            // reports only its private key and never its public one, so a
            // relay that merely stripped the line would leave the console with
            // no server identity and every issued config naming no peer. It
            // substitutes instead, and this arm is the other half of that.
            //
            // Deliberately NOT named `public_key`: that key starts a peer in
            // this protocol, so reusing it would add a phantom device to every
            // listing. See stack-k8s/tunnel/DESIGN-uapi-transport.md.
            "interface_public_key" => {
                // Validated, not trusted: a malformed value here would put a
                // key nobody holds into every client config that follows.
                if decode_hex32(value).is_ok() {
                    state.public_key_hex = Some(value.trim().to_string());
                }
            }
            // Every `public_key` starts a peer, with no special case for the
            // first one. The interface's own identity arrives as `private_key`
            // and is derived above; treating the first peer as the interface
            // would silently drop a real device from the list.
            "public_key" => {
                if let Some(done) = current.take() {
                    state.peers.push(done);
                }
                current = Some(PeerState {
                    public_key_hex: value.to_string(),
                    allowed_ips: Vec::new(),
                    last_handshake_unix: None,
                    endpoint: None,
                    rx_bytes: 0,
                    tx_bytes: 0,
                });
            }
            "listen_port" => state.listen_port = value.parse().ok(),
            "allowed_ip" => {
                if let Some(p) = current.as_mut() {
                    p.allowed_ips.push(value.to_string());
                }
            }
            "endpoint" => {
                if let Some(p) = current.as_mut() {
                    p.endpoint = Some(value.to_string());
                }
            }
            "last_handshake_time_sec" => {
                if let Some(p) = current.as_mut() {
                    match value.parse::<u64>() {
                        Ok(0) | Err(_) => p.last_handshake_unix = None,
                        Ok(n) => p.last_handshake_unix = Some(n),
                    }
                }
            }
            "rx_bytes" => {
                if let Some(p) = current.as_mut() {
                    p.rx_bytes = value.parse().unwrap_or(0);
                }
            }
            "tx_bytes" => {
                if let Some(p) = current.as_mut() {
                    p.tx_bytes = value.parse().unwrap_or(0);
                }
            }
            _ => {}
        }
    }
    if let Some(done) = current.take() {
        state.peers.push(done);
    }
    if !saw_errno {
        return Err(WgUapiError::Malformed("response carried no errno"));
    }
    Ok(state)
}

/// Check a `set=1` response, which carries nothing but its `errno`.
pub fn parse_set_result(response: &str) -> Result<(), WgUapiError> {
    for line in response.lines() {
        if let Some(v) = line.trim().strip_prefix("errno=") {
            let n: i64 = v
                .parse()
                .map_err(|_| WgUapiError::Malformed("errno was not a number"))?;
            return if n == 0 { Ok(()) } else { Err(WgUapiError::Errno(n)) };
        }
    }
    Err(WgUapiError::Malformed("response carried no errno"))
}

// ============================================================================
// address allocation (pure)
// ============================================================================

/// The host octets already routed inside `10.9.0.0/24`, from live peer state.
///
/// This is what replaces "always hand out `10.9.0.2`". A fixed address means
/// the second device issued silently supersedes the first, because WireGuard
/// keeps exactly one owner per `/32`: the first laptop stops receiving without
/// anything reporting an error.
pub fn taken_host_octets(state: &InterfaceState) -> Vec<u8> {
    let mut taken: Vec<u8> = state
        .peers
        .iter()
        .flat_map(|p| p.allowed_ips.iter())
        .filter_map(|cidr| {
            let addr = cidr.split('/').next()?;
            let mut parts = addr.split('.');
            let (a, b, c, d) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
            if parts.next().is_some() || (a, b, c) != ("10", "9", "0") {
                return None;
            }
            d.parse::<u8>().ok()
        })
        .collect();
    taken.sort_unstable();
    taken.dedup();
    taken
}

/// The lowest free client address in `10.9.0.0/24`, skipping `.0` (network),
/// `.1` (the server itself) and `.255` (broadcast).
///
/// Returns `None` when the subnet is full rather than wrapping to an address
/// already in use: handing out a duplicate would break a working device to
/// serve a new one, which is never the better failure.
pub fn next_free_client_ip(state: &InterfaceState) -> Option<String> {
    let taken = taken_host_octets(state);
    (2u8..=254).find(|n| !taken.contains(n)).map(|n| format!("10.9.0.{n}"))
}

// ============================================================================
// live socket
// ============================================================================

/// Where the daemon's UAPI answers.
///
/// A unix socket when the daemon shares this process's pod, which is the only
/// shape that exists today. The enum is here rather than a bare `PathBuf`
/// because a unix socket cannot cross a pod boundary, and that single fact is
/// what forces the console into a privileged pod on Kubernetes: see
/// `stack-k8s/tunnel/DESIGN-uapi-transport.md`. Adding the second variant is
/// the change that lets the console go back where it belongs.
#[derive(Debug, Clone)]
enum Endpoint {
    Unix(PathBuf),
    /// The daemon is in another pod, so its socket cannot be shared. Reached
    /// over TLS with a bearer, through a proxy that also refuses the
    /// operations this side has no business sending.
    Tls(TlsEndpoint),
}

/// Everything needed to reach a daemon that is not in this pod.
///
/// `Debug` is written by hand rather than derived: this lives inside
/// `UapiSocket`, which IS derived, and a bearer that reaches a Debug format is
/// a bearer in a log file.
#[derive(Clone)]
struct TlsEndpoint {
    /// `host:port`. A Service name in cluster, resolved at dial time.
    addr: String,
    /// The proxy's certificate, pinned as the ONLY root. Not a CA: there is no
    /// certificate authority here and no lifecycle to run, just one
    /// self-signed cert both ends were handed at install.
    cert_pem: PathBuf,
    token: String,
}

impl std::fmt::Debug for TlsEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsEndpoint")
            .field("addr", &self.addr)
            .field("cert_pem", &self.cert_pem)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl Endpoint {
    /// How to name this endpoint in an error a human has to act on.
    fn describe(&self) -> String {
        match self {
            Endpoint::Unix(p) => p.display().to_string(),
            Endpoint::Tls(t) => t.addr.clone(),
        }
    }
}

/// A handle to one interface's UAPI. Holds no connection: UAPI is one
/// operation per connection, so every call dials afresh.
#[derive(Debug, Clone)]
pub struct UapiSocket {
    at: Endpoint,
}

impl UapiSocket {
    /// The socket for `iface` in the daemon's standard directory.
    pub fn for_interface(iface: &str) -> Self {
        Self {
            at: Endpoint::Unix(Path::new(UAPI_DIR).join(format!("{iface}.sock"))),
        }
    }

    /// An explicit path, for a sidecar that mounts the directory elsewhere and
    /// for tests that stand up a socket in a temp dir.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            at: Endpoint::Unix(path.into()),
        }
    }

    /// A daemon in another pod, over TLS with a bearer.
    ///
    /// `cert_pem` is the proxy's own certificate, pinned as the only root.
    /// There is no CA and no certificate lifecycle: both ends were handed the
    /// same self-signed cert at install, which is the whole trust story.
    pub fn tls(addr: impl Into<String>, cert_pem: impl Into<PathBuf>, token: impl Into<String>) -> Self {
        Self {
            at: Endpoint::Tls(TlsEndpoint {
                addr: addr.into(),
                cert_pem: cert_pem.into(),
                token: token.into(),
            }),
        }
    }

    /// How to name where this is looking, for an error message. Replaces the
    /// old `path()`, which could only ever describe one kind of endpoint.
    pub fn describe(&self) -> String {
        self.at.describe()
    }

    /// Whether this endpoint is reached over a network rather than a local
    /// socket.
    ///
    /// Exists so a caller can tell a transient failure from a structural one:
    /// a local socket that is missing may mean "start the daemon", while a
    /// network endpoint that does not answer can never be replaced by a local
    /// fallback, so falling back is worse than refusing.
    pub fn is_network(&self) -> bool {
        matches!(self.at, Endpoint::Tls(_))
    }

    /// Whether the endpoint is there at all. Cheap pre-check so callers can
    /// give the "interface is not up" answer without a full exchange.
    ///
    /// Load-bearing beyond the message: `resolve_backend` in the api crate
    /// gates on this and falls through to shelling out to `wg` when it is
    /// false, so an endpoint this answers wrongly about does not fail, it
    /// silently picks a backend that cannot work in a container.
    pub fn exists(&self) -> bool {
        match &self.at {
            Endpoint::Unix(p) => p.exists(),
            // A connect probe, because there is no file to stat. Cheap, and it
            // answers the question `resolve_backend` is really asking: is
            // there a daemon this console can reach at all.
            Endpoint::Tls(t) => t
                .addr
                .to_socket_addrs()
                .ok()
                .and_then(|mut a| a.next())
                .map(|a| TcpStream::connect_timeout(&a, PROBE_TIMEOUT).is_ok())
                .unwrap_or(false),
        }
    }

    /// One request/response exchange.
    fn request(&self, body: &str) -> Result<String, WgUapiError> {
        let path = match &self.at {
            Endpoint::Unix(p) => p,
            // No pre-check here, deliberately. For TLS, connecting IS the
            // check, and probing first would mask a misconfigured certificate
            // behind "the interface is not up": a permanent error reported as
            // a transient one, which sends the reader to restart a daemon that
            // was never the problem. `request_tls` validates its configuration
            // before it dials, so the file nobody filled in is named first.
            Endpoint::Tls(t) => return self.request_tls(t, body),
        };
        if !path.exists() {
            return Err(WgUapiError::NoSocket(self.describe()));
        }
        let mut sock = UnixStream::connect(path).map_err(|source| WgUapiError::Io {
            endpoint: self.describe(),
            source,
        })?;
        let io = |source| WgUapiError::Io {
            endpoint: self.describe(),
            source,
        };
        sock.set_read_timeout(Some(IO_TIMEOUT)).map_err(io)?;
        sock.set_write_timeout(Some(IO_TIMEOUT)).map_err(io)?;
        sock.write_all(body.as_bytes()).map_err(io)?;
        sock.flush().map_err(io)?;
        // Half-close the write side. The daemon reads the operation until the
        // peer stops writing, so without this it keeps waiting and the read
        // below times out with EAGAIN instead of returning the reply that was
        // never sent. Straight to the daemon a blank line is often enough; it
        // is not through the forwarder in front of it, and the shutdown is
        // what makes both paths behave the same.
        sock.shutdown(std::net::Shutdown::Write).map_err(io)?;
        let mut out = String::new();
        sock.read_to_string(&mut out).map_err(io)?;
        Ok(out)
    }

    /// One request/response exchange against a proxy in another pod.
    ///
    /// The bearer goes first, on its own line, and the proxy consumes it
    /// before forwarding anything. Deliberately NOT a half-close afterwards:
    /// the unix path shuts down its write side because the daemon reads until
    /// the peer stops writing, and TLS has no half-close, only `close_notify`.
    /// The proxy reads to the blank line the protocol already terminates on
    /// instead, so both paths end an operation the same way without either
    /// depending on a transport detail.
    fn request_tls(&self, t: &TlsEndpoint, body: &str) -> Result<String, WgUapiError> {
        let fail = |m: String| WgUapiError::Transport {
            endpoint: t.addr.clone(),
            message: m,
        };

        let mut roots = rustls::RootCertStore::empty();
        let file = std::fs::File::open(&t.cert_pem)
            .map_err(|e| fail(format!("cannot read {}: {e}", t.cert_pem.display())))?;
        let mut rd = BufReader::new(file);
        let mut added = 0usize;
        for cert in rustls_pemfile::certs(&mut rd) {
            let cert = cert.map_err(|e| fail(format!("{} is not a PEM certificate: {e}", t.cert_pem.display())))?;
            roots.add(cert).map_err(|e| fail(format!("certificate rejected: {e}")))?;
            added += 1;
        }
        // An empty root store does not fail to build, it fails every
        // handshake, with an error about the SERVER rather than about the file
        // nobody filled in.
        if added == 0 {
            return Err(fail(format!(
                "{} contains no certificate, so nothing would be trusted",
                t.cert_pem.display()
            )));
        }

        let cfg = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();

        let host = t.addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(&t.addr).to_string();
        let server_name = rustls_pki_types::ServerName::try_from(host.clone())
            .map_err(|_| fail(format!("{host} is not a valid server name")))?;
        let conn = rustls::ClientConnection::new(Arc::new(cfg), server_name)
            .map_err(|e| fail(format!("TLS setup failed: {e}")))?;

        let addr = t
            .addr
            .to_socket_addrs()
            .map_err(|e| fail(format!("cannot resolve: {e}")))?
            .next()
            .ok_or_else(|| fail("resolved to no address".into()))?;
        let tcp = TcpStream::connect_timeout(&addr, IO_TIMEOUT)
            .map_err(|e| fail(format!("cannot connect: {e}")))?;
        tcp.set_read_timeout(Some(IO_TIMEOUT))
            .and_then(|_| tcp.set_write_timeout(Some(IO_TIMEOUT)))
            .map_err(|e| fail(format!("cannot set timeouts: {e}")))?;

        let mut tls = rustls::StreamOwned::new(conn, tcp);
        let framed = format!("bearer={}\n{body}", t.token);
        tls.write_all(framed.as_bytes())
            .and_then(|_| tls.flush())
            .map_err(|e| fail(format!("write failed: {e}")))?;

        let mut out = String::new();
        tls.read_to_string(&mut out)
            .map_err(|e| fail(format!("read failed: {e}")))?;
        Ok(out)
    }

    /// Read the interface's live state.
    pub fn state(&self) -> Result<InterfaceState, WgUapiError> {
        parse_get(&self.request(render_get())?)
    }

    /// Add or update one peer, leaving every other peer connected.
    pub fn add_peer(&self, public_key_hex: &str, allowed_ips: &[String]) -> Result<(), WgUapiError> {
        parse_set_result(&self.request(&render_add_peer(public_key_hex, allowed_ips)?)?)
    }

    /// Remove one peer. Idempotent at the daemon: removing an absent peer is
    /// `errno=0`, which is the right shape for a revoke that may be retried.
    pub fn remove_peer(&self, public_key_hex: &str) -> Result<(), WgUapiError> {
        parse_set_result(&self.request(&render_remove_peer(public_key_hex)?)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The real public key of the fixture's private key below, taken from
    // `wg pubkey`. A made-up constant would let a broken derivation pass.
    const SERVER_PRIV: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const SERVER_PUB: &str = "7b4e909bbe7ffe44c465a220037d608ee35897d31ef972f07f74892cb0f73f13";
    const PEER_A: &str = "aa11bb22cc33dd44ee55ff6677889900112233445566778899aabbccddeeff00";
    const PEER_B: &str = "bb11cc22dd33ee44ff5500667788990011223344556677889900aabbccddeeff";

    fn get_response() -> String {
        // Shaped exactly as wireguard-go answers: interface keys first, then
        // one section per peer, errno last.
        format!(
            "private_key={SERVER_PRIV}\n\
             listen_port=51820\n\
             public_key={PEER_A}\n\
             endpoint=203.0.113.7:54321\n\
             allowed_ip=10.9.0.2/32\n\
             last_handshake_time_sec=1769000000\n\
             rx_bytes=4096\n\
             tx_bytes=8192\n\
             public_key={PEER_B}\n\
             allowed_ip=10.9.0.5/32\n\
             last_handshake_time_sec=0\n\
             rx_bytes=0\n\
             tx_bytes=0\n\
             errno=0\n\n"
        )
    }

    #[test]
    fn the_server_identity_is_derived_from_the_private_key() {
        // wireguard-go reports the interface's PRIVATE key and never its
        // public one, so an implementation that waits for `public_key=` at the
        // top reports no server identity at all and issues configs naming
        // nobody. This is what that regression would look like.
        let s = parse_get(&get_response()).unwrap();
        assert_eq!(
            s.public_key_hex.as_deref(),
            Some(SERVER_PUB),
            "the interface public key must be derived from its private key"
        );
        assert_eq!(s.listen_port, Some(51820));
        assert_eq!(s.peers.len(), 2, "the interface key must not become a peer");
    }

    #[test]
    fn peer_fields_land_on_the_peer_they_follow() {
        let s = parse_get(&get_response()).unwrap();
        let a = &s.peers[0];
        assert_eq!(a.public_key_hex, PEER_A);
        assert_eq!(a.allowed_ips, vec!["10.9.0.2/32"]);
        assert_eq!(a.endpoint.as_deref(), Some("203.0.113.7:54321"));
        assert_eq!(a.last_handshake_unix, Some(1769000000));
        assert_eq!((a.rx_bytes, a.tx_bytes), (4096, 8192));
    }

    #[test]
    fn a_zero_handshake_means_never_not_the_epoch() {
        let s = parse_get(&get_response()).unwrap();
        assert_eq!(
            s.peers[1].last_handshake_unix, None,
            "0 is 'never connected'; reporting 1970 would read as a real time"
        );
    }

    #[test]
    #[test]
    fn a_tls_endpoint_is_named_by_address_not_by_a_path_it_does_not_have() {
        let sock = UapiSocket::tls("wg.agent-stack:9090", "/etc/wg/proxy.crt", "s3cret");
        assert_eq!(sock.describe(), "wg.agent-stack:9090");
    }

    #[test]
    fn the_bearer_never_reaches_a_debug_format() {
        // This struct is inside one that derives Debug, and every error path
        // in the api crate formats errors into messages an operator reads. A
        // token that survives `{:?}` is a token in a log file.
        let sock = UapiSocket::tls("wg:9090", "/etc/wg/proxy.crt", "SUPERSECRETTOKEN");
        let rendered = format!("{sock:?}");
        assert!(
            !rendered.contains("SUPERSECRETTOKEN"),
            "the bearer must not survive Debug, got: {rendered}"
        );
        assert!(rendered.contains("redacted"), "and it should say so");
    }

    #[test]
    fn a_certificate_file_with_nothing_in_it_is_named_as_the_problem() {
        // An empty root store does not fail to build: it fails every
        // handshake, with an error about the SERVER rather than about the file
        // nobody filled in. That is an hour of looking in the wrong place.
        let dir = std::env::temp_dir().join(format!("wg-uapi-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let empty = dir.join("empty.crt");
        std::fs::write(&empty, b"").unwrap();

        let sock = UapiSocket::tls("127.0.0.1:9", &empty, "t");
        let err = sock.state().unwrap_err().to_string();
        std::fs::remove_dir_all(&dir).ok();

        assert!(
            err.contains("no certificate"),
            "the empty file must be named, got: {err}"
        );
    }

    #[test]
    fn a_missing_certificate_file_is_named_as_the_problem() {
        let sock = UapiSocket::tls("127.0.0.1:9", "/nonexistent/genaryx-test/proxy.crt", "t");
        let err = sock.state().unwrap_err().to_string();
        assert!(
            err.contains("cannot read"),
            "the unreadable file must be named, got: {err}"
        );
    }

    #[test]
    fn a_relay_may_substitute_the_public_half_and_it_is_not_read_as_a_peer() {
        // What a filtering relay sends: no private_key at all, and the server
        // identity supplied directly. The listing must show the peers that are
        // really there and not one more.
        let raw = format!(
            "interface_public_key={SERVER_PUB}\nlisten_port=51820\n\
             public_key={PEER_A}\nallowed_ip=10.9.0.2/32\nerrno=0\n"
        );
        let s = parse_get(&raw).unwrap();
        assert_eq!(
            s.public_key_hex.as_deref(),
            Some(SERVER_PUB),
            "the substituted public half is the server identity"
        );
        assert_eq!(s.listen_port, Some(51820));
        assert_eq!(s.peers.len(), 1, "the substitution must not read as a peer");
        assert_eq!(s.peers[0].public_key_hex, PEER_A);
    }

    #[test]
    fn a_malformed_substituted_key_is_ignored_rather_than_carried() {
        let raw = "interface_public_key=nonsense\nlisten_port=51820\nerrno=0\n";
        let s = parse_get(raw).unwrap();
        assert!(
            s.public_key_hex.is_none(),
            "a key nobody holds must not reach a client config"
        );
    }

    fn the_interface_private_key_is_never_carried_out_of_the_parser() {
        let raw = get_response();
        assert!(raw.contains("private_key="), "fixture must contain one");
        let s = parse_get(&raw).unwrap();
        let rendered = format!("{s:?}");
        assert!(
            !rendered.contains(SERVER_PRIV),
            "the interface private key must not survive into any struct"
        );
        assert_eq!(
            s.public_key_hex.as_deref(),
            Some(SERVER_PUB),
            "the public half is derived and kept; only the private half is dropped"
        );
    }

    #[test]
    fn a_nonzero_errno_is_an_error_not_a_state() {
        let err = parse_get("errno=-22\n\n").unwrap_err();
        assert!(matches!(err, WgUapiError::Errno(-22)));
    }

    #[test]
    fn a_response_without_errno_is_malformed() {
        // A truncated read must never look like a healthy empty interface.
        let err = parse_get(&format!("public_key={SERVER_PUB}\nlisten_port=51820\n")).unwrap_err();
        assert!(matches!(err, WgUapiError::Malformed(_)));
    }

    #[test]
    fn an_empty_interface_parses_as_empty_not_as_an_error() {
        let s = parse_get("errno=0\n\n").unwrap();
        assert!(s.peers.is_empty() && s.public_key_hex.is_none());
    }

    #[test]
    fn add_peer_never_replaces_the_other_peers() {
        let body = render_add_peer(PEER_A, &["10.9.0.7/32".to_string()]).unwrap();
        assert!(!body.contains("replace_peers"), "issuing a device must not disconnect the others");
        assert!(body.contains(&format!("public_key={PEER_A}")));
        assert!(body.contains("allowed_ip=10.9.0.7/32"));
        assert!(body.contains("replace_allowed_ips=true"), "re-issue must not accumulate CIDRs");
        assert!(body.ends_with("\n\n"), "an operation is terminated by a blank line");
    }

    #[test]
    fn remove_peer_targets_exactly_one_key() {
        let body = render_remove_peer(PEER_B).unwrap();
        assert!(body.contains(&format!("public_key={PEER_B}")));
        assert!(body.contains("remove=true"));
        assert!(!body.contains("replace_peers"));
    }

    #[test]
    fn a_malformed_key_is_refused_before_it_reaches_the_wire() {
        // The daemon would accept "abc" as a different peer rather than fail,
        // silently stranding the device being issued.
        for bad in ["abc", "", &"z".repeat(64), &PEER_A[..63]] {
            assert!(render_add_peer(bad, &[]).is_err(), "accepted {bad:?}");
            assert!(render_remove_peer(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn hex_and_base64_round_trip() {
        let b64 = hex_to_b64(PEER_A).unwrap();
        assert_eq!(b64.len(), 44, "a 32-byte key is 44 base64 characters");
        assert_eq!(b64_to_hex(&b64).unwrap(), PEER_A);
    }

    #[test]
    fn a_base64_key_of_the_wrong_length_is_refused() {
        assert!(b64_to_hex("c2hvcnQ=").is_err());
    }

    #[test]
    fn allocation_skips_every_address_already_routed() {
        let s = parse_get(&get_response()).unwrap();
        assert_eq!(taken_host_octets(&s), vec![2, 5]);
        assert_eq!(
            next_free_client_ip(&s).as_deref(),
            Some("10.9.0.3"),
            "the lowest free address, not a fixed one"
        );
    }

    #[test]
    fn allocation_never_hands_out_the_server_or_the_network_address() {
        let empty = InterfaceState::default();
        assert_eq!(next_free_client_ip(&empty).as_deref(), Some("10.9.0.2"));
    }

    #[test]
    fn a_full_subnet_returns_none_rather_than_a_duplicate() {
        let mut s = InterfaceState::default();
        s.peers = (2u8..=254)
            .map(|n| PeerState {
                public_key_hex: PEER_A.to_string(),
                allowed_ips: vec![format!("10.9.0.{n}/32")],
                last_handshake_unix: None,
                endpoint: None,
                rx_bytes: 0,
                tx_bytes: 0,
            })
            .collect();
        assert_eq!(
            next_free_client_ip(&s), None,
            "a duplicate address would break a working device to serve a new one"
        );
    }

    #[test]
    fn addresses_outside_the_tunnel_subnet_do_not_consume_slots() {
        let mut s = InterfaceState::default();
        s.peers = vec![PeerState {
            public_key_hex: PEER_A.to_string(),
            allowed_ips: vec!["192.168.1.2/32".to_string(), "10.9.0.4/32".to_string()],
            last_handshake_unix: None,
            endpoint: None,
            rx_bytes: 0,
            tx_bytes: 0,
        }];
        assert_eq!(taken_host_octets(&s), vec![4]);
    }

    #[test]
    fn has_peer_matches_on_the_exact_key() {
        let s = parse_get(&get_response()).unwrap();
        assert!(s.has_peer(PEER_A));
        assert!(!s.has_peer(SERVER_PUB), "the interface is not one of its peers");
    }

    #[test]
    fn a_missing_socket_is_its_own_error_not_an_io_failure() {
        let sock = UapiSocket::at("/nonexistent/genaryx-test/wg0.sock");
        assert!(matches!(sock.state().unwrap_err(), WgUapiError::NoSocket(_)));
    }
}
