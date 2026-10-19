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
    /// Make the per-action passkey ceremony MANDATORY, which is the default: a
    /// sensitive command from a caller with no enrolled passkey is refused
    /// with the way to enrol one, instead of running on the session alone and
    /// being journaled software-signed.
    ///
    /// The software-signed fallback is still there, but only as an explicit
    /// choice: `GENARYX_WEB_ALLOW_SOFTWARE_CEREMONY=1` turns it on, the
    /// console announces it at startup, and every use of it is logged as a
    /// warning. See [`require_passkey_from_switch`] for the reading.
    pub require_passkey: bool,
    /// Where `delegation_revoke` sends a revocation, resolved ONCE at
    /// startup (`GENARYX_VOUCHRYX_URL` + `GENARYX_VOUCHRYX_REVOKE_KEY_FILE`).
    /// Unlike every other field here, an inconsistent pair of variables never
    /// reaches this struct at all: `main.rs`'s `Cmd::Serve` refuses to start
    /// first. See `genaryx_api::delegation::env`'s module doc.
    pub delegation: genaryx_api::delegation::env::RevokeConfig,
    /// Whether `money_close_retained` can mint a closure token (D8), resolved
    /// ONCE at startup from `GENARYX_CLOSURE_SECRET_FILE`. A secret file that
    /// cannot be read, or holds fewer than 32 bytes, never reaches this
    /// struct: `main.rs`'s `Cmd::Serve` refuses to start first. See
    /// `genaryx_api::money::closure`'s module doc.
    pub closure: genaryx_api::money::closure::ClosureConfig,
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

    /// Path of the enrolled-passkeys store (docs/CONSOLE-IDP.md, B3/2). Same
    /// posture as `operator.json` beside it: authenticator public keys only,
    /// never people - the customer's IdP stays the identity registry.
    pub fn passkeys_file(&self) -> PathBuf {
        self.state_dir.join("passkeys.json")
    }

    /// Read [`Config::require_passkey`] from `GENARYX_WEB_ALLOW_SOFTWARE_CEREMONY`.
    ///
    /// An environment variable rather than a flag, matching the other two
    /// pieces of WebAuthn configuration this console already reads that way
    /// (`GENARYX_WEB_RP_ID` / `GENARYX_WEB_ORIGIN`) and the OIDC block beside
    /// them: they belong to the deployment, not to one invocation. Absent is
    /// the default, which requires the passkey. The console says at startup
    /// which way it read.
    pub fn require_passkey_from_env() -> bool {
        require_passkey_from_switch(&std::env::var(ALLOW_SOFTWARE_CEREMONY_ENV).unwrap_or_default())
    }

    /// Say which way [`Config::require_passkey`] was read, both ways.
    ///
    /// Stated at startup rather than discovered. The default refuses a
    /// sensitive command from a caller with nothing enrolled. The switch
    /// allows the software-signed fallback instead, and that is a downgrade of
    /// the ceremony, so it is announced at warning level.
    pub fn announce_passkey_policy(&self) {
        if self.require_passkey {
            tracing::info!(
                "the passkey is required for sensitive commands (the default): a caller with \
                 no enrolled passkey is refused until one is enrolled"
            );
        } else {
            tracing::warn!(
                "GENARYX_WEB_ALLOW_SOFTWARE_CEREMONY is set: a sensitive command from a caller \
                 with no enrolled passkey runs on the session alone and is journaled \
                 software-signed. This is an explicit downgrade of the per-action ceremony."
            );
        }
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

/// Whether the session cookie is marked Secure (F-017). It is, when the
/// operator asks for it, and whenever the console's own origin is https: that
/// is a person reaching the console over TLS, directly or through the proxy
/// the origin names. A plain http origin, the tunnel case, keeps its cookie as
/// it was.
pub fn cookie_is_secure(explicit: bool, origin: &str) -> bool {
    explicit || origin.starts_with("https://")
}

/// The environment variable that allows the software-signed fallback. It is
/// off unless an operator sets it to a true word.
pub const ALLOW_SOFTWARE_CEREMONY_ENV: &str = "GENARYX_WEB_ALLOW_SOFTWARE_CEREMONY";

/// Whether the passkey is required, given the raw value of the fallback switch.
///
/// The passkey is required unless the switch names the fallback. A value
/// nobody meant, a typo included, leaves the ceremony enforced: that is the
/// safe direction, and the opposite of what an unmeant value did before.
pub fn require_passkey_from_switch(raw: &str) -> bool {
    !allow_software_ceremony(raw)
}

/// Whether the raw switch value names the software-signed fallback.
fn allow_software_ceremony(raw: &str) -> bool {
    parse_switch_on(raw)
}

/// The words that turn a switch on, split out of
/// [`require_passkey_from_switch`] so the decision has a seam a test can reach.
/// Reading the variable is I/O and setting one is process-global; which words
/// count is the part worth pinning.
fn parse_switch_on(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    fn config_bound_to(ip: IpAddr) -> Config {
        Config {
            bind: SocketAddr::new(ip, 7420),
            state_dir: PathBuf::from("/tmp/genaryx-web"),
            ui_dir: None,
            secure_cookies: false,
            require_passkey: false,
            delegation: genaryx_api::delegation::env::RevokeConfig::NotConfigured,
            closure: genaryx_api::money::closure::ClosureConfig::NotConfigured,
        }
    }

    #[test]
    fn the_cookie_is_secure_when_the_origin_is_https_or_asked_for() {
        assert!(cookie_is_secure(false, "https://console.example"));
        assert!(cookie_is_secure(true, "http://10.0.0.2:7420"));
        assert!(!cookie_is_secure(false, "http://localhost:7420"));
    }

    #[test]
    fn the_passkey_is_required_when_the_fallback_switch_is_unset_or_blank() {
        // The default. An operator who sets nothing gets the ceremony
        // enforced, and the software-signed fallback is an explicit choice.
        assert!(require_passkey_from_switch(""));
        assert!(require_passkey_from_switch("   "));
    }

    #[test]
    fn the_fallback_switch_drops_the_requirement_only_for_the_words_an_operator_would_type() {
        for on in ["1", "true", "yes", "on", "TRUE", " On "] {
            assert!(
                !require_passkey_from_switch(on),
                "{on:?} must allow the software-signed fallback, which is announced loudly"
            );
        }
    }

    #[test]
    fn a_typo_or_a_value_nobody_meant_keeps_the_passkey_required() {
        // The direction is safe: a typo can only leave the ceremony enforced.
        for off in [
            "0", "false", "no", "off", "y", "enable", "enabled", "ON!", "truthy", "2",
        ] {
            assert!(
                require_passkey_from_switch(off),
                "{off:?} must not drop the passkey requirement"
            );
        }
    }

    #[test]
    fn the_fallback_switch_has_its_own_name_in_the_environment() {
        assert_eq!(
            ALLOW_SOFTWARE_CEREMONY_ENV,
            "GENARYX_WEB_ALLOW_SOFTWARE_CEREMONY"
        );
    }

    #[test]
    fn both_state_files_sit_inside_the_state_directory_and_are_not_the_same_file() {
        // They hold the Argon2id credential record and the enrolled
        // authenticator keys. A path escaping the state directory would put
        // them somewhere an operator does not know to protect or back up.
        let c = config_bound_to(IpAddr::V4(Ipv4Addr::LOCALHOST));
        for p in [c.operator_file(), c.passkeys_file()] {
            assert!(
                p.starts_with(&c.state_dir),
                "{} must live under {}",
                p.display(),
                c.state_dir.display()
            );
        }
        assert_ne!(
            c.operator_file(),
            c.passkeys_file(),
            "the credential record and the passkey store must be different files"
        );
    }

    #[test]
    fn the_default_state_directory_sits_beside_the_descriptors_the_planes_read() {
        // Beside ~/.taipan so an operator backing up one backs up both, which
        // is the whole reason it is not somewhere of its own.
        let d = Config::default_state_dir();
        let s = d.to_string_lossy();
        assert!(
            s.contains(".taipan"),
            "the default state dir must sit beside the plane descriptors, got {s}"
        );
        assert!(
            s.ends_with("genaryx-web"),
            "and must be this console's own directory within it, got {s}"
        );
    }

    #[test]
    fn a_wildcard_bind_is_recognised_as_reaching_beyond_this_machine() {
        // warn_if_exposed only logs, so what a test can hold is the
        // classification it branches on: the difference between "reachable
        // over the operator's tunnel" and "reachable from the internet".
        for wildcard in [
            IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            IpAddr::V6(Ipv6Addr::UNSPECIFIED),
        ] {
            let c = config_bound_to(wildcard);
            assert!(
                c.bind.ip().is_unspecified(),
                "{wildcard} is a wildcard bind"
            );
            assert!(
                !c.bind.ip().is_loopback(),
                "{wildcard} must not take the quiet loopback branch"
            );
            c.warn_if_exposed();
        }

        for loopback in [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
        ] {
            let c = config_bound_to(loopback);
            assert!(
                c.bind.ip().is_loopback(),
                "{loopback} must read as loopback"
            );
            c.warn_if_exposed();
        }

        // The expected deployment: the tunnel's own address, which is neither.
        let tunnel = config_bound_to(IpAddr::V4(Ipv4Addr::new(10, 9, 0, 1)));
        assert!(!tunnel.bind.ip().is_loopback());
        assert!(!tunnel.bind.ip().is_unspecified());
        tunnel.warn_if_exposed();
    }

    #[test]
    fn announcing_the_passkey_policy_works_either_way_round() {
        // It is what tells whoever is on call which box they are on. Both
        // branches are legitimate and neither should be a surprise.
        let mut c = config_bound_to(IpAddr::V4(Ipv4Addr::LOCALHOST));
        c.announce_passkey_policy();
        c.require_passkey = true;
        c.announce_passkey_policy();
    }
}
