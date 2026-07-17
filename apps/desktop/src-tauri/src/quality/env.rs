//! Quality-panel environment discovery: where to find `verdryx.db`.
//!
//! Verdryx has no `serve` process and no bearer key at all (docs/PHASE4.md:
//! "Verdryx has NO JSON/machine output on any subcommand... its durable,
//! machine-readable surface is its SQLite store"), so unlike
//! `identity::env`/`policy::env`/`money::env` there is no URL and no admin
//! key to resolve here - only a filesystem path to a SQLite file. Two tiers,
//! tried in order, neither an error when it comes up empty:
//!
//! 1. The SAME `taipan up` descriptor `identity::env` reads
//!    (`~/.taipan/environments/<name>.json`), checked for a `services.verdryx`
//!    entry. Read directly against `~/Development/taipan/src/descriptor.rs`
//!    as ground truth (2026-07-17): today's descriptor schema never actually
//!    populates this entry (verdryx is a batch CLI, not a `taipan up`
//!    managed service), so this tier is forward-compatible rather than
//!    exercised in practice right now - a future taipan could plausibly set
//!    `services.verdryx.url` to the eval store's path (`ServiceEntry.url` is
//!    a bare `String`, not restricted to `http(s)://`, so reusing it for a
//!    filesystem path costs the descriptor schema nothing). Picked up here
//!    for free the day it exists; harmless dead weight until then.
//! 2. A well-known fixed location, `~/.taipan/verdryx.db` - the same
//!    "predictable, fixed spot under the taipan home" convention
//!    `identity::state::resolve_idryx_bin` uses for `~/.taipan/bin/idryx`,
//!    applied to a data file instead of a binary. An operator who wants the
//!    console to auto-discover their `verdryx.db` can park it there (or
//!    symlink it), same as they would `idryx`/`qryx` into `~/.taipan/bin/`.
//!
//! Either tier requires the candidate path to actually exist as a file
//! before [`discover`] reports it - a resolved-but-nonexistent path would
//! blur "no quality plane at all" (this module's job) with "found a plane
//! but couldn't open it" (`super::state::bootstrap`'s job, via
//! `genaryx_connectors::VerdryxError::Open`). This module never touches the
//! database itself and never panics: every filesystem/JSON step is a
//! `?`-chained `Option`, so one malformed descriptor falls through to the
//! well-known-location tier instead of taking down discovery.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where a [`ResolvedEnv`] came from, surfaced to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum EnvSource {
    /// A `services.verdryx.url` entry on `~/.taipan/environments/<name>.json`
    /// (see this module's doc comment - forward-compatible, not exercised by
    /// today's taipan).
    Taipan { name: String },
    /// No descriptor entry; the fixed `~/.taipan/verdryx.db` location existed
    /// as a file.
    WellKnown,
}

/// A fully-resolved place to read Verdryx's quality plane from.
#[derive(Debug, Clone)]
pub struct ResolvedEnv {
    pub source: EnvSource,
    pub db_path: PathBuf,
}

// ---- descriptor wire shape (read-only mirror, see identity::env) ----------

#[derive(Debug, Deserialize)]
struct DescriptorService {
    url: String,
}

#[derive(Debug, Deserialize)]
struct Descriptor {
    name: String,
    services: BTreeMap<String, DescriptorService>,
}

/// Resolve the Quality panel's environment: a `services.verdryx` descriptor
/// entry, else the well-known fixed location, else `None` for a clean "no
/// quality plane" state.
#[must_use]
pub fn discover() -> Option<ResolvedEnv> {
    if let Some(dir) = taipan_environments_dir()
        && let Some(env) = discover_taipan_in(&dir)
    {
        return Some(env);
    }
    discover_well_known()
}

/// `~/.taipan/environments`, or `None` when `$HOME` is not set - mirrors
/// `identity::env::taipan_environments_dir` exactly (same directory; every
/// panel discovers from the same `taipan up` output).
fn taipan_environments_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".taipan").join("environments"))
}

/// Testable core of the descriptor path: scan `environments_dir` for
/// descriptor files (newest last-modified first), and return the first one
/// that yields a `services.verdryx` entry pointing at a file that actually
/// exists.
fn discover_taipan_in(environments_dir: &Path) -> Option<ResolvedEnv> {
    let mut candidates = list_descriptor_paths(environments_dir);
    candidates.sort_by_key(|p| std::cmp::Reverse(modified_time(p)));
    candidates.into_iter().find_map(|p| try_load_descriptor(&p))
}

/// Every `<name>.json` descriptor in `dir`, excluding the sibling
/// `<name>.keys.json` / `<name>.pid.json` files - identical filter to
/// `identity::env::list_descriptor_paths`.
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

/// Load and resolve one descriptor: read `services.verdryx.url` and treat it
/// as a filesystem path (see this module's doc comment). `None` at any
/// step - including "the path is not an existing file" - falls through to
/// the next candidate rather than erroring.
fn try_load_descriptor(path: &Path) -> Option<ResolvedEnv> {
    let bytes = std::fs::read(path).ok()?;
    let descriptor: Descriptor = serde_json::from_slice(&bytes).ok()?;
    let raw = descriptor.services.get("verdryx")?.url.clone();
    let candidate = PathBuf::from(raw);
    if !candidate.is_file() {
        return None;
    }
    Some(ResolvedEnv {
        source: EnvSource::Taipan {
            name: descriptor.name,
        },
        db_path: candidate,
    })
}

/// `~/.taipan/verdryx.db`, best-effort - the well-known fallback tier (see
/// this module's doc comment). `None` when `$HOME` is unset or no file
/// exists there.
fn well_known_db_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".taipan").join("verdryx.db"))
}

fn discover_well_known() -> Option<ResolvedEnv> {
    let candidate = well_known_db_path()?;
    candidate.is_file().then_some(ResolvedEnv {
        source: EnvSource::WellKnown,
        db_path: candidate,
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
            "genaryx-quality-env-test-{tag}-{}-{n}",
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
    fn a_descriptor_with_no_verdryx_service_falls_through() {
        // The common case today (see this module's doc comment): no live
        // taipan ever populates `services.verdryx` yet.
        let dir = unique_dir("no-verdryx");
        write(
            &dir.join("plain.json"),
            r#"{"name":"plain","services":{"cloud":{"url":"http://x"}}}"#,
        );
        assert!(discover_taipan_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_verdryx_service_entry_pointing_at_a_missing_file_falls_through() {
        let dir = unique_dir("verdryx-missing-file");
        write(
            &dir.join("p1full.json"),
            r#"{"name":"p1full","services":{"verdryx":{"url":"/nonexistent/genaryx-quality-test/verdryx.db"}}}"#,
        );
        assert!(discover_taipan_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_a_verdryx_service_entry_whose_path_exists() {
        let dir = unique_dir("verdryx-service");
        std::fs::create_dir_all(&dir).expect("create dir");
        let db_path = dir.join("verdryx.db");
        std::fs::write(&db_path, b"not a real sqlite file, just needs to exist")
            .expect("write fixture db");
        write(
            &dir.join("p1full.json"),
            &format!(
                r#"{{"name":"p1full","services":{{"verdryx":{{"url":"{}"}}}}}}"#,
                db_path.display()
            ),
        );

        let resolved = discover_taipan_in(&dir).expect("must resolve the fixture descriptor");
        assert_eq!(
            resolved.source,
            EnvSource::Taipan {
                name: "p1full".to_string()
            }
        );
        assert_eq!(resolved.db_path, db_path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn newest_descriptor_wins_when_multiple_environments_exist() {
        let dir = unique_dir("multi");
        std::fs::create_dir_all(&dir).expect("create dir");
        let older_db = dir.join("older.db");
        let newer_db = dir.join("newer.db");
        std::fs::write(&older_db, b"x").expect("write older db");
        std::fs::write(&newer_db, b"x").expect("write newer db");

        write(
            &dir.join("older.json"),
            &format!(
                r#"{{"name":"older","services":{{"verdryx":{{"url":"{}"}}}}}}"#,
                older_db.display()
            ),
        );
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(
            &dir.join("newer.json"),
            &format!(
                r#"{{"name":"newer","services":{{"verdryx":{{"url":"{}"}}}}}}"#,
                newer_db.display()
            ),
        );

        let resolved = discover_taipan_in(&dir).expect("must resolve one of the two");
        assert_eq!(
            resolved.source,
            EnvSource::Taipan {
                name: "newer".to_string()
            }
        );
        assert_eq!(resolved.db_path, newer_db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn well_known_db_path_ends_with_the_expected_relative_shape() {
        // Can't safely mutate $HOME for a parallel test suite, so this only
        // proves the join shape relative to whatever $HOME already is -
        // mirrors `identity::state::resolve_idryx_bin_never_panics`'s
        // identical rationale for a HOME-dependent, best-effort path.
        if let Some(p) = well_known_db_path() {
            assert!(p.ends_with("verdryx.db"));
            assert!(p.to_string_lossy().contains(".taipan"));
        }
    }

    #[test]
    fn discover_well_known_never_panics() {
        let _ = discover_well_known();
    }

    #[test]
    fn discover_never_panics() {
        let _ = discover();
    }
}
