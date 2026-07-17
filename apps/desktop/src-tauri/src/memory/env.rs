//! Memory-panel environment discovery: the `engram-mcp` binary and the
//! engram SQLite store path.
//!
//! Unlike every other panel's `env` module, BOTH pieces gate readiness
//! together here (see [`discover`]): Memory has no partial-functionality
//! shape the way Identity does (idryx's three reads work with no `idryx`
//! binary at all - only Rescan needs one). The whole Memory panel IS one
//! long-lived `engram-mcp` process reading one store (docs/PHASE4.md W2's
//! "CRITICAL" note), so "no binary" and "no db" both simply mean "no memory
//! plane" - not two independently-gated concerns the way Identity's
//! optional Rescan binary is.
//!
//! ## The binary: a Python console script, no fixed install path yet
//! Tried in order, first hit wins:
//! 1. `~/.taipan/bin/engram-mcp` - the SAME well-known convention
//!    `crypto::env`/`identity::state` use for `qryx`/`idryx`, in case an
//!    operator symlinks it there.
//! 2. Resolved off `$PATH` (a pip/pipx-installed console script commonly
//!    ends up on `PATH` directly - the normal way a Python CLI is installed
//!    outside a project-local venv).
//! 3. `~/Development/engram/.venv/bin/engram-mcp` - a local checkout's own
//!    virtualenv. docs/PHASE4.md grounds Engram from `~/Development/engram`
//!    (`engdbram` on PyPI, no documented global-install convention this
//!    console can assume), so a dev checkout's venv is the realistic
//!    fallback default.
//!
//! ## The store: `.engram` SQLite, same taipan-descriptor-then-well-known
//! shape as `quality::env`/`identity::env`:
//! 1. `services.engram.url` on the SAME `~/.taipan/environments/<name>.json`
//!    descriptor identity/quality/money read (forward-compatible - today's
//!    taipan never populates this entry, exactly like `quality::env`'s
//!    `services.verdryx` tier), reused as a bare filesystem path
//!    (`ServiceEntry.url` is just a `String`, not restricted to `http(s)://`).
//! 2. `~/.taipan/.engram` - the well-known fixed location, mirroring
//!    `quality::env`'s `~/.taipan/verdryx.db`, using engram's own canonical
//!    dotfile name (docs/PHASE4.md: "the `.engram` SQLite path").
//!
//! Either tier requires the candidate to resolve to a real file that
//! already exists - deliberately never `:memory:` and never a
//! not-yet-created path: a store nothing has ever written to means no agent
//! has actually used this Engram, and spawning a whole `engram-mcp` process
//! (with its lazy embedding-model load) against it would burn real
//! resources just to show an empty panel. This mirrors `quality::env::discover`'s
//! identical "must already exist" requirement, for the identical reason.
//!
//! Never panics: every filesystem/JSON step is a `?`-chained `Option`, so one
//! malformed descriptor or absent tier falls through to the next rather than
//! taking down discovery.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where a [`ResolvedEnv`]'s `db_path` came from, surfaced to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum EnvSource {
    /// A `services.engram.url` entry on `~/.taipan/environments/<name>.json`
    /// (forward-compatible - see this module's doc comment).
    Taipan { name: String },
    /// No descriptor entry; the fixed `~/.taipan/.engram` location existed
    /// as a file.
    WellKnown,
}

/// A fully-resolved place to run the Memory panel from: the `engram-mcp`
/// binary plus the store it should open.
#[derive(Debug, Clone)]
pub struct ResolvedEnv {
    pub source: EnvSource,
    pub engram_mcp_bin: PathBuf,
    pub db_path: PathBuf,
}

// ---- descriptor wire shape (read-only mirror, see quality::env) -----------

#[derive(Debug, Deserialize)]
struct DescriptorService {
    url: String,
}

#[derive(Debug, Deserialize)]
struct Descriptor {
    name: String,
    services: BTreeMap<String, DescriptorService>,
}

/// Resolve the Memory panel's environment: BOTH the `engram-mcp` binary and
/// a real, already-existing `.engram` store must resolve, or this is `None`
/// for a clean "no memory plane" state - see this module's doc comment for
/// why the two are not independently gated the way Identity's binary/URL
/// are.
#[must_use]
pub fn discover() -> Option<ResolvedEnv> {
    let engram_mcp_bin = discover_bin()?;
    let (source, db_path) = discover_db()?;
    Some(ResolvedEnv {
        source,
        engram_mcp_bin,
        db_path,
    })
}

// ---- binary discovery -------------------------------------------------

fn discover_bin() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);

    if let Some(h) = &home {
        let well_known = well_known_bin_path(h);
        if well_known.is_file() {
            return Some(well_known);
        }
    }

    if let Some(path_var) = std::env::var_os("PATH")
        && let Some(found) = find_on_path("engram-mcp", std::env::split_paths(&path_var))
    {
        return Some(found);
    }

    if let Some(h) = &home {
        let venv = venv_bin_path(h);
        if venv.is_file() {
            return Some(venv);
        }
    }

    None
}

fn well_known_bin_path(home: &Path) -> PathBuf {
    home.join(".taipan").join("bin").join("engram-mcp")
}

fn venv_bin_path(home: &Path) -> PathBuf {
    home.join("Development")
        .join("engram")
        .join(".venv")
        .join("bin")
        .join("engram-mcp")
}

/// A dependency-free `$PATH` scan for `name` - no `which` crate pulled in
/// just for this one lookup (this crate's `Cargo.toml` sanctions only
/// `genaryx-connectors`/`genaryx-signing` as new product dependencies). Takes
/// the directory list directly (rather than reading `$PATH` itself) so tests
/// never have to mutate real process environment - mirrors every other
/// `env.rs` module's "take the already-read value directly" testability
/// convention.
fn find_on_path(name: &str, mut dirs: impl Iterator<Item = PathBuf>) -> Option<PathBuf> {
    dirs.find_map(|dir| {
        let candidate = dir.join(name);
        candidate.is_file().then_some(candidate)
    })
}

// ---- store discovery ----------------------------------------------------

fn discover_db() -> Option<(EnvSource, PathBuf)> {
    if let Some(dir) = taipan_environments_dir()
        && let Some(found) = discover_taipan_db_in(&dir)
    {
        return Some(found);
    }
    discover_well_known_db()
}

/// `~/.taipan/environments`, or `None` when `$HOME` is not set - mirrors
/// `quality::env::taipan_environments_dir` exactly (same directory; every
/// panel discovers from the same `taipan up` output).
fn taipan_environments_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".taipan").join("environments"))
}

/// Testable core of the descriptor path: scan `environments_dir` for
/// descriptor files (newest last-modified first), and return the first one
/// that yields a `services.engram` entry pointing at a real, existing,
/// non-`:memory:` file.
fn discover_taipan_db_in(environments_dir: &Path) -> Option<(EnvSource, PathBuf)> {
    let mut candidates = list_descriptor_paths(environments_dir);
    candidates.sort_by_key(|p| std::cmp::Reverse(modified_time(p)));
    candidates.into_iter().find_map(|p| try_load_descriptor(&p))
}

/// Every `<name>.json` descriptor in `dir`, excluding the sibling
/// `<name>.keys.json` / `<name>.pid.json` files - identical filter to
/// `quality::env::list_descriptor_paths`.
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

/// Load and resolve one descriptor: read `services.engram.url` and treat it
/// as a filesystem path (see this module's doc comment). `None` at any
/// step - including "the path is not a real, existing, non-`:memory:` file" -
/// falls through to the next candidate rather than erroring.
fn try_load_descriptor(path: &Path) -> Option<(EnvSource, PathBuf)> {
    let bytes = std::fs::read(path).ok()?;
    let descriptor: Descriptor = serde_json::from_slice(&bytes).ok()?;
    let raw = descriptor.services.get("engram")?.url.clone();
    let candidate = valid_store_path(&raw)?;
    Some((
        EnvSource::Taipan {
            name: descriptor.name,
        },
        candidate,
    ))
}

/// `~/.taipan/.engram`, best-effort - the well-known fallback tier (see this
/// module's doc comment).
fn well_known_db_path(home: &Path) -> PathBuf {
    home.join(".taipan").join(".engram")
}

fn discover_well_known_db() -> Option<(EnvSource, PathBuf)> {
    let home = std::env::var_os("HOME")?;
    let candidate = well_known_db_path(&PathBuf::from(home));
    valid_store_path(candidate.to_str()?).map(|p| (EnvSource::WellKnown, p))
}

/// A real, already-existing, non-`:memory:` store path - see this module's
/// doc comment. Centralized so both tiers reject the same way, including the
/// (redundant with `is_file()` in practice, but explicit and
/// self-documenting) `:memory:` special case.
fn valid_store_path(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == ":memory:" {
        return None;
    }
    let candidate = PathBuf::from(trimmed);
    candidate.is_file().then_some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "genaryx-memory-env-test-{tag}-{}-{n}",
            std::process::id()
        ))
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(path, body).expect("write fixture file");
    }

    // ---- valid_store_path ----

    #[test]
    fn valid_store_path_rejects_memory_and_blank() {
        assert!(valid_store_path(":memory:").is_none());
        assert!(valid_store_path("").is_none());
        assert!(valid_store_path("   ").is_none());
    }

    #[test]
    fn valid_store_path_rejects_a_nonexistent_file() {
        assert!(valid_store_path("/nonexistent/genaryx-memory-test/.engram").is_none());
    }

    #[test]
    fn valid_store_path_accepts_a_real_existing_file() {
        let dir = unique_dir("valid-store");
        let db_path = dir.join(".engram");
        write(&db_path, "not a real sqlite file, just needs to exist");
        assert_eq!(valid_store_path(&db_path.to_string_lossy()), Some(db_path));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- descriptor tier ----

    #[test]
    fn empty_directory_yields_no_candidate() {
        let dir = unique_dir("empty");
        std::fs::create_dir_all(&dir).expect("create dir");
        assert!(discover_taipan_db_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_directory_yields_no_candidate_not_a_panic() {
        let dir = unique_dir("missing").join("nested").join("deeper");
        assert!(discover_taipan_db_in(&dir).is_none());
    }

    #[test]
    fn a_descriptor_with_no_engram_service_falls_through() {
        let dir = unique_dir("no-engram");
        write(
            &dir.join("plain.json"),
            r#"{"name":"plain","services":{"cloud":{"url":"http://x"}}}"#,
        );
        assert!(discover_taipan_db_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_engram_service_entry_pointing_at_a_missing_file_falls_through() {
        let dir = unique_dir("engram-missing-file");
        write(
            &dir.join("p1full.json"),
            r#"{"name":"p1full","services":{"engram":{"url":"/nonexistent/genaryx-memory-test/.engram"}}}"#,
        );
        assert!(discover_taipan_db_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_engram_service_entry_of_memory_falls_through() {
        let dir = unique_dir("engram-memory");
        write(
            &dir.join("p1full.json"),
            r#"{"name":"p1full","services":{"engram":{"url":":memory:"}}}"#,
        );
        assert!(discover_taipan_db_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_an_engram_service_entry_whose_path_exists() {
        let dir = unique_dir("engram-service");
        std::fs::create_dir_all(&dir).expect("create dir");
        let db_path = dir.join(".engram");
        std::fs::write(&db_path, b"not a real sqlite file, just needs to exist")
            .expect("write fixture db");
        write(
            &dir.join("p1full.json"),
            &format!(
                r#"{{"name":"p1full","services":{{"engram":{{"url":"{}"}}}}}}"#,
                db_path.display()
            ),
        );

        let (source, resolved) =
            discover_taipan_db_in(&dir).expect("must resolve the fixture descriptor");
        assert_eq!(
            source,
            EnvSource::Taipan {
                name: "p1full".to_string()
            }
        );
        assert_eq!(resolved, db_path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn newest_descriptor_wins_when_multiple_environments_exist() {
        let dir = unique_dir("multi");
        std::fs::create_dir_all(&dir).expect("create dir");
        let older_db = dir.join("older.engram");
        let newer_db = dir.join("newer.engram");
        std::fs::write(&older_db, b"x").expect("write older db");
        std::fs::write(&newer_db, b"x").expect("write newer db");

        write(
            &dir.join("older.json"),
            &format!(
                r#"{{"name":"older","services":{{"engram":{{"url":"{}"}}}}}}"#,
                older_db.display()
            ),
        );
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(
            &dir.join("newer.json"),
            &format!(
                r#"{{"name":"newer","services":{{"engram":{{"url":"{}"}}}}}}"#,
                newer_db.display()
            ),
        );

        let (source, resolved) = discover_taipan_db_in(&dir).expect("must resolve one of the two");
        assert_eq!(
            source,
            EnvSource::Taipan {
                name: "newer".to_string()
            }
        );
        assert_eq!(resolved, newer_db);
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
        assert!(discover_taipan_db_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- binary discovery ----

    #[test]
    fn well_known_bin_path_ends_with_the_expected_relative_shape() {
        let home = PathBuf::from("/home/op");
        let p = well_known_bin_path(&home);
        assert_eq!(p, PathBuf::from("/home/op/.taipan/bin/engram-mcp"));
    }

    #[test]
    fn venv_bin_path_ends_with_the_expected_relative_shape() {
        let home = PathBuf::from("/home/op");
        let p = venv_bin_path(&home);
        assert_eq!(
            p,
            PathBuf::from("/home/op/Development/engram/.venv/bin/engram-mcp")
        );
    }

    #[test]
    fn find_on_path_finds_an_existing_file_in_a_later_directory() {
        let empty_dir = unique_dir("path-empty");
        let hit_dir = unique_dir("path-hit");
        std::fs::create_dir_all(&empty_dir).expect("create empty dir");
        write(&hit_dir.join("engram-mcp"), "#!/bin/sh\n");

        let found = find_on_path(
            "engram-mcp",
            vec![empty_dir.clone(), hit_dir.clone()].into_iter(),
        );
        assert_eq!(found, Some(hit_dir.join("engram-mcp")));

        let _ = std::fs::remove_dir_all(&empty_dir);
        let _ = std::fs::remove_dir_all(&hit_dir);
    }

    #[test]
    fn find_on_path_returns_none_when_absent_everywhere() {
        let a = unique_dir("path-a");
        let b = unique_dir("path-b");
        std::fs::create_dir_all(&a).expect("create dir a");
        std::fs::create_dir_all(&b).expect("create dir b");

        assert!(find_on_path("engram-mcp", vec![a.clone(), b.clone()].into_iter()).is_none());

        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn discover_bin_never_panics() {
        // Best-effort, like every other HOME/PATH-dependent resolution in
        // this codebase: only proves this resolves to a consistent Option
        // without panicking, regardless of this box's actual local state.
        let _ = discover_bin();
    }

    #[test]
    fn discover_never_panics() {
        let _ = discover();
    }
}
