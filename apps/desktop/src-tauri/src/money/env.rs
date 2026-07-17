//! Money-panel environment discovery: which TokenFuse Cloud to talk to, and
//! with which admin bearer key.
//!
//! Two sources, tried in order, matching the task spec exactly:
//!
//! 1. A `taipan up` descriptor under `~/.taipan/environments/<name>.json`
//!    (the exact artifact the wider itrat-console design calls "the file
//!    consumers auto-discover" - see `~/Development/taipan/src/descriptor.rs`
//!    and `home.rs`, read directly as ground truth for this shape). The
//!    admin bearer key itself is never embedded in the descriptor: it is a
//!    reference (`keys.cloud_admin_ref`, `"taipan/<name>/<label>"`) into a
//!    sibling `<name>.keys.json` file (mode 0600), which is where the real
//!    secret lives.
//! 2. `http://127.0.0.1:8080` + the `TOKENFUSE_CLOUD_ADMIN_KEY` env var, for
//!    a Cloud started by hand (no `taipan up`) during local development.
//!
//! Neither resolving is not an error: [`discover`] returns `None` and the
//! caller (`super::state::bootstrap`) leaves the Money panel in a clean "no
//! environment" state rather than failing app startup. This module never
//! touches the network and never panics: every filesystem/JSON step is a
//! `?`-chained `Option`, so one malformed or half-written descriptor (e.g. a
//! `taipan up` still in progress) just falls through to the next candidate,
//! or to the env-var fallback, instead of taking down discovery entirely.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `http://127.0.0.1:8080` is `tokenfuse-cloud`'s own documented default bind
/// address (matching `crates/connectors/tests/cloud_rest_test.rs`'s and
/// `docs/PHASE1.md`'s local dev-loop conventions).
const FALLBACK_CLOUD_URL: &str = "http://127.0.0.1:8080";

/// Env var carrying the admin bearer key (`key:org[:role][:plan]`, or a
/// devkey when the Cloud was started with `TOKENFUSE_CLOUD_ALLOW_DEVKEY=1`)
/// for the no-descriptor fallback.
const ADMIN_KEY_ENV_VAR: &str = "TOKENFUSE_CLOUD_ADMIN_KEY";

/// Where a [`ResolvedEnv`] came from, surfaced to the UI (06 §0.5 - the
/// operator should always be able to see what the console is actually
/// talking to, never a silently-assumed default).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum EnvSource {
    /// Discovered from `~/.taipan/environments/<name>.json`.
    Taipan { name: String },
    /// No usable descriptor found; `127.0.0.1:8080` + `TOKENFUSE_CLOUD_ADMIN_KEY`.
    EnvFallback,
}

/// A fully-resolved place to pair a device and read/mutate against.
#[derive(Debug, Clone)]
pub struct ResolvedEnv {
    pub source: EnvSource,
    pub cloud_url: String,
    pub admin_bearer: String,
}

// ---- descriptor / keyfile wire shapes (read-only mirror) ------------------
// Deliberately NOT a dependency on the `taipan` binary crate (a sibling repo,
// not part of this workspace): this is the same kind of narrow, field-for-
// field wire mirror `src-tauri/src/events.rs`'s `UiEvent` already is for
// `genaryx_core::store::StoredEvent`. Only the fields this module actually
// reads are modeled; `#[serde(default)]`/unknown-field tolerance throughout
// so a descriptor with extra fields (`unavailable`, `logs_dir`, more
// services) never fails to parse.

#[derive(Debug, Deserialize)]
struct DescriptorService {
    url: String,
}

#[derive(Debug, Default, Deserialize)]
struct DescriptorKeys {
    #[serde(default)]
    cloud_admin_ref: Option<String>,
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

/// Resolve the Money panel's environment: prefer a `taipan up` descriptor,
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
/// panic over a missing env var - mirrors `taipan`'s own `TaipanHome::discover`).
fn taipan_environments_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".taipan").join("environments"))
}

/// Testable core of the taipan-descriptor path: scan `environments_dir` for
/// descriptor files (newest last-modified first, so the most recently
/// `taipan up`'d environment wins when more than one exists), and return the
/// first one that yields a usable Cloud URL and a resolvable admin key.
fn discover_taipan_in(environments_dir: &Path) -> Option<ResolvedEnv> {
    let mut candidates = list_descriptor_paths(environments_dir);
    candidates.sort_by_key(|p| std::cmp::Reverse(modified_time(p)));
    candidates.into_iter().find_map(|p| try_load_descriptor(&p))
}

/// Every `<name>.json` descriptor in `dir`, excluding the sibling
/// `<name>.keys.json` / `<name>.pid.json` files (which also end in `.json`
/// via `Path::extension`'s "last component only" rule, so a suffix check is
/// used instead of an extension check). An unreadable directory (not yet
/// created - no environment has ever been brought up on this box) yields no
/// candidates rather than an error.
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

/// Load and resolve one descriptor: read the `cloud` service URL and follow
/// `keys.cloud_admin_ref` (`"taipan/<name>/<label>"`, only the trailing
/// `<label>` segment is actually needed - it is the key into the sibling
/// keyfile's `secrets` map, see `~/Development/taipan/src/keys.rs::key_ref`)
/// to the real bearer key in `<name>.keys.json`. `None` at any step (missing
/// file, bad JSON, no `cloud` service, no admin ref, no matching secret)
/// falls through to the next candidate rather than erroring.
fn try_load_descriptor(path: &Path) -> Option<ResolvedEnv> {
    let bytes = std::fs::read(path).ok()?;
    let descriptor: Descriptor = serde_json::from_slice(&bytes).ok()?;

    let cloud_url = descriptor.services.get("cloud")?.url.clone();
    let admin_ref = descriptor.keys.cloud_admin_ref?;
    let label = admin_ref.rsplit('/').next()?;

    let keys_path = path.with_file_name(format!("{}.keys.json", descriptor.name));
    let key_bytes = std::fs::read(&keys_path).ok()?;
    let keyfile: KeyFile = serde_json::from_slice(&key_bytes).ok()?;
    let admin_bearer = keyfile.secrets.get(label)?.clone();

    Some(ResolvedEnv {
        source: EnvSource::Taipan {
            name: descriptor.name,
        },
        cloud_url,
        admin_bearer,
    })
}

/// `127.0.0.1:8080` + `TOKENFUSE_CLOUD_ADMIN_KEY`, for a Cloud started
/// without `taipan up`. `None` when the var is unset or blank.
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
        source: EnvSource::EnvFallback,
        cloud_url: FALLBACK_CLOUD_URL.to_string(),
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
            "genaryx-money-env-test-{tag}-{}-{n}",
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
                    "gateway": {"url": "http://127.0.0.1:41000", "mode": "enforce"}
                },
                "events": {"dir": "/tmp/x", "files": {}},
                "keys": {
                    "cloud_admin_ref": "taipan/p1full/cloud_admin",
                    "cloud_viewer_ref": "taipan/p1full/cloud_viewer"
                }
            }"#,
        );
        write(
            &dir.join("p1full.keys.json"),
            r#"{
                "name": "p1full",
                "created_at": "2026-07-16T00:00:00Z",
                "secrets": {
                    "cloud_admin": "tp_deadbeef:taipan-p1full:admin",
                    "cloud_viewer": "tp_c0ffee:taipan-p1full:viewer"
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
        assert_eq!(resolved.cloud_url, "http://127.0.0.1:41001");
        assert_eq!(resolved.admin_bearer, "tp_deadbeef:taipan-p1full:admin");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ignores_keys_json_and_pid_json_as_descriptor_candidates() {
        let dir = unique_dir("siblings");
        // No `<name>.json` at all - only the sibling files a real environment
        // also leaves behind. These must never be mistaken for descriptors.
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
    fn a_descriptor_missing_the_cloud_service_falls_through() {
        let dir = unique_dir("no-cloud");
        write(
            &dir.join("broken.json"),
            r#"{"name":"broken","services":{"gateway":{"url":"http://x"}},"keys":{}}"#,
        );
        assert!(discover_taipan_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_descriptor_with_no_matching_secret_falls_through() {
        let dir = unique_dir("no-secret");
        write(
            &dir.join("orphan.json"),
            r#"{"name":"orphan","services":{"cloud":{"url":"http://127.0.0.1:1"}},
                "keys":{"cloud_admin_ref":"taipan/orphan/cloud_admin"}}"#,
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
            r#"{"name":"older","services":{"cloud":{"url":"http://127.0.0.1:1"}},
                "keys":{"cloud_admin_ref":"taipan/older/cloud_admin"}}"#,
        );
        write(
            &dir.join("older.keys.json"),
            r#"{"name":"older","secrets":{"cloud_admin":"tp_old:org:admin"}}"#,
        );
        // Ensure a strictly later mtime than `older.json` even on coarse
        // filesystem timestamp resolution.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(
            &dir.join("newer.json"),
            r#"{"name":"newer","services":{"cloud":{"url":"http://127.0.0.1:2"}},
                "keys":{"cloud_admin_ref":"taipan/newer/cloud_admin"}}"#,
        );
        write(
            &dir.join("newer.keys.json"),
            r#"{"name":"newer","secrets":{"cloud_admin":"tp_new:org:admin"}}"#,
        );

        let resolved = discover_taipan_in(&dir).expect("must resolve one of the two");
        assert_eq!(
            resolved.source,
            EnvSource::Taipan {
                name: "newer".to_string()
            }
        );
        assert_eq!(resolved.cloud_url, "http://127.0.0.1:2");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_fallback_requires_a_non_blank_admin_key() {
        assert!(env_fallback_from(None).is_none());
        assert!(env_fallback_from(Some(String::new())).is_none());
        assert!(env_fallback_from(Some("   ".to_string())).is_none());

        let resolved = env_fallback_from(Some("tp_x:acme:admin".to_string()))
            .expect("a non-blank key must resolve");
        assert_eq!(resolved.source, EnvSource::EnvFallback);
        assert_eq!(resolved.cloud_url, FALLBACK_CLOUD_URL);
        assert_eq!(resolved.admin_bearer, "tp_x:acme:admin");
    }
}
