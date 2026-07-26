//! Operator-facing WireGuard peer provisioning: mint the signed-in operator a
//! fresh peer against THIS box's own WireGuard server, so their laptop or
//! phone reaches the console over the tunnel instead of an SSH forward, and
//! revoke that peer when the device is gone.
//!
//! ## Not the same WireGuard as `commands::remote_wg_connect`
//!
//! [`super::commands::remote_wg_connect`] dials the console OUT to a remote
//! box over its own `wireguard-go`, using a keypair the console owns. This
//! module is the OPPOSITE direction: the server already runs here, and these
//! commands mint a peer so a human's device can dial IN. The two share only
//! the protocol.
//!
//! ## Two backends, chosen by what is actually reachable
//!
//! The console runs in a container on a single-box install, and a container
//! has no `NET_ADMIN`, no `/dev/net/tun` and no `wg` binary. The previous
//! version of this module shelled `wg` unconditionally and therefore refused
//! outright in exactly the deployment the product ships, which left the SSH
//! forward as the real answer while the UI offered a button that could not
//! work.
//!
//! So peer management goes through [`PeerBackend`]:
//!
//! - [`PeerBackend::Uapi`] talks to `wireguard-go`'s userspace API socket. The
//!   privileged sidecar holds the tunnel; this process holds a unix socket and
//!   needs no capability at all. This is the single-box and cluster path.
//! - [`PeerBackend::Shell`] shells `wg` against a kernel interface, for a
//!   console running directly on a box whose WireGuard someone brought up with
//!   `wg-quick`. Unchanged behaviour for that deployment.
//!
//! Resolution is by evidence, never by guessing the environment: a socket that
//! answers wins, then a `wg` binary that runs, then an honest refusal naming
//! both roads not taken.
//!
//! ## Nothing shells for keys or QR any more
//!
//! Keys are Curve25519, generated in memory by
//! [`genaryx_connectors::WgKeypair`] with exactly `wg genkey`'s clamping, and
//! the QR is rendered as SVG in Rust. `wg`, `wg genkey`, `wg pubkey` and
//! `qrencode` are no longer required for the container path at all: three
//! fewer binaries that the image must ship and that can be missing at 2am.
//!
//! ## Addresses are allocated, not assumed
//!
//! The address comes from [`genaryx_connectors::next_free_client_ip`], which
//! reads the live peer list first. The previous fixed `10.9.0.2` meant the
//! second device issued silently superseded the first, because WireGuard keeps
//! one owner per `/32`: the first laptop simply stopped receiving, with
//! nothing anywhere reporting an error.
//!
//! ## The endpoint host is configuration, never a guess
//!
//! A client `.conf` must name the address the device dials back on, and no
//! interface can report it. It comes from `GENARYX_WG_ENDPOINT_HOST`, which
//! `install.sh` writes. When it is absent this refuses and says so: the
//! previous default was one specific box's public IP, so every other
//! deployment issued configs pointing at a machine that was not theirs.
//!
//! ## Side-effect honesty
//!
//! Issuing really adds a live peer, and revoking really removes one. The
//! client's private key is returned exactly once, to the browser that asked,
//! over the console's authenticated transport: never logged, never written to
//! disk, never recoverable afterwards.

use genaryx_connectors::{
    InterfaceState, UapiSocket, WgKeypair, WgUapiError, hex_to_b64, next_free_client_ip,
};
use serde::Serialize;
use std::process::Command;

/// The tunnel subnet's server address: what an issued client routes back
/// through the tunnel, and the address the console answers on from inside it.
const SERVER_TUNNEL_IP: &str = "10.9.0.1";

/// Where the console itself listens, behind the tunnel's TLS terminator.
const CONSOLE_PORT: u16 = 7420;

/// The URL an operator should open once their tunnel is up.
///
/// Read from `GENARYX_WEB_ORIGIN` rather than assembled from the tunnel IP,
/// because those two answers diverged the moment the console moved behind TLS:
/// the plain `http://10.9.0.1:7420` this used to hand out is now a closed port,
/// so the card was telling operators to open an address that refuses
/// connections. The origin is the same value WebAuthn is bound to, which is
/// exactly the address that has to work.
fn console_tunnel_url() -> String {
    match std::env::var("GENARYX_WEB_ORIGIN") {
        Ok(origin) if !origin.trim().is_empty() => origin.trim().to_string(),
        // No TLS configured: the console is served plainly on the tunnel
        // address, which is what the forwarder points at in that case.
        _ => format!("http://{SERVER_TUNNEL_IP}:{CONSOLE_PORT}"),
    }
}

/// `GENARYX_WG_IFACE` override, `wg-op` otherwise. The same name `install.sh`
/// brings the sidecar up with.
fn iface_name() -> String {
    std::env::var("GENARYX_WG_IFACE").unwrap_or_else(|_| "wg-op".to_string())
}

/// Where the daemon answers, and how.
///
/// `GENARYX_WG_UAPI_SOCKET` keeps its meaning: a filesystem path when it looks
/// like one, which is the sidecar case and every deployment that exists today.
/// Anything else is read as `host:port`, which is the daemon in ANOTHER pod,
/// and then two more values are required rather than defaulted:
///
///   GENARYX_WG_UAPI_CERT   the proxy's certificate, pinned as the only root
///   GENARYX_WG_UAPI_TOKEN  the bearer the proxy checks
///
/// Required, not optional, and the reason is worth stating: a missing
/// certificate would otherwise mean "trust nothing", which fails every
/// handshake with an error about the server, and a missing token would mean
/// "send an empty bearer", which fails as an authorisation error. Both read as
/// the far end being broken. Absent configuration is named as absent
/// configuration instead.
fn uapi_endpoint() -> Result<UapiSocket, WgOperatorError> {
    let raw = std::env::var("GENARYX_WG_UAPI_SOCKET").unwrap_or_default();
    if raw.is_empty() {
        return Ok(UapiSocket::for_interface(&iface_name()));
    }
    if raw.starts_with('/') || !raw.contains(':') {
        return Ok(UapiSocket::at(raw));
    }
    let cert = std::env::var("GENARYX_WG_UAPI_CERT").unwrap_or_default();
    let token = std::env::var("GENARYX_WG_UAPI_TOKEN").unwrap_or_default();
    if cert.is_empty() || token.is_empty() {
        let missing = match (cert.is_empty(), token.is_empty()) {
            (true, true) => "GENARYX_WG_UAPI_CERT and GENARYX_WG_UAPI_TOKEN are",
            (true, false) => "GENARYX_WG_UAPI_CERT is",
            _ => "GENARYX_WG_UAPI_TOKEN is",
        };
        return Err(WgOperatorError::Misconfigured {
            message: format!(
                "GENARYX_WG_UAPI_SOCKET is {raw}, which is a network endpoint rather than a \
                 path, but {missing} not set. Reaching a tunnel daemon in another pod needs \
                 the proxy's certificate to pin and the bearer it checks; both are written \
                 into this console's Secret by install.sh."
            ),
        });
    }
    Ok(UapiSocket::tls(raw, cert, token))
}

// ============================================================================
// DTOs
// ============================================================================

/// [`operator_wg_config`]'s return: everything the frontend needs to show the
/// QR and offer the `.conf` download in one round trip.
#[derive(Debug, Clone, Serialize)]
pub struct RemoteWgOperatorConfigDto {
    /// The complete client `.conf` TEXT, private key included: what the
    /// Download button saves and what the QR encodes.
    pub conf: String,
    /// The QR as an inline SVG document. The frontend renders it directly or
    /// wraps it in a `data:image/svg+xml` URI; either way no binary image
    /// encoder is involved.
    pub qr_svg: String,
    pub client_ip: String,
    /// `host:port` the client dials.
    pub endpoint: String,
    pub server_public_key: String,
    /// The peer's public key, base64. The handle a later revoke names, and the
    /// only part of the issued identity worth keeping.
    pub peer_public_key: String,
    /// Where the console answers once the tunnel is up.
    pub console_tunnel_url: String,
}

/// One currently-authorized device.
#[derive(Debug, Clone, Serialize)]
pub struct RemoteWgPeerDto {
    /// Base64, the form `wg show` prints and a revoke names.
    pub public_key: String,
    pub allowed_ips: Vec<String>,
    /// Unix seconds of the last completed handshake, `None` if this device has
    /// never connected. A peer that was issued and never used looks exactly
    /// like one that was issued to the wrong person.
    pub last_handshake_unix: Option<u64>,
    pub endpoint: Option<String>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteWgPeersDto {
    pub iface: String,
    pub server_public_key: Option<String>,
    pub listen_port: Option<u16>,
    /// Which backend answered, so the UI can say where this came from rather
    /// than implying one uniform mechanism.
    pub backend: &'static str,
    pub peers: Vec<RemoteWgPeerDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteWgRevokeDto {
    pub public_key: String,
    /// False when the key was not configured to begin with. A revoke of an
    /// absent peer still succeeds (the end state is what was asked for), but
    /// saying which happened keeps the audit trail honest.
    pub was_present: bool,
    pub remaining_peers: usize,
}

// ============================================================================
// errors
// ============================================================================

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WgOperatorError {
    /// No WireGuard server this console can reach, by either backend. Carries
    /// the reason rather than a raw `command not found`, which reads like a
    /// missing package and sends the operator to install something that would
    /// change nothing.
    ServerNotConfigured { iface: String, message: String },
    /// Reachable, but the operation failed.
    Exec { message: String },
    /// The deployment is missing a value only it can supply.
    Misconfigured { message: String },
    /// Every address in the tunnel subnet is already assigned.
    SubnetExhausted { message: String },
}

fn exec_err(message: String) -> WgOperatorError {
    WgOperatorError::Exec { message }
}

fn not_configured(iface: &str, detail: impl std::fmt::Display) -> WgOperatorError {
    WgOperatorError::ServerNotConfigured {
        iface: iface.to_string(),
        message: format!("{detail}"),
    }
}

impl From<WgUapiError> for WgOperatorError {
    fn from(e: WgUapiError) -> Self {
        match e {
            WgUapiError::NoSocket(p) => WgOperatorError::ServerNotConfigured {
                iface: iface_name(),
                message: format!(
                    "the WireGuard tunnel is not running: no UAPI socket at {}. \
                     On a single-box install this is the `wg` service in compose.yaml; \
                     start it with `docker compose up -d wg`.",
                    p
                ),
            },
            other => exec_err(other.to_string()),
        }
    }
}

// ============================================================================
// backend selection
// ============================================================================

/// How this process reaches the WireGuard server.
#[derive(Debug, Clone)]
pub enum PeerBackend {
    /// `wireguard-go`'s userspace API socket, held by a sidecar.
    Uapi(UapiSocket),
    /// `wg` against a kernel interface on this same host.
    Shell { iface: String },
}

impl PeerBackend {
    pub fn label(&self) -> &'static str {
        match self {
            PeerBackend::Uapi(_) => "uapi",
            PeerBackend::Shell { .. } => "wg",
        }
    }
}

/// Whether this console is inside a container. Not used to refuse any more,
/// only to word the refusal: the UAPI backend works perfectly well in one.
fn containerised() -> Option<&'static str> {
    if std::env::var_os("KUBERNETES_SERVICE_HOST").is_some() {
        return Some("a Kubernetes pod");
    }
    if std::path::Path::new("/.dockerenv").exists()
        || std::path::Path::new("/run/.containerenv").exists()
    {
        return Some("a container");
    }
    None
}

/// Pick a backend by evidence: a UAPI socket that exists, else a `wg` binary
/// that runs, else refuse and name both.
fn resolve_backend() -> Result<PeerBackend, WgOperatorError> {
    let iface = iface_name();
    let sock = uapi_endpoint()?;
    if sock.exists() {
        return Ok(PeerBackend::Uapi(sock));
    }
    // A NETWORK endpoint that did not answer must not fall through. The `wg`
    // fallback below exists for a host install where the binary is genuinely
    // present; on a console configured to reach another pod, silently shelling
    // out to a kernel interface that does not exist there turns "the proxy is
    // down" into "no WireGuard server this console can reach", naming a socket
    // path nobody configured. Refuse, and name what was actually tried.
    if sock.is_network() {
        return Err(not_configured(
            &iface,
            format!(
                "the tunnel daemon at {} did not answer. It is in another pod, so there is \
                 no local interface to fall back to: check that the wg pod is running and \
                 that a NetworkPolicy admits this one.",
                sock.describe()
            ),
        ));
    }
    if Command::new("wg").arg("--version").output().is_ok() {
        return Ok(PeerBackend::Shell { iface });
    }
    let where_ = containerised().unwrap_or("this host");
    Err(not_configured(
        &iface,
        format!(
            "no WireGuard server this console can reach. It looked for a userspace API \
             socket at {} (the `wg` sidecar service, which is how {where_} is meant to \
             reach one) and for a `wg` binary on PATH (a kernel interface on this host). \
             Neither answered. On a single-box install: `docker compose up -d wg`.",
            sock.describe()
        ),
    ))
}

// ============================================================================
// backend operations
// ============================================================================

/// One `<cli> <args>` invocation's trimmed stdout on a clean exit.
fn run(cli: &str, args: &[&str]) -> Result<String, WgOperatorError> {
    let out = Command::new(cli).args(args).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            exec_err(format!("{cli}: command not found (is it installed and on PATH?)"))
        } else {
            exec_err(format!("could not run {cli}: {e}"))
        }
    })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(exec_err(format!(
            "{cli} {} exited {}: {stderr}",
            args.join(" "),
            out.status.code().unwrap_or(-1)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Parse `wg show <iface> dump` into the same shape the UAPI parser produces,
/// so everything downstream is backend-agnostic.
///
/// The first line is the interface (private key, public key, listen port,
/// fwmark); every later line is a peer (public key, preshared key, endpoint,
/// allowed ips, latest handshake, rx, tx, keepalive), tab-separated.
fn parse_wg_dump(dump: &str) -> InterfaceState {
    use genaryx_connectors::PeerState;
    let mut state = InterfaceState::default();
    for (i, line) in dump.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let f: Vec<&str> = line.split('\t').collect();
        if i == 0 {
            // Field 0 is the interface's PRIVATE key and is deliberately not
            // read: nothing here needs it, so it never enters a struct.
            if let Some(pk) = f.get(1).filter(|s| !s.is_empty() && **s != "(none)") {
                state.public_key_hex = genaryx_connectors::b64_to_hex(pk).ok();
            }
            state.listen_port = f.get(2).and_then(|s| s.parse().ok());
            continue;
        }
        let Some(pk_b64) = f.first().filter(|s| !s.is_empty()) else {
            continue;
        };
        let Ok(public_key_hex) = genaryx_connectors::b64_to_hex(pk_b64) else {
            continue;
        };
        state.peers.push(PeerState {
            public_key_hex,
            allowed_ips: f
                .get(3)
                .filter(|s| !s.is_empty() && **s != "(none)")
                .map(|s| s.split(',').map(|c| c.trim().to_string()).collect())
                .unwrap_or_default(),
            last_handshake_unix: f.get(4).and_then(|s| s.parse::<u64>().ok()).filter(|n| *n != 0),
            endpoint: f
                .get(2)
                .filter(|s| !s.is_empty() && **s != "(none)")
                .map(|s| s.to_string()),
            rx_bytes: f.get(5).and_then(|s| s.parse().ok()).unwrap_or(0),
            tx_bytes: f.get(6).and_then(|s| s.parse().ok()).unwrap_or(0),
        });
    }
    state
}

fn read_state(backend: &PeerBackend) -> Result<InterfaceState, WgOperatorError> {
    match backend {
        PeerBackend::Uapi(sock) => Ok(sock.state()?),
        PeerBackend::Shell { iface } => {
            let dump = run("wg", &["show", iface, "dump"]).map_err(|e| match e {
                WgOperatorError::Exec { message } => not_configured(
                    iface,
                    format!("interface '{iface}' is not up ({message})"),
                ),
                other => other,
            })?;
            Ok(parse_wg_dump(&dump))
        }
    }
}

fn add_peer(
    backend: &PeerBackend,
    public_key_hex: &str,
    client_ip: &str,
) -> Result<(), WgOperatorError> {
    let allowed = format!("{client_ip}/32");
    match backend {
        PeerBackend::Uapi(sock) => Ok(sock.add_peer(public_key_hex, &[allowed])?),
        PeerBackend::Shell { iface } => {
            let b64 = hex_to_b64(public_key_hex).map_err(|e| exec_err(e.to_string()))?;
            run("wg", &["set", iface, "peer", &b64, "allowed-ips", &allowed])?;
            // Best effort: the peer is already live either way, so a failure
            // to persist must not fail the command.
            if let Err(e) = run("wg-quick", &["save", iface]) {
                eprintln!("genaryx: wg-quick save {iface} failed, peer is live regardless: {e:?}");
            }
            Ok(())
        }
    }
}

fn remove_peer(backend: &PeerBackend, public_key_hex: &str) -> Result<(), WgOperatorError> {
    match backend {
        PeerBackend::Uapi(sock) => Ok(sock.remove_peer(public_key_hex)?),
        PeerBackend::Shell { iface } => {
            let b64 = hex_to_b64(public_key_hex).map_err(|e| exec_err(e.to_string()))?;
            run("wg", &["set", iface, "peer", &b64, "remove"])?;
            if let Err(e) = run("wg-quick", &["save", iface]) {
                eprintln!("genaryx: wg-quick save {iface} failed, peer is gone regardless: {e:?}");
            }
            Ok(())
        }
    }
}

// ============================================================================
// rendering
// ============================================================================

/// The address a device dials back on. Configuration only: no interface can
/// report it, and guessing it hands the operator a config pointing at someone
/// else's machine.
fn endpoint_host() -> Result<String, WgOperatorError> {
    match std::env::var("GENARYX_WG_ENDPOINT_HOST") {
        Ok(h) if !h.trim().is_empty() => Ok(h.trim().to_string()),
        _ => Err(WgOperatorError::Misconfigured {
            message: "GENARYX_WG_ENDPOINT_HOST is not set, so there is no address to put in \
                      the client config. It is the public address of THIS box, the one a \
                      phone dials from outside; nothing on the interface can report it. \
                      install.sh writes it into .env; set it there and restart the console."
                .to_string(),
        }),
    }
}

/// The client `.conf`: what the Download button saves and the QR encodes.
fn render_conf(
    client_private_key_b64: &str,
    client_ip: &str,
    server_public_key_b64: &str,
    endpoint: &str,
) -> String {
    format!(
        "[Interface]\nPrivateKey = {client_private_key_b64}\nAddress = {client_ip}/32\n\n\
         [Peer]\nPublicKey = {server_public_key_b64}\nEndpoint = {endpoint}\n\
         AllowedIPs = {SERVER_TUNNEL_IP}/32\nPersistentKeepalive = 25\n"
    )
}

/// The config as a scannable QR, SVG. Pure Rust: no `qrencode`, no PNG codec.
fn render_qr_svg(text: &str) -> Result<String, WgOperatorError> {
    use qrcode::QrCode;
    use qrcode::render::svg;
    let code = QrCode::new(text.as_bytes())
        .map_err(|e| exec_err(format!("could not encode the config as a QR: {e}")))?;
    let doc = code
        .render::<svg::Color>()
        .min_dimensions(240, 240)
        .quiet_zone(true)
        .build();
    // The renderer emits a standalone XML document. The frontend inlines this
    // into HTML, where a leading `<?xml ... ?>` declaration is invalid, so the
    // prolog is trimmed and only the `<svg>` element is handed out.
    Ok(match doc.find("<svg") {
        Some(i) => doc[i..].to_string(),
        None => doc,
    })
}

// ============================================================================
// audit
// ============================================================================

/// Journal one peer-lifecycle action onto the console's own command journal
/// and the event bus.
///
/// Issuing a peer hands out a road into the control plane and revoking one
/// takes an operator's access away; both already require a passkey. Until this
/// existed, the strongest evidence the console produces - a human physically
/// confirming that grant on their own authenticator - was written to a log line
/// and nowhere else, so an evidence pack could show a kill but not who was
/// given the keys. `money_kill_run` and `policy_decide_approval` have journaled
/// their whole lives; this is the same act and was the odd one out.
///
/// Best-effort by design: the peer is already live on the interface by the time
/// this runs, so a journal failure must not turn a completed grant into a
/// reported error. It is logged loudly instead, because an unjournaled
/// privileged action is exactly the thing an operator needs to know about.
fn journal_peer_action(bus: Option<&crate::money::state::BusHandle>, action: &str, target: &str, detail: String) {
    let Some(bus) = bus else {
        eprintln!("genaryx: {action} on {target} was NOT journaled: no live event bus");
        return;
    };
    // The signature the WebAuthn gate put in scope for THIS request, so the
    // record names the credential the human actually touched. Falls back to the
    // same honest label the rest of the console uses when no passkey is
    // enrolled - never a fabricated one.
    let (sig_alg, sig_fpr) =
        crate::console_actor::signature_or("software-signed", "software-signed");
    let org_domain = std::env::var("GENARYX_ORG_DOMAIN").unwrap_or_else(|_| "local".to_string());
    let operator = crate::console_actor::operator_or(&format!("user://{org_domain}/operator"));
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "console".to_string());

    let rec = genaryx_core::command::CommandRecord {
        operator,
        env: org_domain.clone(),
        action: action.to_string(),
        target: target.to_string(),
        params: serde_json::json!({}),
        // Not a break-glass override: this is the sanctioned path, gated by a
        // passkey rather than bypassing anything.
        decision: "allow".to_string(),
        sig_alg,
        sig_fpr,
        http_status: 200,
        verify_result: detail,
    };
    match genaryx_core::store::Store::open(&bus.store_db_path) {
        Ok(store) => {
            if let Err(e) =
                genaryx_core::command::record(&store, &bus.console_events_path, &org_domain, &host, &rec)
            {
                eprintln!("genaryx: {action} on {target} succeeded but was NOT journaled: {e}");
            }
        }
        Err(e) => eprintln!("genaryx: {action} on {target} succeeded but was NOT journaled: {e}"),
    }
}



fn issue_blocking() -> Result<RemoteWgOperatorConfigDto, WgOperatorError> {
    let backend = resolve_backend()?;
    let iface = iface_name();
    // Resolved BEFORE minting anything: failing after a peer is live would
    // leave an authorized device nobody holds a config for.
    let host = endpoint_host()?;

    let state = read_state(&backend)?;
    let server_public_key = state.public_key_b64().ok_or_else(|| {
        not_configured(
            &iface,
            format!("interface '{iface}' reports no public key, so it is not a running server"),
        )
    })?;
    let listen_port = state.listen_port.ok_or_else(|| {
        not_configured(&iface, format!("interface '{iface}' reports no listen port"))
    })?;

    let client_ip = next_free_client_ip(&state).ok_or_else(|| WgOperatorError::SubnetExhausted {
        message: format!(
            "every address in 10.9.0.0/24 is already assigned ({} peers). \
             Revoke a device before issuing another; handing out a duplicate address \
             would break a working device to serve a new one.",
            state.peers.len()
        ),
    })?;

    let keypair = WgKeypair::generate()
        .map_err(|e| exec_err(format!("could not generate a client keypair: {e}")))?;
    let client_public_hex = keypair.public_hex();
    let client_private_b64 =
        hex_to_b64(&keypair.private_hex()).map_err(|e| exec_err(e.to_string()))?;
    let client_public_b64 = hex_to_b64(&client_public_hex).map_err(|e| exec_err(e.to_string()))?;

    // Authorize before rendering: the operator must never walk away with a
    // config that was never going to connect.
    add_peer(&backend, &client_public_hex, &client_ip)?;

    let endpoint = format!("{host}:{listen_port}");
    let conf = render_conf(&client_private_b64, &client_ip, &server_public_key, &endpoint);
    let qr_svg = render_qr_svg(&conf)?;

    Ok(RemoteWgOperatorConfigDto {
        conf,
        qr_svg,
        client_ip,
        endpoint,
        server_public_key,
        peer_public_key: client_public_b64,
        console_tunnel_url: console_tunnel_url(),
    })
}

fn peers_blocking() -> Result<RemoteWgPeersDto, WgOperatorError> {
    let backend = resolve_backend()?;
    let state = read_state(&backend)?;
    Ok(RemoteWgPeersDto {
        iface: iface_name(),
        server_public_key: state.public_key_b64(),
        listen_port: state.listen_port,
        backend: backend.label(),
        peers: state
            .peers
            .iter()
            .map(|p| RemoteWgPeerDto {
                public_key: hex_to_b64(&p.public_key_hex).unwrap_or_else(|_| p.public_key_hex.clone()),
                allowed_ips: p.allowed_ips.clone(),
                last_handshake_unix: p.last_handshake_unix,
                endpoint: p.endpoint.clone(),
                rx_bytes: p.rx_bytes,
                tx_bytes: p.tx_bytes,
            })
            .collect(),
    })
}

fn revoke_blocking(public_key_b64: &str) -> Result<RemoteWgRevokeDto, WgOperatorError> {
    let backend = resolve_backend()?;
    let hex = genaryx_connectors::b64_to_hex(public_key_b64).map_err(|e| {
        WgOperatorError::Misconfigured {
            message: format!("not a WireGuard public key: {e}"),
        }
    })?;
    let before = read_state(&backend)?;
    let was_present = before.has_peer(&hex);
    remove_peer(&backend, &hex)?;
    let after = read_state(&backend)?;
    Ok(RemoteWgRevokeDto {
        public_key: public_key_b64.to_string(),
        was_present,
        remaining_peers: after.peers.len(),
    })
}

/// Mint the signed-in operator a fresh WireGuard peer against this box's own
/// server. Runs the blocking work on the pool, like every other connector call.
pub async fn operator_wg_config(
    bus: Option<&crate::money::state::BusHandle>,
) -> Result<RemoteWgOperatorConfigDto, WgOperatorError> {
    // The journal write happens on THIS task, not inside the blocking one:
    // `console_actor`'s signature and operator live in a task-local scope that
    // `spawn_blocking` does not carry across, so recording in there would lose
    // the very passkey attribution this exists to capture.
    let issued = tokio::task::spawn_blocking(issue_blocking)
        .await
        .unwrap_or_else(|e| Err(exec_err(format!("operator wg config task failed to run: {e}"))))?;
    journal_peer_action(
        bus,
        "console.issue_wg_peer",
        &issued.peer_public_key,
        format!("issued:{}", issued.client_ip),
    );
    Ok(issued)
}

/// Every device currently authorized on the tunnel.
pub async fn operator_wg_peers() -> Result<RemoteWgPeersDto, WgOperatorError> {
    tokio::task::spawn_blocking(peers_blocking)
        .await
        .unwrap_or_else(|e| Err(exec_err(format!("operator wg peers task failed to run: {e}"))))
}

/// Revoke one device. The key stops completing a handshake as soon as the
/// daemon applies it.
pub async fn operator_wg_revoke(
    public_key_b64: String,
    bus: Option<&crate::money::state::BusHandle>,
) -> Result<RemoteWgRevokeDto, WgOperatorError> {
    let key_for_record = public_key_b64.clone();
    let revoked = tokio::task::spawn_blocking(move || revoke_blocking(&public_key_b64))
        .await
        .unwrap_or_else(|e| Err(exec_err(format!("operator wg revoke task failed to run: {e}"))))?;
    journal_peer_action(
        bus,
        "console.revoke_wg_peer",
        &key_for_record,
        format!(
            "revoked:was_present={} remaining={}",
            revoked.was_present, revoked.remaining_peers
        ),
    );
    Ok(revoked)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVER_B64: &str = "6PLBoLPUlYZ3obLD1OX2BxgpOktcbX6PkBI0Vniivt8=";
    const PEER_B64: &str = "qhG7Iswz3UTuVf9md4iZABEiM0RVZneImZAKq7zN3v8=";

    #[test]
    fn a_wg_dump_parses_into_the_same_shape_as_uapi() {
        let dump = format!(
            "PRIVATEKEYPRIVATEKEYPRIVATEKEYPRIVATEKEYPRI=\t{SERVER_B64}\t51820\toff\n\
             {PEER_B64}\t(none)\t203.0.113.7:54321\t10.9.0.4/32\t1769000000\t4096\t8192\t25\n"
        );
        let state = parse_wg_dump(&dump);
        assert_eq!(state.listen_port, Some(51820));
        assert_eq!(state.public_key_b64().as_deref(), Some(SERVER_B64));
        assert_eq!(state.peers.len(), 1, "the interface line must not become a peer");
        let p = &state.peers[0];
        assert_eq!(p.allowed_ips, vec!["10.9.0.4/32"]);
        assert_eq!(p.last_handshake_unix, Some(1769000000));
        assert_eq!((p.rx_bytes, p.tx_bytes), (4096, 8192));
    }

    #[test]
    fn a_dump_never_carries_the_interface_private_key_into_state() {
        let dump = format!("PRIVATEKEYPRIVATEKEYPRIVATEKEYPRIVATEKEYPRI=\t{SERVER_B64}\t51820\toff\n");
        let rendered = format!("{:?}", parse_wg_dump(&dump));
        assert!(!rendered.contains("PRIVATEKEY"));
    }

    #[test]
    fn a_never_connected_peer_reports_none_not_the_epoch() {
        let dump = format!(
            "priv=\t{SERVER_B64}\t51820\toff\n{PEER_B64}\t(none)\t(none)\t10.9.0.2/32\t0\t0\t0\toff\n"
        );
        let state = parse_wg_dump(&dump);
        assert_eq!(state.peers[0].last_handshake_unix, None);
        assert_eq!(state.peers[0].endpoint, None, "(none) is not an endpoint");
    }

    #[test]
    fn the_conf_routes_only_the_tunnel_not_the_whole_internet() {
        let conf = render_conf("PRIV=", "10.9.0.3", SERVER_B64, "198.51.100.9:51820");
        assert!(
            conf.contains(&format!("AllowedIPs = {SERVER_TUNNEL_IP}/32")),
            "a 0.0.0.0/0 config would silently route the operator's whole device through the box"
        );
        assert!(conf.contains("Address = 10.9.0.3/32"));
        assert!(conf.contains("Endpoint = 198.51.100.9:51820"));
        assert!(conf.contains("PersistentKeepalive = 25"));
    }

    #[test]
    fn a_network_endpoint_without_its_certificate_or_bearer_refuses_by_name() {
        // Absent configuration must be named as absent configuration. Without
        // this, a missing certificate means "trust nothing" and fails every
        // handshake with an error about the SERVER, and a missing bearer sends
        // an empty one and fails as authorisation. Both read as the far end
        // being broken, which is an hour spent on the wrong pod.
        //
        // Safety: single-threaded test, every var restored below.
        let saved: Vec<_> = ["GENARYX_WG_UAPI_SOCKET", "GENARYX_WG_UAPI_CERT", "GENARYX_WG_UAPI_TOKEN"]
            .iter()
            .map(|k| (*k, std::env::var(k).ok()))
            .collect();
        unsafe {
            std::env::set_var("GENARYX_WG_UAPI_SOCKET", "wg.agent-stack:9090");
            std::env::remove_var("GENARYX_WG_UAPI_CERT");
            std::env::remove_var("GENARYX_WG_UAPI_TOKEN");
        }

        let err = match uapi_endpoint().unwrap_err() {
            WgOperatorError::Misconfigured { message } => message,
            other => panic!("absent configuration must be Misconfigured, got {other:?}"),
        };

        unsafe {
            for (k, v) in saved {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
        assert!(err.contains("GENARYX_WG_UAPI_CERT"), "names the cert: {err}");
        assert!(err.contains("GENARYX_WG_UAPI_TOKEN"), "names the token: {err}");
    }

    #[test]
    fn a_path_is_still_a_path_and_never_read_as_an_address() {
        // The same variable carries both shapes, so the discrimination has to
        // be exact: every deployment that exists today passes a path.
        let saved = std::env::var("GENARYX_WG_UAPI_SOCKET").ok();
        unsafe { std::env::set_var("GENARYX_WG_UAPI_SOCKET", "/var/run/wireguard/console.sock") };
        let sock = uapi_endpoint().expect("a path needs no certificate");
        assert!(!sock.is_network(), "a path must not become a network endpoint");
        unsafe {
            match saved {
                Some(v) => std::env::set_var("GENARYX_WG_UAPI_SOCKET", v),
                None => std::env::remove_var("GENARYX_WG_UAPI_SOCKET"),
            }
        }
    }

    #[test]
    fn a_missing_endpoint_host_refuses_rather_than_guessing() {
        // Safety: single-threaded test, and the var is restored below.
        let saved = std::env::var("GENARYX_WG_ENDPOINT_HOST").ok();
        unsafe { std::env::remove_var("GENARYX_WG_ENDPOINT_HOST") };
        let err = endpoint_host().unwrap_err();
        assert!(matches!(err, WgOperatorError::Misconfigured { .. }));
        unsafe { std::env::set_var("GENARYX_WG_ENDPOINT_HOST", "  198.51.100.9  ") };
        assert_eq!(endpoint_host().unwrap(), "198.51.100.9", "trimmed, not raw");
        match saved {
            Some(v) => unsafe { std::env::set_var("GENARYX_WG_ENDPOINT_HOST", v) },
            None => unsafe { std::env::remove_var("GENARYX_WG_ENDPOINT_HOST") },
        }
    }

    #[test]
    fn the_qr_encodes_the_whole_config_and_is_an_svg() {
        let conf = render_conf("PRIV=", "10.9.0.3", SERVER_B64, "198.51.100.9:51820");
        let svg = render_qr_svg(&conf).unwrap();
        assert!(svg.starts_with("<svg"), "must be inline SVG, no image codec involved");
        assert!(svg.len() > 500, "a real QR, not an empty canvas");
    }

    #[test]
    fn the_backend_label_names_which_road_answered() {
        assert_eq!(PeerBackend::Uapi(UapiSocket::at("/x")).label(), "uapi");
        assert_eq!(PeerBackend::Shell { iface: "wg-op".into() }.label(), "wg");
    }
}
