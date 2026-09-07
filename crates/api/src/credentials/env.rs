//! Credentials-plane environment discovery: which gateway to talk to.
//!
//! Mirrors `crate::identity::env`'s shape (one `Taipan`-only [`EnvSource`],
//! `?`-chained `Option`s throughout, newest-descriptor-wins), resolving the
//! SAME `services.gateway.url` `crate::drills::env` reads off a `taipan up`
//! descriptor - explicitly NOT `services.cloud` (TokenFuse Cloud's own admin
//! API, `crate::money::env`'s target). Like idryx, the gateway's `/v1/keys`
//! read needs no auth at all on a loopback bind (see
//! `genaryx_connectors::gateway`'s module doc), so unlike `drills::env`
//! there is no bearer resolved FROM the descriptor, and unlike
//! `identity::env` there is no extra `events` section to carry either.
//!
//! A descriptor with no `services.gateway` entry (or no descriptor found at
//! all) resolves to `None`: the caller (`super::state::bootstrap`) renders a
//! clean "no credentials plane" state, never an error.
//!
//! ## The admin key, environment only
//!
//! tokenfuse main (`crates/gateway/src/adminkeys.rs`) gates `/v1/keys` and
//! four sibling routes behind `TOKENFUSE_ADMIN_KEYS` once the gateway is
//! bound off loopback. Unlike `money::env`'s `cloud_admin_ref` /
//! `policy::env`'s `wardryx_admin_ref`, this plane's key has no descriptor
//! representation at all: the descriptor still owns the URL alone, and
//! [`ADMIN_KEY_ENV_VAR`] (`TOKENFUSE_GATEWAY_ADMIN_KEY`) is the ONLY source
//! for the key, read the same way `money::env`/`policy::env` read their own
//! env-fallback admin keys - trimmed, blank treated as unset. This is
//! deliberately not a third `EnvSource` variant: the URL still only ever
//! comes from a descriptor, so there is nothing to distinguish a "fallback"
//! source for.
//!
//! Never touches the network and never panics: every filesystem/JSON step is
//! a `?`-chained `Option`, so one malformed or half-written descriptor falls
//! through to the next candidate instead of taking down discovery - same
//! discipline `identity::env`/`drills::env` keep.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Env var carrying the gateway admin bearer key - see this module's doc
/// comment, "The admin key, environment only".
const ADMIN_KEY_ENV_VAR: &str = "TOKENFUSE_GATEWAY_ADMIN_KEY";

/// Where a [`ResolvedEnv`] came from, surfaced to the UI. A single variant
/// today, mirroring `identity::env::EnvSource`'s identical rationale: the
/// gateway read needs no key, so there is no env-fallback counterpart to
/// resolve a hand-started gateway from - only a discovered `taipan up`
/// descriptor. Kept as a tagged enum rather than a bare `{ name: String }` so
/// it stays structurally parallel to every other plane's `EnvSource`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum EnvSource {
    /// Discovered from `~/.taipan/environments/<name>.json`.
    Taipan { name: String },
}

/// A fully-resolved place to talk to the gateway.
#[derive(Debug, Clone)]
pub struct ResolvedEnv {
    pub source: EnvSource,
    pub gateway_url: String,
    /// `TOKENFUSE_GATEWAY_ADMIN_KEY`, trimmed, `None` when unset or blank -
    /// see this module's doc comment. Independent of `source`: the URL can
    /// come from a descriptor whether or not a key is configured.
    pub admin_key: Option<String>,
}

// ---- descriptor wire shape (read-only mirror) ------------------------------
// Deliberately duplicated from `identity::env`/`drills::env`'s own private
// structs rather than shared - see `identity::env`'s module doc for why.

#[derive(Debug, Deserialize)]
struct DescriptorService {
    url: String,
}

#[derive(Debug, Deserialize)]
struct Descriptor {
    name: String,
    services: BTreeMap<String, DescriptorService>,
}

/// Resolve the Credentials plane's environment: the newest `taipan up`
/// descriptor with a usable `services.gateway` entry, or `None` for a clean
/// "no credentials plane" state.
#[must_use]
pub fn discover() -> Option<ResolvedEnv> {
    let dir = genaryx_core::taipan_home::environments_dir()?;
    discover_taipan_in(&dir)
}

/// Testable core of the discovery path: scan `environments_dir` for
/// descriptor files (newest last-modified first), and return the first one
/// that yields a usable gateway URL.
fn discover_taipan_in(environments_dir: &Path) -> Option<ResolvedEnv> {
    let mut candidates = list_descriptor_paths(environments_dir);
    candidates.sort_by_key(|p| std::cmp::Reverse(modified_time(p)));
    candidates.into_iter().find_map(|p| try_load_descriptor(&p))
}

/// Every `<name>.json` descriptor in `dir`, excluding the sibling
/// `<name>.keys.json` / `<name>.pid.json` files - identical filter to
/// `identity::env::list_descriptor_paths`/`drills::env::list_descriptor_paths`.
fn list_descriptor_paths(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                return false;
            };
            name.ends_with(".json") && !name.ends_with(".keys.json") && !name.ends_with(".pid.json")
        })
        .collect()
}

fn modified_time(path: &Path) -> std::time::SystemTime {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
}

/// Load and resolve one descriptor: read the `gateway` service URL (falling
/// through to the next candidate when absent), same as
/// `drills::env::try_load_descriptor`'s equivalent step minus the bearer
/// resolution this plane has no use for.
fn try_load_descriptor(path: &Path) -> Option<ResolvedEnv> {
    let bytes = std::fs::read(path).ok()?;
    let descriptor: Descriptor = serde_json::from_slice(&bytes).ok()?;
    let gateway_url = descriptor.services.get("gateway")?.url.clone();
    Some(ResolvedEnv {
        source: EnvSource::Taipan {
            name: descriptor.name,
        },
        gateway_url,
        admin_key: admin_key_from_env(),
    })
}

/// `TOKENFUSE_GATEWAY_ADMIN_KEY`, read live from the process environment.
fn admin_key_from_env() -> Option<String> {
    admin_key_from(std::env::var(ADMIN_KEY_ENV_VAR).ok())
}

/// Testable core of [`admin_key_from_env`]: trims the raw value and treats
/// blank as unset, mirroring `money::env`/`policy::env`'s identical
/// `env_fallback_from` rule for their own admin-key env vars. Takes the
/// (already-read) value directly so tests never have to mutate real process
/// environment, same rationale as those modules' own tests.
fn admin_key_from(raw: Option<String>) -> Option<String> {
    let raw = raw?;
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "genaryx-credentials-env-test-{tag}-{}-{n}",
            std::process::id()
        ))
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(path, body).expect("write fixture file");
    }

    #[test]
    fn empty_directory_yields_no_candidate() {
        let dir = unique_dir("empty");
        std::fs::create_dir_all(&dir).expect("create dir");
        assert!(discover_taipan_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_directory_yields_no_candidate_not_a_panic() {
        let dir = unique_dir("missing").join("nested").join("deeper");
        assert!(discover_taipan_in(&dir).is_none());
    }

    #[test]
    fn ignores_keys_json_and_pid_json_as_descriptor_candidates() {
        let dir = unique_dir("siblings");
        write(
            &dir.join("p1full.keys.json"),
            r#"{"name":"p1full","secrets":{}}"#,
        );
        write(
            &dir.join("p1full.pid.json"),
            r#"{"name":"p1full","processes":[]}"#,
        );
        assert!(discover_taipan_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_descriptor_with_no_gateway_service_falls_through() {
        let dir = unique_dir("no-gateway");
        write(
            &dir.join("plain.json"),
            r#"{"name":"plain","services":{"cloud":{"url":"http://x"}}}"#,
        );
        assert!(discover_taipan_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_a_real_shaped_descriptor() {
        let dir = unique_dir("happy");
        write(
            &dir.join("p1full.json"),
            r#"{
                "name": "p1full",
                "services": {
                    "cloud": {"url": "http://127.0.0.1:41001"},
                    "gateway": {"url": "http://127.0.0.1:4100", "mode": "enforce"}
                }
            }"#,
        );
        let resolved = discover_taipan_in(&dir).expect("must resolve the fixture descriptor");
        assert_eq!(
            resolved.source,
            EnvSource::Taipan {
                name: "p1full".to_string()
            }
        );
        assert_eq!(resolved.gateway_url, "http://127.0.0.1:4100");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn newest_descriptor_wins_when_multiple_environments_exist() {
        let dir = unique_dir("multi");
        write(
            &dir.join("older.json"),
            r#"{"name":"older","services":{"gateway":{"url":"http://127.0.0.1:1"}}}"#,
        );
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(
            &dir.join("newer.json"),
            r#"{"name":"newer","services":{"gateway":{"url":"http://127.0.0.1:2"}}}"#,
        );

        let resolved = discover_taipan_in(&dir).expect("must resolve one of the two");
        assert_eq!(
            resolved.source,
            EnvSource::Taipan {
                name: "newer".to_string()
            }
        );
        assert_eq!(resolved.gateway_url, "http://127.0.0.1:2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- admin key (T2, PLAN-GATEWAY-ADMIN-KEY-2026-09-07.md) --------------

    #[test]
    fn admin_key_from_env_is_none_when_unset() {
        assert!(admin_key_from(None).is_none());
    }

    #[test]
    fn admin_key_from_env_treats_blank_as_unset() {
        assert!(admin_key_from(Some(String::new())).is_none());
        assert!(admin_key_from(Some("   ".to_string())).is_none());
    }

    #[test]
    fn admin_key_from_env_trims_a_configured_key() {
        assert_eq!(
            admin_key_from(Some("  sk-gateway-admin  ".to_string())),
            Some("sk-gateway-admin".to_string())
        );
    }

    #[test]
    fn a_resolved_descriptor_carries_whatever_admin_key_from_env_would() {
        // The URL always comes from the descriptor; the key always comes
        // from the environment, independent of the descriptor - proven here
        // by confirming a resolved candidate's `admin_key` matches
        // `admin_key_from_env()` read at the same moment, rather than
        // anything baked into the fixture.
        let dir = unique_dir("admin-key-independence");
        write(
            &dir.join("p1full.json"),
            r#"{"name":"p1full","services":{"gateway":{"url":"http://127.0.0.1:4100"}}}"#,
        );
        let resolved = discover_taipan_in(&dir).expect("must resolve the fixture descriptor");
        assert_eq!(resolved.admin_key, admin_key_from_env());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
