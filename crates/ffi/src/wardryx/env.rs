//! Environment discovery for [`super::WardryxHandle`]: which Wardryx policy
//! plane to talk to, and with which admin bearer key.
//!
//! A line-for-line mirror of [`crate::cloud::env`] (docs/PHASE2.md wave 2,
//! "PARITY across both shells"), swapped from Cloud to Wardryx: `services`
//! key `"cloud"` -> `"wardryx"`, key reference `cloud_admin_ref` ->
//! `wardryx_admin_ref`, env var `TOKENFUSE_CLOUD_ADMIN_KEY` ->
//! `WARDRYX_ADMIN_KEY`, fallback URL `127.0.0.1:8080` ->
//! `127.0.0.1:8090` (`crates/connectors/tests/wardryx_test.rs`'s own doc
//! comment: "an uncommon port so a real 8090 stack is never touched... 8090
//! (wardryx's documented default)"). This module is otherwise a deliberate,
//! self-contained copy rather than a shared import of `cloud::env` (see
//! `crates/ffi/src/wardryx/mod.rs`'s module doc for why): two sources, tried
//! in order:
//!
//! 1. A `taipan up` descriptor under `~/.taipan/environments/<name>.json`.
//!    The admin bearer key is never embedded in the descriptor: it is a
//!    reference (`keys.wardryx_admin_ref`, `"taipan/<name>/<label>"`) into a
//!    sibling `<name>.keys.json` file, where the real secret lives.
//! 2. `http://127.0.0.1:8090` + the `WARDRYX_ADMIN_KEY` env var, for a
//!    Wardryx started by hand (no `taipan up`) during local development.
//!
//! A descriptor whose `services` map simply has no `"wardryx"` entry (a
//! `taipan up` run without `--with wardryx`) is not an error either: it
//! falls through exactly like a missing/unresolvable key does anywhere else
//! in this module, ultimately yielding [`discover`] `None` and
//! [`super::WardryxHandle::discover`] failing closed with
//! `WardryxError::NoEnvironment` - PHASE2.md's "no wardryx service resolves
//! to a clean 'no policy plane' empty state, never an error". This module
//! never touches the network.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `http://127.0.0.1:8090` is wardryx's own documented default bind address
/// (`crates/connectors/tests/wardryx_test.rs`'s `free_port` doc comment;
/// `crates/connectors/src/wardryx.rs`'s module example and
/// `WardryxClient::new`'s own doc comment both illustrate the same port).
const FALLBACK_WARDRYX_URL: &str = "http://127.0.0.1:8090";

/// Env var carrying the admin bearer key (the BARE token half of a
/// `WARDRYX_KEYS="token:org[:role],..."` entry - see
/// `genaryx_connectors::WardryxClient`'s own doc comment for why sending the
/// full spec instead 401s) for the no-descriptor fallback.
const ADMIN_KEY_ENV_VAR: &str = "WARDRYX_ADMIN_KEY";

/// Where a [`ResolvedEnv`] came from, surfaced to the Swift shell so the
/// operator can always see what the console is actually talking to (06
/// §0.5), exported as a UniFFI enum - Swift sees `.taipan(name:)` /
/// `.envFallback`. Named distinctly from [`crate::cloud::EnvSource`] (rather
/// than reused) because UniFFI's generated namespace is flat per crate: two
/// Rust types named `EnvSource` in the same crate, even in different
/// modules, would collide once both are exported to one generated
/// `genaryx_ffi.swift`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum WardryxEnvSource {
    /// Discovered from `~/.taipan/environments/<name>.json`.
    Taipan { name: String },
    /// No usable descriptor found; `127.0.0.1:8090` + `WARDRYX_ADMIN_KEY`.
    EnvFallback,
}

/// A fully-resolved place to build a [`genaryx_connectors::WardryxClient`]
/// against. Not itself exported over FFI (the admin bearer must never cross
/// into Swift as a plain value beyond what construction already consumes it
/// for); `WardryxHandle` exposes only [`WardryxEnvSource`] and the Wardryx
/// URL back out.
#[derive(Debug, Clone)]
pub struct ResolvedEnv {
    pub source: WardryxEnvSource,
    pub wardryx_url: String,
    pub admin_bearer: String,
}

// ---- descriptor / keyfile wire shapes (read-only mirror) ------------------
// Only the fields this module actually reads are modeled; `#[serde(default)]`
// / unknown-field tolerance throughout so a descriptor with extra fields
// never fails to parse.

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
/// fall back to the env var, or `None` for a clean "no environment" state.
#[must_use]
pub fn discover() -> Option<ResolvedEnv> {
    if let Some(dir) = taipan_environments_dir()
        && let Some(env) = discover_taipan_in(&dir)
    {
        return Some(env);
    }
    discover_env_fallback()
}

/// `~/.taipan/environments`, or `None` when `$HOME` is not set (never a
/// panic over a missing env var).
fn taipan_environments_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".taipan").join("environments"))
}

/// Testable core of the taipan-descriptor path: scan `environments_dir` for
/// descriptor files (newest last-modified first, so the most recently
/// `taipan up`'d environment wins when more than one exists), and return the
/// first one that yields a usable Wardryx URL and a resolvable admin key.
fn discover_taipan_in(environments_dir: &Path) -> Option<ResolvedEnv> {
    let mut candidates = list_descriptor_paths(environments_dir);
    candidates.sort_by_key(|p| std::cmp::Reverse(modified_time(p)));
    candidates.into_iter().find_map(|p| try_load_descriptor(&p))
}

/// Every `<name>.json` descriptor in `dir`, excluding the sibling
/// `<name>.keys.json` / `<name>.pid.json` files. An unreadable directory (not
/// yet created) yields no candidates rather than an error.
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

/// Load and resolve one descriptor: read the `wardryx` service URL (`None`,
/// falling through to the next candidate, when a descriptor was written by a
/// `taipan up` that never started wardryx at all) and follow
/// `keys.wardryx_admin_ref` (`"taipan/<name>/<label>"`, only the trailing
/// `<label>` segment is actually needed) to the real bearer key in
/// `<name>.keys.json`. `None` at any step falls through to the next
/// candidate rather than erroring.
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
        source: WardryxEnvSource::Taipan {
            name: descriptor.name,
        },
        wardryx_url,
        admin_bearer,
    })
}

/// `127.0.0.1:8090` + `WARDRYX_ADMIN_KEY`, for a Wardryx started without
/// `taipan up`. `None` when the var is unset or blank.
fn discover_env_fallback() -> Option<ResolvedEnv> {
    env_fallback_from(std::env::var(ADMIN_KEY_ENV_VAR).ok())
}

/// Testable core of [`discover_env_fallback`], taking the (already-read) env
/// var value directly so tests never have to mutate real process
/// environment (which `cargo test`'s parallel-by-default threads make
/// inherently racy across a shared process).
fn env_fallback_from(admin_bearer: Option<String>) -> Option<ResolvedEnv> {
    let admin_bearer = admin_bearer?;
    if admin_bearer.trim().is_empty() {
        return None;
    }
    Some(ResolvedEnv {
        source: WardryxEnvSource::EnvFallback,
        wardryx_url: FALLBACK_WARDRYX_URL.to_string(),
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
            "genaryx-ffi-wardryx-env-test-{tag}-{}-{n}",
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
                    "wardryx": {"url": "http://127.0.0.1:41002"},
                    "gateway": {"url": "http://127.0.0.1:41000", "mode": "enforce"}
                },
                "events": {"dir": "/tmp/x", "files": {}},
                "keys": {
                    "wardryx_admin_ref": "taipan/p1full/wardryx_admin",
                    "wardryx_viewer_ref": "taipan/p1full/wardryx_viewer"
                }
            }"#,
        );
        write(
            &dir.join("p1full.keys.json"),
            r#"{
                "name": "p1full",
                "created_at": "2026-07-16T00:00:00Z",
                "secrets": {
                    "wardryx_admin": "tk_deadbeef:taipan-p1full:admin",
                    "wardryx_viewer": "tk_c0ffee:taipan-p1full:viewer"
                }
            }"#,
        );

        let resolved = discover_taipan_in(&dir).expect("must resolve the fixture descriptor");
        assert_eq!(
            resolved.source,
            WardryxEnvSource::Taipan {
                name: "p1full".to_string()
            }
        );
        assert_eq!(resolved.wardryx_url, "http://127.0.0.1:41002");
        assert_eq!(resolved.admin_bearer, "tk_deadbeef:taipan-p1full:admin");

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

    /// A descriptor from a `taipan up` that never started wardryx at all
    /// (no `services.wardryx` key) must fall through cleanly, never error -
    /// this is the "no policy plane" empty-state case (PHASE2.md).
    #[test]
    fn a_descriptor_missing_the_wardryx_service_falls_through() {
        let dir = unique_dir("no-wardryx");
        write(
            &dir.join("broken.json"),
            r#"{"name":"broken","services":{"cloud":{"url":"http://x"}},"keys":{}}"#,
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
            r#"{"name":"older","secrets":{"wardryx_admin":"tk_old:org:admin"}}"#,
        );
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(
            &dir.join("newer.json"),
            r#"{"name":"newer","services":{"wardryx":{"url":"http://127.0.0.1:2"}},
                "keys":{"wardryx_admin_ref":"taipan/newer/wardryx_admin"}}"#,
        );
        write(
            &dir.join("newer.keys.json"),
            r#"{"name":"newer","secrets":{"wardryx_admin":"tk_new:org:admin"}}"#,
        );

        let resolved = discover_taipan_in(&dir).expect("must resolve one of the two");
        assert_eq!(
            resolved.source,
            WardryxEnvSource::Taipan {
                name: "newer".to_string()
            }
        );
        assert_eq!(resolved.wardryx_url, "http://127.0.0.1:2");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_fallback_requires_a_non_blank_admin_key() {
        assert!(env_fallback_from(None).is_none());
        assert!(env_fallback_from(Some(String::new())).is_none());
        assert!(env_fallback_from(Some("   ".to_string())).is_none());

        let resolved = env_fallback_from(Some("tk_x:acme:admin".to_string()))
            .expect("a non-blank key must resolve");
        assert_eq!(resolved.source, WardryxEnvSource::EnvFallback);
        assert_eq!(resolved.wardryx_url, FALLBACK_WARDRYX_URL);
        assert_eq!(resolved.admin_bearer, "tk_x:acme:admin");
    }
}
