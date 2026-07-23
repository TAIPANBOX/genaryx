//! Admission-plane environment discovery: three INDEPENDENT pieces, each
//! resolved and reported honestly on its own - the gateway to verify against,
//! the `verdryx` binary to run an eval/baseline through it, and the
//! `verdryx.db` store to read the result back from.
//!
//! ## Honest per-piece resolution states
//!
//! Unlike `crate::drills::env` (which gates its ONE `discover()` on BOTH the
//! mockryx binary AND the gateway resolving together, since Drills has
//! exactly one function that needs both at once), this plane's three pieces
//! are surfaced independently: `admission_status` (`super::commands`) reports
//! the gateway's own Bootstrapping/NoEnvironment/Unreachable/Ready state
//! (`super::state`) ALONGSIDE the verdryx binary's presence and the verdryx db's
//! resolution, never collapsing them into one combined gate. This is
//! deliberate: `admission_check` (viewer) only ever needs the gateway leg,
//! while `admission_baseline` (admin) additionally needs the verdryx
//! binary+db - conflating all three into a single `Option<ResolvedEnv>` would
//! make "gateway is up but verdryx is not installed yet" indistinguishable
//! from "nothing at all is configured," which is exactly the kind of
//! conflation `crate::quality::env`'s own module doc warns against for its
//! one piece ("blur[ring] 'no quality plane at all'... with 'found a plane
//! but couldn't open it'").
//!
//! ## The gateway
//!
//! Resolved EXACTLY like `crate::credentials::env` does: the SAME
//! `services.gateway.url` `crate::drills::env`/`crate::money::env` read off a
//! `taipan up` descriptor - explicitly NOT `services.cloud`. No key, no auth
//! (see `genaryx_connectors::gateway`'s module doc). Deliberately duplicated
//! rather than shared (this plane is its own independent one, same
//! "parallel, not coupled" convention `identity::env`'s module doc states for
//! why IT duplicates rather than imports `policy::env`/`money::env`) - a
//! descriptor with no gateway service (or none found at all) resolves to
//! `None`, a normal, renderable "no gateway leg" state, never an error.
//!
//! ## The verdryx binary
//!
//! Default `$TAIPAN_HOME/bin/verdryx`, `$TAIPAN_HOME` defaulting to
//! `~/.taipan` - mirrors `crate::drills::env`'s well-known-location tier for
//! `mockryx` (`~/.taipan/bin/mockryx`), but with only that ONE tier, not
//! `drills::env`'s second "local checkout build" tier: verdryx (grounded in
//! `~/Development/verdryx/pyproject.toml`, read 2026-07-23) ships as a
//! `[project.scripts] verdryx = "verdryx.cli:main"` `pip`-installed console
//! script, not a compiled binary a checkout's own `Makefile` drops into a
//! `bin/` directory the way mockryx's Go build does - there is no
//! `~/Development/verdryx/bin/verdryx` to also look for. This also matches
//! verdryx's OWN resolution convention for its sibling `VERDRYX_DB`/db-path
//! logic (`verdryx/config.py::default_taipan_home`: `$TAIPAN_HOME`, else
//! `~/.taipan`) - same env var, same default, so an operator who has already
//! pointed `TAIPAN_HOME` at a scratch install for one gets the other moved
//! along with it for free.
//!
//! Always names the one candidate path it would look at (never a bare
//! `Option` that hides where the console looked) alongside an honest
//! `exists` bool - there is only one place to check, so there is no
//! "which of several candidates matched" ambiguity a `None` would need to
//! stand in for the way `resolve_verdryx_db` below has.
//!
//! ## The verdryx db
//!
//! Resolved EXACTLY like `crate::quality::env` resolves `verdryx.db`: a
//! `services.verdryx` descriptor entry (read as a filesystem path, forward-
//! compatible plumbing - today's `taipan up` never actually populates this,
//! see `quality::env`'s own module doc), else the well-known
//! `~/.taipan/verdryx.db`, else `None` for a clean "no verdryx store found"
//! state. Deliberately duplicated rather than imported (same rationale as
//! the gateway section above); `admission_baseline` needs both a place to
//! WRITE the eval/baseline through the `verdryx` CLI and a place to READ the
//! result back through `genaryx_connectors::VerdryxClient`, so unlike
//! Quality's read-only plane this path is not optional the way Quality's
//! whole panel is - a missing db here is a real refusal for that one command,
//! reported honestly by `super::commands::AdmissionError::VerdryxDbMissing`.
//!
//! ## The drills scenario dir (surfaced, not owned)
//!
//! `admission_status` also reports whether `crate::drills::env`'s own
//! well-known scenario directory (`~/Development/mockryx/scenarios`) exists,
//! since the UI's "Run drill as this key" action reuses
//! `crate::drills::commands::drills_run` unmodified and the operator
//! benefits from knowing up front whether that leg has somewhere to run from.
//! [`drills_scenario_dir_default`] is a faithful, private copy of
//! `drills::env`'s own private `discover_scenario_dir` (a two-line
//! well-known-path check) - not worth a `pub(crate)` bump to share for one
//! boolean presence check; see `drills::env`'s own module doc for the full
//! rationale behind that location.
//!
//! Never touches the network and never panics: every filesystem/JSON step
//! anywhere in this module is a `?`-chained `Option`, so one malformed or
//! half-written descriptor falls through to the next candidate instead of
//! taking down discovery - same discipline every sibling `env.rs` keeps.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ============================================================================
// The gateway (mirrors `credentials::env` exactly)
// ============================================================================

/// Where a gateway [`ResolvedEnv`] came from, surfaced to the UI. A single
/// variant, mirroring `credentials::env::EnvSource`'s identical rationale:
/// the gateway read needs no key, so there is no env-fallback counterpart to
/// resolve a hand-started gateway from - only a discovered `taipan up`
/// descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum EnvSource {
    /// Discovered from `~/.taipan/environments/<name>.json`.
    Taipan { name: String },
}

/// A fully-resolved place to verify admission against.
#[derive(Debug, Clone)]
pub struct ResolvedEnv {
    pub source: EnvSource,
    pub gateway_url: String,
}

// ---- descriptor wire shape (read-only mirror, deliberately duplicated - see
// this module's doc comment) ----

#[derive(Debug, Deserialize)]
struct DescriptorService {
    url: String,
}

#[derive(Debug, Deserialize)]
struct Descriptor {
    name: String,
    services: BTreeMap<String, DescriptorService>,
}

/// Resolve the gateway leg: the newest `taipan up` descriptor with a usable
/// `services.gateway` entry, or `None` for a clean "no gateway leg" state.
#[must_use]
pub fn discover_gateway() -> Option<ResolvedEnv> {
    let dir = genaryx_core::taipan_home::environments_dir()?;
    discover_taipan_in(&dir)
}

fn discover_taipan_in(environments_dir: &Path) -> Option<ResolvedEnv> {
    let mut candidates = list_descriptor_paths(environments_dir);
    candidates.sort_by_key(|p| std::cmp::Reverse(modified_time(p)));
    candidates.into_iter().find_map(|p| try_load_descriptor(&p))
}

/// Every `<name>.json` descriptor in `dir`, excluding the sibling
/// `<name>.keys.json` / `<name>.pid.json` files - identical filter to every
/// sibling `env.rs`.
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

// ============================================================================
// The verdryx binary
// ============================================================================

/// A resolved candidate for the `verdryx` binary: always names the one place
/// this plane looks, plus an honest `exists` bool - see this module's doc
/// comment for why there is only one tier here (unlike `drills::env`'s two
/// for `mockryx`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerdryxBinResolution {
    pub path: PathBuf,
    pub exists: bool,
}

/// `$TAIPAN_HOME/bin/verdryx`, `$TAIPAN_HOME` defaulting to `~/.taipan` (or a
/// bare relative `.taipan` when even `$HOME` is unset, matching
/// `genaryx_core::taipan_home`'s own "never panic over a missing env var"
/// discipline). Never `None`: the candidate path is always named, so a
/// "not found" UI can tell the operator exactly where to put it.
#[must_use]
pub fn resolve_verdryx_bin() -> VerdryxBinResolution {
    let path = taipan_home().join("bin").join("verdryx");
    let exists = path.is_file();
    VerdryxBinResolution { path, exists }
}

/// `$TAIPAN_HOME`, else `$HOME/.taipan`, else a bare `.taipan` - mirrors
/// verdryx's own `config.py::default_taipan_home` (env `TAIPAN_HOME`, else
/// `~/.taipan`) precisely, see this module's doc comment.
fn taipan_home() -> PathBuf {
    if let Some(home) = std::env::var_os("TAIPAN_HOME") {
        return PathBuf::from(home);
    }
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(".taipan"),
        None => PathBuf::from(".taipan"),
    }
}

// ============================================================================
// The verdryx db (mirrors `quality::env` exactly)
// ============================================================================

/// Where a [`VerdryxDbResolution`] came from, surfaced to the UI - mirrors
/// `quality::env::EnvSource` exactly (same two tiers, same names).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum VerdryxDbSource {
    /// A `services.verdryx.url` entry on a `taipan up` descriptor - forward-
    /// compatible plumbing, not exercised by today's taipan (see this
    /// module's doc comment).
    Taipan { name: String },
    /// No descriptor entry; the fixed `~/.taipan/verdryx.db` location existed
    /// as a file.
    WellKnown,
}

#[derive(Debug, Clone)]
pub struct VerdryxDbResolution {
    pub source: VerdryxDbSource,
    pub db_path: PathBuf,
}

/// Resolve the verdryx db leg: a `services.verdryx` descriptor entry, else
/// the well-known fixed location, else `None` for a clean "no verdryx store
/// found" state - byte-for-byte the same resolution `quality::env::discover`
/// performs, duplicated per this module's doc comment.
#[must_use]
pub fn resolve_verdryx_db() -> Option<VerdryxDbResolution> {
    if let Some(dir) = genaryx_core::taipan_home::environments_dir()
        && let Some(env) = resolve_verdryx_db_taipan_in(&dir)
    {
        return Some(env);
    }
    resolve_verdryx_db_well_known()
}

fn resolve_verdryx_db_taipan_in(environments_dir: &Path) -> Option<VerdryxDbResolution> {
    let mut candidates = list_descriptor_paths(environments_dir);
    candidates.sort_by_key(|p| std::cmp::Reverse(modified_time(p)));
    candidates
        .into_iter()
        .find_map(|p| try_load_verdryx_db_descriptor(&p))
}

fn try_load_verdryx_db_descriptor(path: &Path) -> Option<VerdryxDbResolution> {
    let bytes = std::fs::read(path).ok()?;
    let descriptor: Descriptor = serde_json::from_slice(&bytes).ok()?;
    let raw = descriptor.services.get("verdryx")?.url.clone();
    let candidate = PathBuf::from(raw);
    if !candidate.is_file() {
        return None;
    }
    Some(VerdryxDbResolution {
        source: VerdryxDbSource::Taipan {
            name: descriptor.name,
        },
        db_path: candidate,
    })
}

fn well_known_verdryx_db_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".taipan").join("verdryx.db"))
}

fn resolve_verdryx_db_well_known() -> Option<VerdryxDbResolution> {
    let candidate = well_known_verdryx_db_path()?;
    candidate.is_file().then_some(VerdryxDbResolution {
        source: VerdryxDbSource::WellKnown,
        db_path: candidate,
    })
}

// ============================================================================
// The drills scenario dir (surfaced only - see this module's doc comment)
// ============================================================================

/// Whether `crate::drills::env`'s own well-known scenario directory
/// (`~/Development/mockryx/scenarios`) exists right now. A faithful,
/// private copy of `drills::env`'s private `discover_scenario_dir` - see
/// this module's doc comment for why this is duplicated rather than shared.
#[must_use]
pub fn drills_scenario_dir_default() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let candidate = PathBuf::from(home)
        .join("Development")
        .join("mockryx")
        .join("scenarios");
    candidate.is_dir().then_some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "genaryx-admission-env-test-{tag}-{}-{n}",
            std::process::id()
        ))
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(path, body).expect("write fixture file");
    }

    // ---- gateway ----

    #[test]
    fn empty_directory_yields_no_gateway_candidate() {
        let dir = unique_dir("empty");
        std::fs::create_dir_all(&dir).expect("create dir");
        assert!(discover_taipan_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_directory_yields_no_gateway_candidate_not_a_panic() {
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
    fn resolves_a_real_shaped_gateway_descriptor() {
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
    fn newest_gateway_descriptor_wins_when_multiple_environments_exist() {
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

    #[test]
    fn discover_gateway_never_panics() {
        let _ = discover_gateway();
    }

    // ---- verdryx binary ----

    #[test]
    fn resolve_verdryx_bin_never_panics() {
        let _ = resolve_verdryx_bin();
    }

    #[test]
    fn taipan_home_honours_the_env_var_when_set() {
        // Cannot safely mutate process-wide env vars in a parallel test
        // suite, so this only proves the join shape relative to whatever
        // TAIPAN_HOME/HOME already is - mirrors `quality::env`'s identical
        // "prove the shape, not a mocked env" discipline for a HOME-dependent
        // best-effort path.
        let resolution = resolve_verdryx_bin();
        assert!(
            resolution.path.ends_with("bin/verdryx") || resolution.path.ends_with("bin\\verdryx")
        );
    }

    // ---- verdryx db ----

    #[test]
    fn empty_directory_yields_no_verdryx_db_candidate() {
        let dir = unique_dir("db-empty");
        std::fs::create_dir_all(&dir).expect("create dir");
        assert!(resolve_verdryx_db_taipan_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_verdryx_service_entry_pointing_at_a_missing_file_falls_through() {
        let dir = unique_dir("db-missing-file");
        write(
            &dir.join("p1full.json"),
            r#"{"name":"p1full","services":{"verdryx":{"url":"/nonexistent/genaryx-admission-test/verdryx.db"}}}"#,
        );
        assert!(resolve_verdryx_db_taipan_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_a_verdryx_db_service_entry_whose_path_exists() {
        let dir = unique_dir("db-service");
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

        let resolved =
            resolve_verdryx_db_taipan_in(&dir).expect("must resolve the fixture descriptor");
        assert_eq!(
            resolved.source,
            VerdryxDbSource::Taipan {
                name: "p1full".to_string()
            }
        );
        assert_eq!(resolved.db_path, db_path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn well_known_verdryx_db_path_ends_with_the_expected_relative_shape() {
        if let Some(p) = well_known_verdryx_db_path() {
            assert!(p.ends_with("verdryx.db"));
            assert!(p.to_string_lossy().contains(".taipan"));
        }
    }

    #[test]
    fn resolve_verdryx_db_never_panics() {
        let _ = resolve_verdryx_db();
    }

    // ---- drills scenario dir ----

    #[test]
    fn drills_scenario_dir_default_never_panics() {
        let _ = drills_scenario_dir_default();
    }
}
