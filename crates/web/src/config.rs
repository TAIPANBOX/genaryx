//! Where the server listens, and where it keeps the little state that is its
//! own (the operator account and, with it, nothing else).
//!
//! The default bind is `127.0.0.1:7420`, and that default is a security
//! decision rather than a convenience one. Genaryx's whole remote story (D11)
//! is that the control plane is not on the internet: the operator reaches it
//! through their own WireGuard tunnel, so the address worth binding is the
//! tunnel's (`10.9.0.1`, say), never `0.0.0.0`. Binding to a wildcard address
//! is therefore allowed but never silent: [`Config::warn_if_exposed`] says so
//! out loud at startup, because the difference between "reachable over the
//! tunnel" and "reachable from the internet" is the difference between this
//! product working and this product being the incident.

use std::net::SocketAddr;
use std::path::PathBuf;

/// Resolved server configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Address to listen on.
    pub bind: SocketAddr,
    /// Directory holding `operator.json` (the Argon2id credential record).
    /// Never holds plane data: everything the UI shows is read live from the
    /// customer's own stack, so there is no copy of it here to leak.
    pub state_dir: PathBuf,
    /// Directory of the built web UI to serve. Absent means API only, which
    /// is what a `vite dev` front end wants (it serves the UI itself and
    /// proxies `/api` here).
    pub ui_dir: Option<PathBuf>,
    /// Mark the session cookie `Secure`, so the browser only ever sends it
    /// back over TLS.
    ///
    /// Off by default, and that is not laziness: the expected deployment is
    /// plain HTTP inside an already-encrypted WireGuard tunnel, and a
    /// `Secure` cookie there is simply never sent, which locks the operator
    /// out with no explanation. Turn it on the moment anything terminates
    /// TLS in front of this.
    pub secure_cookies: bool,
}

impl Config {
    /// The default state directory, beside the descriptors the planes already
    /// read (`~/.taipan/`), so an operator backing up one backs up both.
    pub fn default_state_dir() -> PathBuf {
        home().join(".taipan").join("genaryx-web")
    }

    /// Path of the single credential record.
    pub fn operator_file(&self) -> PathBuf {
        self.state_dir.join("operator.json")
    }

    /// Say plainly when the bind address reaches beyond this machine.
    ///
    /// Not a refusal: an operator who terminates TLS in front of this, or who
    /// runs it on a host with no public route, has a legitimate reason to
    /// bind wide. It is a statement, so that nobody discovers the exposure
    /// from someone else.
    pub fn warn_if_exposed(&self) {
        let ip = self.bind.ip();
        if ip.is_loopback() {
            tracing::info!(bind = %self.bind, "listening on loopback only");
        } else if ip.is_unspecified() {
            tracing::warn!(
                bind = %self.bind,
                "listening on ALL interfaces. Genaryx expects to be reached over the \
                 operator's own tunnel (D11), not from the open internet: bind the \
                 tunnel address instead unless something in front of this is \
                 terminating TLS and authenticating."
            );
        } else {
            tracing::info!(
                bind = %self.bind,
                "listening on a specific non-loopback address (expected: the tunnel's)"
            );
        }
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
