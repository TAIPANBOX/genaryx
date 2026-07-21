//! Where the Pocket panel finds `genaryx-relay`'s admin API (docs/PHASE5.md
//! W2).
//!
//! Deliberately NOT a `taipan up` descriptor lookup like `money::env`'s
//! Cloud discovery: no `taipan` descriptor schema key for the relay exists
//! yet (confirmed 2026-07-18 by reading `~/Development/taipan/src/descriptor.rs`
//! directly - packaging the relay into `taipan`'s deploy flow is
//! docs/PHASE5.md's own explicit "Deferred to R1+" item, "`taipan`-CLI
//! packaging + systemd unit"). Until that lands, the admin URL is an env
//! var override with a loopback default matching the relay's own
//! `admin_bind_addr` default (`crates/relay/src/config.rs`), so a same-host
//! sim/dev setup (docs/PHASE5.md's "Cloud is local... The relay colocates
//! with it over loopback") needs no configuration at all.

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
        // Pins the literal default constant against
        // `crates/relay/src/config.rs`'s own `admin_bind_addr` default
        // ("127.0.0.1:8444") so a drift between the two is caught here
        // rather than discovered at runtime as a mysteriously unreachable
        // Pocket panel.
        assert_eq!(DEFAULT_RELAY_ADMIN_URL, "http://127.0.0.1:8444");
    }

    #[test]
    fn env_var_name_is_the_documented_genaryx_relay_admin_url() {
        assert_eq!(RELAY_ADMIN_URL_ENV_VAR, "GENARYX_RELAY_ADMIN_URL");
    }
}
