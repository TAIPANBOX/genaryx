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
    /// Make the per-action passkey ceremony MANDATORY: a sensitive command
    /// from a caller with no enrolled passkey is refused, instead of running
    /// on the session alone and being journaled software-signed.
    ///
    /// Off by default, so an upgrade changes nothing about a running box: the
    /// software-signed fallback is documented, journaled honestly, and the
    /// bridge an operator who has not enrolled yet is standing on. What was
    /// missing was any way to STOP standing on it. An operator who wants the
    /// invariant enforced ("a sensitive command requires a per-action
    /// ceremony", CLAUDE.md 2) sets `GENARYX_WEB_REQUIRE_PASSKEY=1` and the
    /// fallback is gone for every command in `SENSITIVE_COMMANDS`.
    pub require_passkey: bool,
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

    /// Read [`Config::require_passkey`] from `GENARYX_WEB_REQUIRE_PASSKEY`.
    ///
    /// An environment variable rather than a flag, matching the other two
    /// pieces of WebAuthn configuration this console already reads that way
    /// (`GENARYX_WEB_RP_ID` / `GENARYX_WEB_ORIGIN`) and the OIDC block beside
    /// them: they belong to the deployment, not to one invocation. Absent, or
    /// any value other than the accepted true words, is off - a typo must not
    /// silently lock an operator out of their own kill switch, and the console
    /// says at startup which way it read.
    pub fn require_passkey_from_env() -> bool {
        parse_require_passkey(&std::env::var("GENARYX_WEB_REQUIRE_PASSKEY").unwrap_or_default())
    }

    /// Say which way [`Config::require_passkey`] was read, both ways.
    ///
    /// Stated at startup rather than discovered: on a strict box a sensitive
    /// command from a caller with nothing enrolled is refused, and on a
    /// permissive one it runs software-signed. Either is a legitimate
    /// deployment, and neither should be a surprise to whoever is on call.
    pub fn announce_passkey_policy(&self) {
        if self.require_passkey {
            tracing::info!(
                "GENARYX_WEB_REQUIRE_PASSKEY is on: a sensitive command from a caller with no \
                 enrolled passkey is REFUSED, not run software-signed"
            );
        } else {
            tracing::info!(
                "GENARYX_WEB_REQUIRE_PASSKEY is off (the default): a caller with no enrolled \
                 passkey still runs sensitive commands, journaled software-signed. Set it to 1 \
                 to require the ceremony."
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

/// The words that turn the strict mode on, split out of
/// [`Config::require_passkey_from_env`] so the decision has a seam a test can
/// reach. Reading the variable is I/O and setting one is process-global; which
/// words count is the part worth pinning, and it is the part that decides
/// whether an operator can be locked out of their own kill switch by a typo.
fn parse_require_passkey(raw: &str) -> bool {
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
        }
    }

    #[test]
    fn the_strict_passkey_mode_turns_on_for_the_words_an_operator_would_type() {
        for on in ["1", "true", "yes", "on", "TRUE", "Yes", " ON ", "\ttrue\n"] {
            assert!(
                parse_require_passkey(on),
                "{on:?} should turn the strict mode on; an operator who typed it and \
                 got the permissive mode has an invariant they believe is enforced"
            );
        }
    }

    #[test]
    fn anything_else_leaves_the_strict_mode_off_including_a_typo() {
        // The direction is deliberate. Strict mode REFUSES a sensitive command
        // from a caller with no enrolled passkey, so a typo that turned it ON
        // would lock an operator out of their own kill switch with no
        // explanation. Off is the safe reading of a value nobody meant.
        for off in [
            "", " ", "0", "false", "no", "off", "y", "enable", "enabled", "ON!", "truthy", "2",
        ] {
            assert!(
                !parse_require_passkey(off),
                "{off:?} must leave the strict mode off: a value nobody meant must not \
                 refuse the commands an operator needs in an incident"
            );
        }
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
