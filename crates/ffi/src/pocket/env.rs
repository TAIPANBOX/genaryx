//! Where the Pocket panel finds `genaryx-relay`'s admin API (docs/PHASE5.md
//! W2). A line-for-line mirror of the Tauri shell's
//! `apps/desktop/src-tauri/src/pocket/env.rs` (the same shell-parity
//! convention `cloud/env.rs`'s own module doc follows against
//! `money/env.rs`: no Tauri-specific dependencies to reuse, so this tiny
//! resolution rule is reproduced here rather than shared through a crate
//! neither shell already depends on for it).
//!
//! Deliberately NOT a `taipan up` descriptor lookup like `cloud::env`'s
//! Cloud discovery: no `taipan` descriptor schema key for the relay exists
//! yet (confirmed 2026-07-18 by reading `~/Development/taipan/src/descriptor.rs`
//! directly - packaging the relay into `taipan`'s deploy flow is
//! docs/PHASE5.md's own explicit "Deferred to R1+" item, "`taipan`-CLI
//! packaging + systemd unit"). Until that lands, the admin URL is an env var
//! override with a loopback default matching the relay's own
//! `admin_bind_addr` default (`crates/relay/src/config.rs`).

const RELAY_ADMIN_URL_ENV_VAR: &str = "GENARYX_RELAY_ADMIN_URL";
const DEFAULT_RELAY_ADMIN_URL: &str = "http://127.0.0.1:8444";

/// Resolve the relay admin API base URL. Never fails: a missing or
/// blank/whitespace-only env var simply falls back to the loopback default.
pub fn relay_admin_url() -> String {
    std::env::var(RELAY_ADMIN_URL_ENV_VAR)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_RELAY_ADMIN_URL.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_the_relays_own_loopback_admin_default() {
        assert_eq!(DEFAULT_RELAY_ADMIN_URL, "http://127.0.0.1:8444");
    }

    #[test]
    fn env_var_name_is_the_documented_genaryx_relay_admin_url() {
        assert_eq!(RELAY_ADMIN_URL_ENV_VAR, "GENARYX_RELAY_ADMIN_URL");
    }
}
