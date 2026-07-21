//! Policy-panel environment discovery: which Wardryx to talk to, and with
//! which admin bearer key.
//!
//! Mirrors `crate::money::env` exactly in shape (two sources, tried in
//! order), deliberately duplicated rather than shared (07 §4.3's Wardryx
//! connector is its own independent plane, same "parallel, not coupled"
//! convention `crates/connectors/src/wardryx.rs` already keeps relative to
//! `cloud_rest.rs`):
//!
//! 1. The SAME `taipan up` descriptor `money::env` reads
//!    (`~/.taipan/environments/<name>.json`), just a different service entry
//!    (`services.wardryx` instead of `services.cloud`) and a different key
//!    ref (`keys.wardryx_admin_ref` instead of `keys.cloud_admin_ref`,
//!    resolved the same way: a reference into the sibling
//!    `<name>.keys.json` file's `secrets` map). Per docs/PHASE1.md's
//!    wave-5 closing note ("the keyfile secret is the bare token, the
//!    server env keeps the full spec" - issue #20's fix), the resolved
//!    secret is already the bare token `WardryxClient::new` expects; no
//!    splitting is done here.
//! 2. A `WARDRYX_URL` + `WARDRYX_ADMIN_KEY` env fallback, for a Wardryx
//!    started by hand (no `taipan up`) during local development.
//!    `WARDRYX_ADMIN_KEY` gates the fallback (mirroring
//!    `money::env`'s `TOKENFUSE_CLOUD_ADMIN_KEY`); `WARDRYX_URL` is optional
//!    and defaults to `http://127.0.0.1:8090` - wardryx's own documented
//!    default port (`crates/connectors/src/wardryx.rs`'s module doc
//!    example, confirmed again by `wardryx_test.rs`'s "never 8090" comment).
//!
//! Neither resolving is an error: [`discover`] returns `None` and the
//! caller (`super::state::bootstrap`) leaves the Policy panel in a clean "no
//! policy plane" state - this is also exactly what happens for a `taipan up`
//! stack that never passed `--with wardryx` (no `services.wardryx` entry at
//! all), which is the common case today (PHASE1.md: "gateway+cloud
//! (+wardryx/idryx via --with)"). This module never touches the network and
//! never panics: every filesystem/JSON step is a `?`-chained `Option`, so
//! one malformed or half-written descriptor falls through to the next
//! candidate, or to the env-var fallback, instead of taking down discovery.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Wardryx's own documented default bind address
/// (`crates/connectors/src/wardryx.rs`'s module doc example:
/// `WardryxClient::new("http://127.0.0.1:8090", "tk_ops")`), used only when
/// `WARDRYX_URL` is unset/blank but `WARDRYX_ADMIN_KEY` is not - mirrors
/// `money::env::FALLBACK_CLOUD_URL`'s identical role for tokenfuse-cloud.
const FALLBACK_WARDRYX_URL: &str = "http://127.0.0.1:8090";

/// Env var carrying the Wardryx base URL for the no-descriptor fallback.
const URL_ENV_VAR: &str = "WARDRYX_URL";

/// Env var carrying the admin bearer token (the BARE token half of a
/// `WARDRYX_KEYS="token:org[:role],..."` entry) for the no-descriptor
/// fallback.
const ADMIN_KEY_ENV_VAR: &str = "WARDRYX_ADMIN_KEY";

/// Where a [`ResolvedEnv`] came from, surfaced to the UI - identical shape
/// to `money::env::EnvSource` (same wire tag/rename_all), so the frontend's
/// `EnvSource` type mirrors one Rust shape used by both panels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum EnvSource {
    /// Discovered from `~/.taipan/environments/<name>.json`.
    Taipan { name: String },
    /// No usable descriptor found; `WARDRYX_URL` (or its default) +
    /// `WARDRYX_ADMIN_KEY`.
    EnvFallback,
}

/// A fully-resolved place to talk to Wardryx: base URL plus the BARE admin
/// bearer token. Unlike `money::env::ResolvedEnv`, there is no separate org
/// carried here - Wardryx has no pairing handshake to learn it from (see
/// `super::state`'s module docs for how the policy panel derives an
/// `org_domain` for its own journal entries without one).
#[derive(Debug, Clone)]
pub struct ResolvedEnv {
    pub source: EnvSource,
    pub wardryx_url: String,
    pub admin_bearer: String,
}

// ---- descriptor / keyfile wire shapes (read-only mirror) ------------------
// Deliberately duplicated from `money::env`'s identical private structs
// rather than shared: this module's only coupling to the taipan descriptor
// format is "read these two fields out of the same JSON file", which is
// cheaper to keep as two small, independent mirrors than to introduce a
// money <-> policy dependency for.

#[derive(Debug, Deserialize)]
struct DescriptorService {
    url: String,
}

#[derive(Debug, Default, Deserialize)]
struct DescriptorKeys {
    #[serde(default)]
    wardryx_admin_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Descriptor {
    name: String,
    services: BTreeMap<String, DescriptorService>,
    #[serde(default)]
    keys: DescriptorKeys,
}

#[derive(Debug, Deserialize)]
struct KeyFile {
    #[serde(default)]
    secrets: BTreeMap<String, String>,
}

/// Resolve the Policy panel's environment: prefer a `taipan up` descriptor,
/// fall back to the env vars, or `None` for a clean "no policy plane" state.
#[must_use]
pub fn discover() -> Option<ResolvedEnv> {
    if let Some(dir) = genaryx_core::taipan_home::environments_dir()
        && let Some(env) = discover_taipan_in(&dir)
    {
        return Some(env);
    }
    discover_env_fallback()
}

/// Testable core of the taipan-descriptor path: scan `environments_dir` for
/// descriptor files (newest last-modified first), and return the first one
/// that yields a usable Wardryx URL and a resolvable admin key. A descriptor
/// with no `services.wardryx` entry at all (no `--with wardryx`) simply
/// yields `None` for that candidate, same as any other missing field.
fn discover_taipan_in(environments_dir: &Path) -> Option<ResolvedEnv> {
    let mut candidates = list_descriptor_paths(environments_dir);
    candidates.sort_by_key(|p| std::cmp::Reverse(modified_time(p)));
    candidates.into_iter().find_map(|p| try_load_descriptor(&p))
}

/// Every `<name>.json` descriptor in `dir`, excluding the sibling
/// `<name>.keys.json` / `<name>.pid.json` files - identical filter to
/// `money::env::list_descriptor_paths`.
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

/// Load and resolve one descriptor: read the `wardryx` service URL and
/// follow `keys.wardryx_admin_ref` (`"taipan/<name>/<label>"`, only the
/// trailing `<label>` segment is needed - the key into the sibling
/// keyfile's `secrets` map) to the real bearer token in `<name>.keys.json`.
/// `None` at any step falls through to the next candidate rather than
/// erroring - mirrors `money::env::try_load_descriptor` field-for-field,
/// substituting the `wardryx` service and `wardryx_admin_ref` key.
fn try_load_descriptor(path: &Path) -> Option<ResolvedEnv> {
    let bytes = std::fs::read(path).ok()?;
    let descriptor: Descriptor = serde_json::from_slice(&bytes).ok()?;

    let wardryx_url = descriptor.services.get("wardryx")?.url.clone();
    let admin_ref = descriptor.keys.wardryx_admin_ref?;
    let label = admin_ref.rsplit('/').next()?;

    let keys_path = path.with_file_name(format!("{}.keys.json", descriptor.name));
    let key_bytes = std::fs::read(&keys_path).ok()?;
    let keyfile: KeyFile = serde_json::from_slice(&key_bytes).ok()?;
    let admin_bearer = keyfile.secrets.get(label)?.clone();

    Some(ResolvedEnv {
        source: EnvSource::Taipan {
            name: descriptor.name,
        },
        wardryx_url,
        admin_bearer,
    })
}

/// `WARDRYX_URL` (or [`FALLBACK_WARDRYX_URL`]) + `WARDRYX_ADMIN_KEY`, for a
/// Wardryx started without `taipan up`. `None` when the admin key is unset
/// or blank - the URL alone is never enough to activate this path, mirroring
/// `money::env::discover_env_fallback`'s "the key gates the fallback" rule.
fn discover_env_fallback() -> Option<ResolvedEnv> {
    env_fallback_from(
        std::env::var(URL_ENV_VAR).ok(),
        std::env::var(ADMIN_KEY_ENV_VAR).ok(),
    )
}

/// Testable core of [`discover_env_fallback`], taking the (already-read) env
/// var values directly so tests never have to mutate real process
/// environment - mirrors `money::env::env_fallback_from`'s identical
/// rationale.
fn env_fallback_from(url: Option<String>, admin_bearer: Option<String>) -> Option<ResolvedEnv> {
    let admin_bearer = admin_bearer?;
    if admin_bearer.trim().is_empty() {
        return None;
    }
    let wardryx_url = url
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| FALLBACK_WARDRYX_URL.to_string());
    Some(ResolvedEnv {
        source: EnvSource::EnvFallback,
        wardryx_url,
        admin_bearer,
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
            "genaryx-policy-env-test-{tag}-{}-{n}",
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
    fn resolves_a_real_shaped_descriptor_and_keyfile() {
        let dir = unique_dir("happy");
        write(
            &dir.join("p1full.json"),
            r#"{
                "name": "p1full",
                "created_at": "2026-07-16T00:00:00Z",
                "host": "box.local",
                "services": {
                    "cloud": {"url": "http://127.0.0.1:41001"},
                    "wardryx": {"url": "http://127.0.0.1:41002"}
                },
                "events": {"dir": "/tmp/x", "files": {}},
                "keys": {
                    "cloud_admin_ref": "taipan/p1full/cloud_admin",
                    "wardryx_admin_ref": "taipan/p1full/wardryx_admin"
                }
            }"#,
        );
        write(
            &dir.join("p1full.keys.json"),
            r#"{
                "name": "p1full",
                "created_at": "2026-07-16T00:00:00Z",
                "secrets": {
                    "cloud_admin": "tp_deadbeef",
                    "wardryx_admin": "tk_deadbeef"
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
        assert_eq!(resolved.wardryx_url, "http://127.0.0.1:41002");
        assert_eq!(resolved.admin_bearer, "tk_deadbeef");

        let _ = std::fs::remove_dir_all(&dir);
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
    fn a_descriptor_with_no_wardryx_service_falls_through() {
        // The common case today (PHASE1.md: "gateway+cloud (+wardryx/idryx
        // via --with)") - a stack brought up without `--with wardryx` has a
        // `cloud` service but no `wardryx` one at all.
        let dir = unique_dir("no-wardryx");
        write(
            &dir.join("plain.json"),
            r#"{"name":"plain","services":{"cloud":{"url":"http://x"}},"keys":{}}"#,
        );
        assert!(discover_taipan_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_descriptor_with_no_matching_secret_falls_through() {
        let dir = unique_dir("no-secret");
        write(
            &dir.join("orphan.json"),
            r#"{"name":"orphan","services":{"wardryx":{"url":"http://127.0.0.1:1"}},
                "keys":{"wardryx_admin_ref":"taipan/orphan/wardryx_admin"}}"#,
        );
        // No sibling keyfile at all.
        assert!(discover_taipan_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn newest_descriptor_wins_when_multiple_environments_exist() {
        let dir = unique_dir("multi");
        write(
            &dir.join("older.json"),
            r#"{"name":"older","services":{"wardryx":{"url":"http://127.0.0.1:1"}},
                "keys":{"wardryx_admin_ref":"taipan/older/wardryx_admin"}}"#,
        );
        write(
            &dir.join("older.keys.json"),
            r#"{"name":"older","secrets":{"wardryx_admin":"tk_old"}}"#,
        );
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(
            &dir.join("newer.json"),
            r#"{"name":"newer","services":{"wardryx":{"url":"http://127.0.0.1:2"}},
                "keys":{"wardryx_admin_ref":"taipan/newer/wardryx_admin"}}"#,
        );
        write(
            &dir.join("newer.keys.json"),
            r#"{"name":"newer","secrets":{"wardryx_admin":"tk_new"}}"#,
        );

        let resolved = discover_taipan_in(&dir).expect("must resolve one of the two");
        assert_eq!(
            resolved.source,
            EnvSource::Taipan {
                name: "newer".to_string()
            }
        );
        assert_eq!(resolved.wardryx_url, "http://127.0.0.1:2");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_fallback_requires_a_non_blank_admin_key() {
        assert!(env_fallback_from(None, None).is_none());
        assert!(env_fallback_from(Some("http://x".to_string()), None).is_none());
        assert!(env_fallback_from(None, Some(String::new())).is_none());
        assert!(env_fallback_from(None, Some("   ".to_string())).is_none());

        let resolved = env_fallback_from(None, Some("tk_x".to_string()))
            .expect("a non-blank key must resolve");
        assert_eq!(resolved.source, EnvSource::EnvFallback);
        assert_eq!(resolved.wardryx_url, FALLBACK_WARDRYX_URL);
        assert_eq!(resolved.admin_bearer, "tk_x");
    }

    #[test]
    fn env_fallback_uses_an_explicit_url_when_given() {
        let resolved = env_fallback_from(
            Some("http://127.0.0.1:9999".to_string()),
            Some("tk_x".to_string()),
        )
        .expect("must resolve");
        assert_eq!(resolved.wardryx_url, "http://127.0.0.1:9999");
    }

    #[test]
    fn env_fallback_treats_a_blank_url_as_unset() {
        let resolved = env_fallback_from(Some("   ".to_string()), Some("tk_x".to_string()))
            .expect("must resolve");
        assert_eq!(resolved.wardryx_url, FALLBACK_WARDRYX_URL);
    }
}
