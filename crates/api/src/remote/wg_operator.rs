//! Operator-facing WireGuard client provisioning (`remote_operator_wg_config`,
//! `crate::remote::commands`'s thin wrapper of the same name): mint the
//! signed-in operator a fresh WireGuard peer against THIS box's own kernel
//! WireGuard server, so their laptop or phone can reach the console over the
//! tunnel instead of SSH.
//!
//! ## Not the same WireGuard as `commands::remote_wg_connect`
//!
//! [`super::commands::remote_wg_connect`] and its [`super::state::TunnelState`]
//! dial the console OUT to a remote Hetzner box, over `wireguard-go`
//! (userspace), using a keypair this console itself owns
//! (`RemoteClient::console_keypair`). This module is the OPPOSITE direction:
//! THIS box already runs the WireGuard SERVER (a kernel interface brought up
//! by the box operator's own `wg-quick`, entirely outside this app's
//! lifecycle - genaryx never creates or tears it down), and
//! [`operator_wg_config`] mints a NEW peer, the signed-in human's own
//! laptop or phone, so it can dial IN. Nothing here reuses `RemoteClient`'s
//! cells: there is no long-lived connection to hold, just a handful of
//! `wg`/`wg-quick`/`qrencode` shells against the box's already-running
//! interface, so this command takes no managed state at all - mirrors
//! `commands::remote_hetzner_list`'s identical "stateless connector" shape.
//!
//! ## Reading the server identity live, never guessed
//!
//! The server's public key and listen port are read straight off the
//! running interface (`wg show <iface> public-key`/`listen-port`) rather
//! than from any saved config, so a rotated key or a changed port is
//! reflected immediately. Only the endpoint HOST (the box's public IP,
//! which `wg show` cannot report) and the interface name are configurable,
//! via `GENARYX_WG_ENDPOINT_HOST`/`GENARYX_WG_IFACE`, each defaulting to
//! this specific box's own real value - see [`endpoint_host`]/[`iface_name`],
//! so this stays usable, not hostile, on a differently-addressed deployment.
//!
//! ## Client IP allocation (v1: fixed, documented, not hidden)
//!
//! [`next_client_ip`] always hands out `10.9.0.2` - the simplest acceptable
//! v1 for a single operator device rather than a real allocator that scans
//! `wg show <iface> allowed-ips` for already-taken addresses first (the way
//! `provisioning/new-device.sh` does for its own, differently-addressed,
//! class of peer). See that function's own doc comment for the consequence
//! this simplification has on a second issue.
//!
//! ## Never a private key touches a file
//!
//! `wg set <iface> private-key <file>` is known to fail on this box with a
//! permission error under AppArmor when the file lives under `/root`. This
//! module never makes that call at all: the SERVER's own private key is
//! never touched (only its PUBLIC key is read, via `wg show`), and the fresh
//! CLIENT keypair this command mints is generated and consumed entirely in
//! memory - `wg genkey`'s stdout piped straight into `wg pubkey`'s stdin
//! ([`generate_client_keypair`]), no temp file ever created for either half
//! of it. Adding the new peer to the live server (`wg set <iface> peer <pub>
//! allowed-ips <ip>/32`, [`add_peer`]) takes the public key as a plain
//! argument, which needs no file either.
//!
//! ## Where this cannot work, and says so
//!
//! Every shell below targets the HOST's kernel interface, so a console running
//! in a container or a Kubernetes pod has neither the binaries nor the network
//! namespace to do any of it. [`containerised`] detects that BEFORE shelling
//! anything and returns [`WgOperatorError::ServerNotConfigured`] with the
//! reason and the alternative, because the raw failure
//! (`wg: command not found`) reads like a missing package and sends the
//! operator to install one that would change nothing. A cluster-native entry
//! point for the console is a separate component, not this command.
//!
//! ## Side-effect honesty
//!
//! [`operator_wg_config`] really adds a peer to the live interface - it is
//! not a preview or a dry run (see [`add_peer`]). Persisting that peer to
//! `/etc/wireguard/<iface>.conf` (`wg-quick save`, [`persist_best_effort`])
//! is attempted afterward but is deliberately best-effort: a failure there
//! is logged and swallowed, never turned into a command failure, because
//! the peer is already live on the running interface either way.
//!
//! The client's private key is returned to the caller exactly once, inside
//! the DTO the operator's own browser receives over the console's existing
//! authenticated transport - never logged, never written to disk by this
//! module.

use serde::Serialize;
use std::io::Write as _;
use std::process::{Command, Stdio};

/// The tunnel subnet's server address - what the issued client's own
/// `AllowedIPs` names (the one address it routes back through the tunnel),
/// never itself read off `wg show` (a listen interface reports no notion of
/// "the subnet base").
const SERVER_TUNNEL_IP: &str = "10.9.0.1";

/// Where the console itself listens, reachable at this address once the
/// tunnel is up.
const CONSOLE_PORT: u16 = 7420;

/// `GENARYX_WG_IFACE` override; this box's real kernel WireGuard server
/// interface otherwise.
fn iface_name() -> String {
    std::env::var("GENARYX_WG_IFACE").unwrap_or_else(|_| "wg-op".to_string())
}

/// `GENARYX_WG_ENDPOINT_HOST` override; this box's real public IP otherwise -
/// `wg show` has no way to report the address a client dials back in on, so
/// this is the one piece of the client config that cannot be read live.
fn endpoint_host() -> String {
    std::env::var("GENARYX_WG_ENDPOINT_HOST").unwrap_or_else(|_| "46.225.171.155".to_string())
}

/// The next free client tunnel address. ALWAYS `10.9.0.2` for now - the
/// simplest acceptable v1 (a real allocator would scan `wg show <iface>
/// allowed-ips` for taken addresses first, the way `provisioning/
/// new-device.sh` does for its own peer class). One consequence worth
/// stating rather than hiding: re-issuing a config mints a brand new
/// keypair at this SAME address, which silently supersedes whatever peer
/// previously held it (WireGuard's allowed-ips routing keeps only one owner
/// per exact `/32`) - harmless for the single-operator demo this ships for,
/// but a real multi-seat rollout needs the scanning allocator instead.
/// Deliberately a `fn`, not a `const`, so that later allocator slots in
/// without changing any call site's shape.
fn next_client_ip() -> String {
    "10.9.0.2".to_string()
}

// ============================================================================
// DTO
// ============================================================================

/// [`operator_wg_config`]'s return - everything the frontend needs to render
/// the QR code and offer the `.conf` download in one round trip.
#[derive(Debug, Clone, Serialize)]
pub struct RemoteWgOperatorConfigDto {
    /// The complete client `wg0.conf` TEXT, private key included - what the
    /// Download button saves verbatim and what the QR code encodes.
    pub conf: String,
    /// A QR-code PNG of `conf`, base64-encoded, ready for
    /// `<img src="data:image/png;base64,...">`.
    pub qr_png_base64: String,
    pub client_ip: String,
    /// `host:port` the client dials.
    pub endpoint: String,
    pub server_public_key: String,
    /// Where the console answers once the tunnel is up.
    pub console_tunnel_url: String,
}

// ============================================================================
// errors
// ============================================================================

/// Every failure mode [`operator_wg_config`] can surface, fail-closed -
/// mirrors `genaryx_connectors::cloud_cli::CloudCliError`'s "shell an
/// installed CLI" shape, collapsed to the two outcomes this feature's own
/// caller ([`super::commands::RemoteError`], via the `From` impl in
/// `commands.rs`) needs to tell apart.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WgOperatorError {
    /// The box's kernel WireGuard server (`iface`) is not up, so there is no
    /// live server key/port to hand a fresh peer - a normal, honest outcome
    /// on a box where WireGuard has not been brought up yet, never a panic.
    ServerNotConfigured { iface: String, message: String },
    /// `wg`/`wg-quick`/`qrencode` shelled, ran, and exited nonzero (or could
    /// not be spawned at all) for any other reason.
    Exec { message: String },
}

fn exec_err(message: String) -> WgOperatorError {
    WgOperatorError::Exec { message }
}

// ============================================================================
// shelling wg / wg-quick / qrencode
// ============================================================================

/// One `<cli> <args>` invocation's captured, trimmed stdout on a clean exit -
/// mirrors `genaryx_connectors::cloud_cli::run_cli`'s spawn/exit
/// classification, simplified to this module's own two-variant error (no
/// separate "not authenticated" case: none of `wg`/`wg-quick`/`qrencode`
/// have one).
fn run(cli: &str, args: &[&str]) -> Result<String, WgOperatorError> {
    let out = Command::new(cli).args(args).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            exec_err(format!(
                "{cli}: command not found (is it installed and on PATH?)"
            ))
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

/// `wg pubkey`, fed `private_key` on stdin rather than a file - the
/// `wg genkey | wg pubkey` idiom, minus the shell pipe (see this module's
/// doc comment on why no key ever touches disk here).
fn pubkey_from_private(private_key: &str) -> Result<String, WgOperatorError> {
    let mut child = Command::new("wg")
        .arg("pubkey")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                exec_err("wg: command not found (is it installed and on PATH?)".to_string())
            } else {
                exec_err(format!("could not run wg pubkey: {e}"))
            }
        })?;

    // Scoped so `stdin` drops (closing the pipe) before `wait_with_output`
    // below reads: `wg pubkey` reads exactly one key from stdin until EOF,
    // then exits.
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| exec_err("wg pubkey: could not open its stdin pipe".to_string()))?;
        stdin
            .write_all(private_key.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .map_err(|e| {
                exec_err(format!(
                    "wg pubkey: could not write the private key to stdin: {e}"
                ))
            })?;
    }

    let out = child
        .wait_with_output()
        .map_err(|e| exec_err(format!("wg pubkey: could not read its output: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(exec_err(format!(
            "wg pubkey exited {}: {stderr}",
            out.status.code().unwrap_or(-1)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A fresh WireGuard client keypair: `wg genkey`'s stdout piped directly into
/// `wg pubkey`'s stdin, entirely in memory.
fn generate_client_keypair() -> Result<(String, String), WgOperatorError> {
    let private_key = run("wg", &["genkey"])?;
    let public_key = pubkey_from_private(&private_key)?;
    Ok((private_key, public_key))
}

/// Read the live server identity off the running interface. Either `wg show`
/// call failing - most commonly because the interface does not exist yet -
/// is folded into [`WgOperatorError::ServerNotConfigured`]: whatever the
/// underlying reason, there is no live server identity to hand a fresh peer,
/// which is the honest framing for this specific read.
fn read_server_identity(iface: &str) -> Result<(String, u16), WgOperatorError> {
    let not_configured = |detail: String| WgOperatorError::ServerNotConfigured {
        iface: iface.to_string(),
        message: format!(
            "WireGuard is not configured on this box: interface '{iface}' is not up ({detail})"
        ),
    };

    let public_key = run("wg", &["show", iface, "public-key"]).map_err(|e| match e {
        WgOperatorError::Exec { message } => not_configured(message),
        other => other,
    })?;
    if public_key.is_empty() || public_key == "(none)" {
        return Err(not_configured("no public key reported".to_string()));
    }

    let listen_port_raw = run("wg", &["show", iface, "listen-port"]).map_err(|e| match e {
        WgOperatorError::Exec { message } => not_configured(message),
        other => other,
    })?;
    let listen_port: u16 = listen_port_raw.parse().map_err(|_| {
        not_configured(format!(
            "interface reported a non-numeric listen port ({listen_port_raw:?})"
        ))
    })?;

    Ok((public_key, listen_port))
}

/// Add `client_public_key` as a live peer on `iface`, routable only for its
/// own `/32` - side-effect-honest: a failure here is a real, returned
/// failure, never swallowed (unlike [`persist_best_effort`] below).
fn add_peer(iface: &str, client_public_key: &str, client_ip: &str) -> Result<(), WgOperatorError> {
    let allowed = format!("{client_ip}/32");
    run(
        "wg",
        &[
            "set",
            iface,
            "peer",
            client_public_key,
            "allowed-ips",
            &allowed,
        ],
    )?;
    Ok(())
}

/// Best-effort `wg-quick save <iface>` so the new peer survives a reboot -
/// NEVER fails the command: the peer is already live on the running
/// interface regardless of whether this succeeds. Swallows its own error
/// after a plain best-effort log line (this crate's established `genaryx:
/// ...` eprintln convention for a non-fatal background problem, e.g.
/// `bus::mod`'s fallback-to-mock log).
fn persist_best_effort(iface: &str) {
    if let Err(e) = run("wg-quick", &["save", iface]) {
        eprintln!(
            "genaryx: wg-quick save {iface} failed, the peer is still live on the running interface: {e:?}"
        );
    }
}

/// Render a QR PNG of `text` via `qrencode`, base64-encoded - ready for
/// `<img src="data:image/png;base64,...">`. Fed on stdin and read back off
/// stdout, the same no-temp-file shape [`pubkey_from_private`] uses.
fn qr_png_base64(text: &str) -> Result<String, WgOperatorError> {
    let mut child = Command::new("qrencode")
        .args(["-t", "PNG", "-o", "-", "-s", "6", "-m", "2"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                exec_err("qrencode: command not found (is it installed and on PATH?)".to_string())
            } else {
                exec_err(format!("could not run qrencode: {e}"))
            }
        })?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| exec_err("qrencode: could not open its stdin pipe".to_string()))?;
        stdin.write_all(text.as_bytes()).map_err(|e| {
            exec_err(format!(
                "qrencode: could not write the config to stdin: {e}"
            ))
        })?;
    }

    let out = child
        .wait_with_output()
        .map_err(|e| exec_err(format!("qrencode: could not read its output: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(exec_err(format!(
            "qrencode exited {}: {stderr}",
            out.status.code().unwrap_or(-1)
        )));
    }

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;
    Ok(B64.encode(out.stdout))
}

/// The client `.conf` TEXT this command hands back - both what the Download
/// button saves and what the QR code encodes.
fn render_conf(
    client_private_key: &str,
    client_ip: &str,
    server_public_key: &str,
    endpoint: &str,
) -> String {
    format!(
        "[Interface]\nPrivateKey = {client_private_key}\nAddress = {client_ip}/32\n\n[Peer]\nPublicKey = {server_public_key}\nEndpoint = {endpoint}\nAllowedIPs = {SERVER_TUNNEL_IP}/32\nPersistentKeepalive = 25\n"
    )
}

// ============================================================================
// the command's synchronous body + async entry point
// ============================================================================

/// The whole sequence, entirely synchronous (called only from inside
/// [`operator_wg_config`]'s `spawn_blocking`): read the live server
/// identity, mint a client keypair, allocate its tunnel address, add it as a
/// live peer, persist best-effort, then render the `.conf` and its QR.
/// Whether this console is running inside a container rather than on the box
/// whose WireGuard server it would mint a peer against.
///
/// This matters because the failure is otherwise unreadable. Everything below
/// shells `wg`/`wg-quick` against the HOST's kernel interface, and a container
/// has neither those binaries nor the host's network namespace, so the honest
/// answer ("this console cannot reach a WireGuard server from in here") would
/// otherwise reach the operator as `wg: command not found`, which reads like a
/// missing package on the box and sends them to install one that would change
/// nothing.
///
/// `KUBERNETES_SERVICE_HOST` is set by the kubelet in every pod, and
/// `/.dockerenv`/`/run/.containerenv` cover a plain container runtime. None of
/// the three can be true on a box running WireGuard directly, which is the
/// only place this feature works.
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

fn build_config_blocking() -> Result<RemoteWgOperatorConfigDto, WgOperatorError> {
    let iface = iface_name();

    // Refuse early and explain, rather than shelling a binary that is not
    // there. Reported as ServerNotConfigured because that is exactly what it
    // is from the operator's side: no WireGuard server this console can reach.
    if let Some(where_) = containerised() {
        return Err(WgOperatorError::ServerNotConfigured {
            iface: iface.clone(),
            message: format!(
                "this console is running in {where_}, so it cannot mint a WireGuard peer: \
                 the server it would add you to is the HOST's kernel interface, which a \
                 container has no access to. Reach the console over a tunnel you already \
                 have to that host (for a cluster, an ssh port-forward onto the node \
                 running this pod), and enrol a passkey once you are in. A cluster-native \
                 entry point is a separate component, not this command."
            ),
        });
    }

    let (server_public_key, listen_port) = read_server_identity(&iface)?;

    let client_ip = next_client_ip();
    let (client_private_key, client_public_key) = generate_client_keypair()?;

    // Authorize before rendering: if this fails, the operator must not walk
    // away with a config that was never going to connect (mirrors
    // `provisioning/new-device.sh`'s identical ordering and its own comment
    // on why).
    add_peer(&iface, &client_public_key, &client_ip)?;
    persist_best_effort(&iface);

    let endpoint = format!("{}:{listen_port}", endpoint_host());
    let conf = render_conf(&client_private_key, &client_ip, &server_public_key, &endpoint);
    let qr_png_base64 = qr_png_base64(&conf)?;

    Ok(RemoteWgOperatorConfigDto {
        conf,
        qr_png_base64,
        client_ip,
        endpoint,
        server_public_key,
        console_tunnel_url: format!("http://{SERVER_TUNNEL_IP}:{CONSOLE_PORT}"),
    })
}

/// Mint the signed-in operator a fresh WireGuard peer against this box's own
/// kernel WireGuard server - see this module's doc comment. Runs the actual
/// shelling inside [`tokio::task::spawn_blocking`], the same bridge every
/// other blocking connector call in this crate uses.
pub async fn operator_wg_config() -> Result<RemoteWgOperatorConfigDto, WgOperatorError> {
    tokio::task::spawn_blocking(build_config_blocking)
        .await
        .unwrap_or_else(|join_err| {
            Err(exec_err(format!(
                "operator wg config task failed to run: {join_err}"
            )))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pod_is_told_why_rather_than_that_wg_is_missing() {
        // The whole point of the preflight: an operator in a cluster must not
        // be sent to install a package that would change nothing. This asserts
        // the reason travels, not just that it fails.
        //
        // SAFETY: single-threaded test, and the variable is removed again
        // before it can leak into a sibling test.
        unsafe { std::env::set_var("KUBERNETES_SERVICE_HOST", "10.43.0.1") };
        let out = build_config_blocking();
        unsafe { std::env::remove_var("KUBERNETES_SERVICE_HOST") };

        match out {
            Err(WgOperatorError::ServerNotConfigured { message, .. }) => {
                assert!(message.contains("Kubernetes pod"), "names where it is: {message}");
                assert!(message.contains("tunnel"), "says what to do instead: {message}");
                assert!(!message.contains("command not found"), "not the raw shell error");
            }
            other => panic!("expected an explained ServerNotConfigured, got {other:?}"),
        }
    }

    #[test]
    fn render_conf_matches_the_documented_shape() {
        let conf = render_conf("PRIVKEY==", "10.9.0.2", "SERVERPUB==", "46.225.171.155:51820");
        assert_eq!(
            conf,
            "[Interface]\nPrivateKey = PRIVKEY==\nAddress = 10.9.0.2/32\n\n[Peer]\nPublicKey = SERVERPUB==\nEndpoint = 46.225.171.155:51820\nAllowedIPs = 10.9.0.1/32\nPersistentKeepalive = 25\n"
        );
    }

    #[test]
    fn iface_name_defaults_to_wg_op_when_unset() {
        // SAFETY (edition-2024 env contract, same as `copilot::state`'s own
        // `config_from_env_reads_the_provider_surface` test): no other test
        // in this binary reads or writes GENARYX_WG_IFACE, so this mutation
        // cannot race a concurrent getenv of the same variable.
        unsafe {
            std::env::remove_var("GENARYX_WG_IFACE");
        }
        assert_eq!(iface_name(), "wg-op");
    }

    #[test]
    fn endpoint_host_defaults_to_the_real_box_ip_when_unset() {
        // SAFETY: see `iface_name_defaults_to_wg_op_when_unset` above - this
        // test owns GENARYX_WG_ENDPOINT_HOST exclusively in this binary.
        unsafe {
            std::env::remove_var("GENARYX_WG_ENDPOINT_HOST");
        }
        assert_eq!(endpoint_host(), "46.225.171.155");
    }

    #[test]
    fn next_client_ip_is_fixed_at_10_9_0_2_for_v1() {
        assert_eq!(next_client_ip(), "10.9.0.2");
    }

    #[test]
    fn run_against_a_missing_binary_is_a_clean_exec_error_not_a_panic() {
        match run("definitely-not-a-real-wg-cli-xyz", &[]) {
            Err(WgOperatorError::Exec { message }) => {
                assert!(message.contains("not found"));
            }
            other => panic!("expected Exec, got {other:?}"),
        }
    }

    #[test]
    fn read_server_identity_against_a_missing_or_down_interface_is_server_not_configured() {
        // Whether `wg` itself is missing from whatever box runs this test, or
        // `wg` is present but this interface name certainly does not exist,
        // `run("wg", ...)` fails either way, and `read_server_identity` folds
        // ANY such failure into `ServerNotConfigured` - the honest framing
        // for its one call site ("read the server identity, or admit
        // WireGuard is not configured here"), never a generic `Exec` for
        // this specific read.
        match read_server_identity("genaryx-test-nonexistent-iface-xyz") {
            Err(WgOperatorError::ServerNotConfigured { iface, message }) => {
                assert_eq!(iface, "genaryx-test-nonexistent-iface-xyz");
                assert!(!message.is_empty());
            }
            other => panic!("expected ServerNotConfigured, got {other:?}"),
        }
    }
}
