//! Credentials-plane environment discovery: which gateway to talk to.
//!
//! Mirrors `crate::identity::env`'s shape (one `Taipan`-only [`EnvSource`],
//! `?`-chained `Option`s throughout, newest-descriptor-wins), resolving the
//! SAME `services.gateway.url` `crate::drills::env` reads off a `taipan up`
//! descriptor - explicitly NOT `services.cloud` (TokenFuse Cloud's own admin
//! API, `crate::money::env`'s target). Like idryx, the gateway's `/v1/keys`
//! read needs no key or auth at all (see `genaryx_connectors::gateway`'s
//! module doc), so unlike `drills::env` there is no bearer to resolve
//! alongside the URL, and unlike `identity::env` there is no extra `events`
//! section to carry either - this plane's only need is the one URL.
//!
//! A descriptor with no `services.gateway` entry (or no descriptor found at
//! all) resolves to `None`: the caller (`super::state::bootstrap`) renders a
//! clean "no credentials plane" state, never an error.
//!
//! Never touches the network and never panics: every filesystem/JSON step is
//! a `?`-chained `Option`, so one malformed or half-written descriptor falls
//! through to the next candidate instead of taking down discovery - same
//! discipline `identity::env`/`drills::env` keep.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
    })
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
}
