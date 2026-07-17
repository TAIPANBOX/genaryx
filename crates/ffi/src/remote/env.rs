//! Environment defaults for [`super::RemoteHandle`] (docs/PHASE4.md W4,
//! decision D11). UNLIKE every sibling `env` module in this crate, nothing
//! here gates panel readiness - there is no single "Remote environment" to
//! discover: Hetzner (a pasted API token), the WireGuard tunnel (a box
//! admin's peer config), and SSH (a campaign's host/identity/pinned key) are
//! each independently operator-configured per docs/PHASE4.md W4's own field
//! list, not resolved from a `taipan up` descriptor the way Cloud/Wardryx/
//! Idryx are. So every function below is a pre-filled, always-overridable
//! SUGGESTION for exactly one of the Remote panel's form fields - "operator
//! can see/set it, never enforced", the same idiom
//! [`crate::crypto::env::default_scan_target`] and
//! [`crate::drills::env::default_scenario_dir`] already establish for their
//! own panels. None of these functions can fail; an `Option`/empty result
//! here always means "no honest suggestion exists yet", never "an error
//! occurred" - [`super::RemoteHandle`] never gates on any of them.

use std::path::{Path, PathBuf};

/// Genaryx-side convenience override for the `wireguard-go` binary path (not
/// a wireguard-go-documented var; the upstream tool has no such override) -
/// mirrors [`crate::drills::env`]'s own `MOCKRYX_BIN` convenience tier for a
/// bundled third-party binary with no `taipan up`-managed install yet.
const WIREGUARD_GO_BIN_ENV_VAR: &str = "WIREGUARD_GO_BIN";
const WIREGUARD_GO_BIN_NAME: &str = "wireguard-go";

/// The real `hcloud` CLI's OWN documented token env var - reused here purely
/// as a discovery convenience (mirrors [`crate::memory::env`]'s own reuse of
/// engram-mcp's `ENGRAM_MCP_DB`): an operator who already manages a
/// campaign's box with the official Hetzner CLI gets the console's Hetzner
/// inventory pre-filled for free. The resolved value only ever flows into an
/// explicit [`super::RemoteHandle::list_hetzner`] `token` argument, never
/// inherited/smuggled into a subprocess environment (there is no subprocess
/// here at all - Hetzner is a plain HTTPS read).
const HCLOUD_TOKEN_ENV_VAR: &str = "HCLOUD_TOKEN";

/// A live-validation campaign's Hetzner boxes are tagged this way
/// (`crates/connectors/src/hetzner.rs`'s own module doc: "the taipan-tagged
/// servers a live-validation campaign is running on"; that connector's own
/// tests use this exact label) - a reasonable pre-filled starting point, not
/// a requirement: an operator scanning a token with no taipan-labeled boxes
/// at all can always clear the field to see everything the token can read.
const DEFAULT_LABEL_SELECTOR: &str = "managed-by=taipan";

/// A private-use `/32` point-to-point pair (RFC 1918), matching the exact
/// addresses `crates/connectors/src/wg.rs`'s own tests/docs use as their
/// worked example - a plausible, non-colliding starting point for a THIS
/// side/peer-side tunnel address pair, always overridable.
const DEFAULT_TUNNEL_LOCAL_IP: &str = "10.9.0.2";
const DEFAULT_TUNNEL_PEER_IP: &str = "10.9.0.1";

/// WireGuard's own conventional NAT-traversal keepalive
/// (`crates/connectors/src/wg.rs`'s own doc on `WgPeer::persistent_keepalive`:
/// "typically 25").
const DEFAULT_PERSISTENT_KEEPALIVE: u16 = 25;

const DEFAULT_SSH_PORT: u16 = 22;

/// [`WIREGUARD_GO_BIN_ENV_VAR`], then the well-known `~/.taipan/bin/
/// wireguard-go` (in case a future `taipan up --with wireguard-go` install
/// step lands, this starts working with zero changes here - mirrors
/// [`crate::memory::env::discover_bin`]'s own forward-looking rationale),
/// then a `$PATH` scan. `None` when nothing anywhere resolves - a normal,
/// honest "no suggestion" the operator fills in by hand; this NEVER gates the
/// rest of the panel (see the module doc) - Hetzner inventory and SSH ops
/// stay fully usable even with zero WireGuard tooling installed.
#[must_use]
pub fn default_wireguard_go_bin() -> Option<PathBuf> {
    env_bin().or_else(taipan_bin).or_else(path_bin)
}

/// The env-var tier: trusted verbatim once non-blank, existence unchecked -
/// mirrors [`crate::memory::env`]'s own `env_db_path_from` rationale ("an
/// explicit override must be reported as-is"), unlike the two auto-discovery
/// tiers below which only ever report a real file.
fn env_bin() -> Option<PathBuf> {
    env_bin_from(std::env::var(WIREGUARD_GO_BIN_ENV_VAR).ok())
}

fn env_bin_from(value: Option<String>) -> Option<PathBuf> {
    let value = value?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

fn taipan_bin() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    taipan_bin_under(&home)
}

/// Testable core of [`taipan_bin`]: `home/.taipan/bin/wireguard-go`, `None`
/// when nothing file-shaped exists there - mirrors
/// [`crate::idryx::env::idryx_binary_under`]'s own shape.
fn taipan_bin_under(home: &Path) -> Option<PathBuf> {
    let path = home.join(".taipan").join("bin").join(WIREGUARD_GO_BIN_NAME);
    path.is_file().then_some(path)
}

fn path_bin() -> Option<PathBuf> {
    path_bin_from(std::env::var_os("PATH")?)
}

/// Testable core of [`path_bin`], taking the (already-read) `$PATH` value
/// directly so tests never have to mutate real process environment - mirrors
/// [`crate::memory::env::path_bin_from`]'s own shape.
fn path_bin_from(path_var: std::ffi::OsString) -> Option<PathBuf> {
    std::env::split_paths(&path_var).find_map(|dir| {
        let candidate = dir.join(WIREGUARD_GO_BIN_NAME);
        candidate.is_file().then_some(candidate)
    })
}

/// A platform-appropriate default interface NAME to hand `wireguard-go`.
/// macOS ignores the number and always allocates the next free `utunN`
/// (`crates/connectors/src/wg.rs`'s own doc: "interface is `utun` on macOS
/// (kernel picks the number)"), so `"utun"` is the literal correct value to
/// pre-fill there, not a placeholder. Linux takes the name literally, so this
/// pre-fills a genaryx-specific name unlikely to collide with another tunnel
/// already up on the box.
#[must_use]
pub fn default_interface() -> &'static str {
    if cfg!(target_os = "macos") {
        "utun"
    } else {
        "genaryx0"
    }
}

/// See [`DEFAULT_LABEL_SELECTOR`]'s own doc.
#[must_use]
pub fn default_hetzner_label_selector() -> &'static str {
    DEFAULT_LABEL_SELECTOR
}

/// [`HCLOUD_TOKEN_ENV_VAR`], or `None` - see that const's own doc. A
/// deliberately different honesty posture from every OTHER secret this crate
/// pre-fills (there is normally none: e.g. the Evidence Center's own Qryx
/// sign-key field is never pre-filled, "no honest default for a private
/// signing key path") because a read-scoped Hetzner API token is not a
/// signing key or a long-lived credential this console holds any trust
/// boundary around - it is exactly the same class of value
/// [`crate::cloud::env`]'s own `TOKENFUSE_CLOUD_ADMIN_KEY` env-var tier
/// already pre-fills for the Money panel's admin bearer.
#[must_use]
pub fn default_hetzner_token() -> Option<String> {
    hetzner_token_from(std::env::var(HCLOUD_TOKEN_ENV_VAR).ok())
}

fn hetzner_token_from(value: Option<String>) -> Option<String> {
    let value = value?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// See [`DEFAULT_TUNNEL_LOCAL_IP`]'s own doc.
#[must_use]
pub fn default_tunnel_local_ip() -> &'static str {
    DEFAULT_TUNNEL_LOCAL_IP
}

/// See [`DEFAULT_TUNNEL_PEER_IP`]'s own doc.
#[must_use]
pub fn default_tunnel_peer_ip() -> &'static str {
    DEFAULT_TUNNEL_PEER_IP
}

/// See [`DEFAULT_PERSISTENT_KEEPALIVE`]'s own doc. Always `Some` - unlike the
/// binary/token tiers above, this is a protocol-level convention, not a
/// presence/absence signal.
#[must_use]
pub fn default_persistent_keepalive() -> Option<u16> {
    Some(DEFAULT_PERSISTENT_KEEPALIVE)
}

/// See [`DEFAULT_SSH_PORT`]'s own doc.
#[must_use]
pub fn default_ssh_port() -> u16 {
    DEFAULT_SSH_PORT
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "genaryx-ffi-remote-env-test-{tag}-{}-{n}",
            std::process::id()
        ))
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(path, body).expect("write fixture file");
    }

    // ---- wireguard-go binary location --------------------------------------

    #[test]
    fn env_bin_from_requires_a_non_blank_value_and_does_not_check_existence() {
        assert!(env_bin_from(None).is_none());
        assert!(env_bin_from(Some(String::new())).is_none());
        assert!(env_bin_from(Some("   ".to_string())).is_none());

        // An explicit override is trusted verbatim, even for a path that does
        // not exist yet - see the function's own doc.
        let resolved = env_bin_from(Some("/definitely/not/real/wireguard-go".to_string()))
            .expect("a non-blank override resolves regardless of existence");
        assert_eq!(resolved, PathBuf::from("/definitely/not/real/wireguard-go"));
    }

    #[test]
    fn taipan_bin_under_missing_home_yields_none() {
        let home = unique_dir("no-bin-home");
        assert!(taipan_bin_under(&home).is_none());
    }

    #[test]
    fn taipan_bin_under_finds_a_real_file() {
        let home = unique_dir("has-bin-home");
        let bin = home.join(".taipan").join("bin").join("wireguard-go");
        write(&bin, "#!/bin/sh\nexit 0\n");

        let found = taipan_bin_under(&home).expect("must find the fixture binary");
        assert_eq!(found, bin);

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn path_bin_from_finds_the_binary_in_any_listed_directory() {
        let empty_dir = unique_dir("path-empty");
        let hit_dir = unique_dir("path-hit");
        std::fs::create_dir_all(&empty_dir).expect("create empty dir");
        write(&hit_dir.join("wireguard-go"), "#!/bin/sh\nexit 0\n");

        let path_var = std::env::join_paths([&empty_dir, &hit_dir]).expect("join paths");
        let found = path_bin_from(path_var).expect("must find wireguard-go on the synthetic PATH");
        assert_eq!(found, hit_dir.join("wireguard-go"));

        let _ = std::fs::remove_dir_all(&empty_dir);
        let _ = std::fs::remove_dir_all(&hit_dir);
    }

    #[test]
    fn path_bin_from_with_no_match_anywhere_is_none() {
        let dir = unique_dir("path-miss");
        std::fs::create_dir_all(&dir).expect("create dir");
        let path_var = std::env::join_paths([&dir]).expect("join paths");
        assert!(path_bin_from(path_var).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_wireguard_go_bin_never_panics() {
        // Whatever this box's real HOME/PATH happen to contain, this must
        // never panic - either a real path or a clean None.
        let _ = default_wireguard_go_bin();
    }

    // ---- hetzner token ------------------------------------------------------

    #[test]
    fn hetzner_token_from_requires_a_non_blank_value() {
        assert!(hetzner_token_from(None).is_none());
        assert!(hetzner_token_from(Some(String::new())).is_none());
        assert!(hetzner_token_from(Some("   ".to_string())).is_none());
        assert_eq!(
            hetzner_token_from(Some("  tok_abc123  ".to_string())),
            Some("tok_abc123".to_string())
        );
    }

    // ---- everything else: never panics, always a sensible value -----------

    #[test]
    fn default_interface_is_never_blank_and_is_platform_appropriate() {
        let iface = default_interface();
        assert!(!iface.is_empty());
        if cfg!(target_os = "macos") {
            assert_eq!(iface, "utun");
        }
    }

    #[test]
    fn default_hetzner_label_selector_is_never_blank() {
        assert!(!default_hetzner_label_selector().is_empty());
    }

    #[test]
    fn default_tunnel_addresses_are_never_blank_and_differ() {
        assert!(!default_tunnel_local_ip().is_empty());
        assert!(!default_tunnel_peer_ip().is_empty());
        assert_ne!(default_tunnel_local_ip(), default_tunnel_peer_ip());
    }

    #[test]
    fn default_persistent_keepalive_is_a_sane_positive_value() {
        let keepalive = default_persistent_keepalive().expect("always Some");
        assert!(keepalive > 0);
    }

    #[test]
    fn default_ssh_port_is_22() {
        assert_eq!(default_ssh_port(), 22);
    }
}
