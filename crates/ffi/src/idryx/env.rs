//! Environment discovery for [`super::IdryxHandle`]: which Idryx identity
//! plane to talk to, plus (separately) what a **Rescan** needs to shell out
//! to `idryx detect`.
//!
//! Structurally this mirrors [`crate::wardryx::env`] (docs/PHASE3.md W2,
//! "PARITY across both shells"): a `taipan up` descriptor under
//! `~/.taipan/environments/<name>.json` tried first, an env-var fallback
//! second. One deliberate difference from that module, forced by Idryx
//! having **no authentication at all** (see `super`'s module doc): where
//! `wardryx::env`'s fallback is a fixed URL (`127.0.0.1:8090`) gated on a
//! *secret* (`WARDRYX_ADMIN_KEY` - the secret is the actual signal that the
//! operator means to use the fallback), Idryx has no secret to gate on, so
//! [`IDRYX_URL_ENV_VAR`] carries the **URL itself**. A fixed fallback port
//! would additionally be actively misleading here: idryx's own documented
//! default bind is `:8080` (docs/PHASE3.md: "`--addr` defaults to `:8080`...
//! `taipan up` remaps it to `127.0.0.1:8081`" specifically because `:8080`
//! collides with Cloud in this stack) - assuming that default for a
//! hand-started idryx would routinely guess wrong.
//!
//! This module is otherwise a deliberate, self-contained copy rather than a
//! shared import of `wardryx::env` (see `wardryx::mod`'s own module doc for
//! why: two crates-internal sibling modules choosing independent evolution
//! over a shared abstraction). Two entry points:
//!
//! - [`discover`]: resolves the Identity panel's connection - a `taipan up`
//!   descriptor's `services.idryx.url`, or [`IDRYX_URL_ENV_VAR`].
//! - [`resolve_rescan_inputs`]: resolves what the **Rescan** button needs -
//!   the idryx binary (`~/.taipan/bin/idryx`) plus `--load source:path`
//!   pairs built from a descriptor's `events.dir`/`events.files`. Resolved
//!   independently of (and every time, not cached alongside) [`discover`]'s
//!   own result - see [`resolve_rescan_inputs`]'s own doc for why.
//!
//! A descriptor whose `services` map simply has no `"idryx"` entry (a
//! `taipan up` run without `--with idryx`) is not an error: it falls through
//! exactly like a missing/unresolvable field does anywhere else in this
//! module, ultimately yielding [`discover`] `None` and
//! [`super::IdryxHandle::discover`] failing closed with
//! `IdryxError::NoEnvironment` - PHASE3.md's "No-idryx environment renders a
//! clean empty state, not an error". This module never touches the network.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Env var carrying a directly-known idryx base URL, for an idryx started by
/// hand (no `taipan up`) during local development - see the module doc for
/// why this carries the URL itself rather than a secret-gated fixed port.
const IDRYX_URL_ENV_VAR: &str = "IDRYX_URL";

/// The stack-bus sources idryx's `--load` flag accepts (docs/PHASE3.md:
/// "sources for the stack bus: tokenfuse|wardryx|mockryx|verdryx, routed
/// through tokenfuse.Load, main.go:208-213"). A descriptor's `events.files`
/// may carry other keys idryx's own `--load` vocabulary also understands
/// (`okta`, `aws_iam`, ...) that this console never writes to its bus itself,
/// so Rescan only ever forwards these four - in this fixed order, so
/// [`RescanInputs::loads`] is deterministic regardless of the descriptor's
/// own (alphabetical, `BTreeMap`) key order.
const RESCAN_SOURCES: [&str; 4] = ["tokenfuse", "wardryx", "mockryx", "verdryx"];

/// Where a [`ResolvedEnv`] came from, surfaced to the Swift shell (06 §0.5),
/// exported as a UniFFI enum - Swift sees `.taipan(name:)` / `.envFallback`.
/// Named distinctly from `crate::wardryx::env::WardryxEnvSource` /
/// `crate::cloud::env::EnvSource` (rather than reused) for the same reason
/// `wardryx::env::WardryxEnvSource`'s own doc comment gives: UniFFI's
/// generated namespace is flat per crate, so two same-named Rust types would
/// collide once both are exported to one generated `genaryx_ffi.swift`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum IdryxEnvSource {
    /// Discovered from `~/.taipan/environments/<name>.json`.
    Taipan { name: String },
    /// No usable descriptor found; [`IDRYX_URL_ENV_VAR`] instead (or a
    /// caller-supplied URL via [`super::IdryxHandle::connect`], which always
    /// reports this variant too - mirrors `WardryxEnvSource::EnvFallback`'s
    /// own dual use).
    EnvFallback,
}

/// A fully-resolved place to build a [`genaryx_connectors::IdryxClient`]
/// against. No bearer field at all (unlike `wardryx::env::ResolvedEnv`):
/// idryx has nothing to authenticate with.
#[derive(Debug, Clone)]
pub struct ResolvedEnv {
    pub source: IdryxEnvSource,
    pub idryx_url: String,
}

/// What [`super::IdryxHandle::rescan`] needs: the idryx binary, plus
/// `(source, absolute path)` pairs for every recognized stack-bus file the
/// current taipan environment knows about. See [`resolve_rescan_inputs`]'s
/// own doc for the freshness/caching rationale.
#[derive(Debug, Clone)]
pub struct RescanInputs {
    pub idryx_bin: PathBuf,
    pub loads: Vec<(String, String)>,
}

// ---- descriptor wire shapes (read-only mirror) -----------------------------
// Only the fields this module actually reads are modeled; `#[serde(default)]`
// / unknown-field tolerance throughout so a descriptor with extra fields
// never fails to parse (matches `wardryx::env`'s own convention).

#[derive(Debug, Deserialize)]
struct DescriptorService {
    url: String,
}

#[derive(Debug, Default, Deserialize)]
struct DescriptorEvents {
    #[serde(default)]
    dir: String,
    #[serde(default)]
    files: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct Descriptor {
    name: String,
    services: BTreeMap<String, DescriptorService>,
    #[serde(default)]
    events: DescriptorEvents,
}

/// Resolve the Identity panel's environment: prefer a `taipan up` descriptor,
/// fall back to [`IDRYX_URL_ENV_VAR`], or `None` for a clean "no environment"
/// state.
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

fn discover_taipan_in(environments_dir: &Path) -> Option<ResolvedEnv> {
    let descriptor = newest_descriptor_with_idryx(environments_dir)?;
    let idryx_url = descriptor.services.get("idryx")?.url.clone();
    Some(ResolvedEnv {
        source: IdryxEnvSource::Taipan {
            name: descriptor.name,
        },
        idryx_url,
    })
}

/// The most-recently-modified descriptor in `dir` whose `services` map has
/// an `"idryx"` entry, skipping any newer descriptor that lacks one (a
/// `taipan up` run without `--with idryx`) rather than stopping at the very
/// newest file regardless of content. Shared by [`discover_taipan_in`] (which
/// only needs the URL) and [`rescan_loads_in`] (which only needs
/// `events.*`), so both agree on exactly which environment "the current
/// stack" means - picking two different descriptors for those two purposes
/// would let Rescan recompute over a different environment's bus files than
/// the one the panel is actually showing identities/alerts from.
fn newest_descriptor_with_idryx(dir: &Path) -> Option<Descriptor> {
    descriptor_paths_newest_first(dir)
        .into_iter()
        .find_map(|p| {
            let bytes = std::fs::read(&p).ok()?;
            let descriptor: Descriptor = serde_json::from_slice(&bytes).ok()?;
            descriptor
                .services
                .contains_key("idryx")
                .then_some(descriptor)
        })
}

/// Every `<name>.json` descriptor in `dir`, newest last-modified first,
/// excluding the sibling `<name>.keys.json` / `<name>.pid.json` files. An
/// unreadable directory (not yet created) yields no candidates rather than an
/// error.
fn descriptor_paths_newest_first(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut candidates: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                return false;
            };
            name.ends_with(".json") && !name.ends_with(".keys.json") && !name.ends_with(".pid.json")
        })
        .collect();
    candidates.sort_by_key(|p| std::cmp::Reverse(modified_time(p)));
    candidates
}

fn modified_time(path: &Path) -> std::time::SystemTime {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
}

/// [`IDRYX_URL_ENV_VAR`], for an idryx started without `taipan up`. `None`
/// when the var is unset or blank.
fn discover_env_fallback() -> Option<ResolvedEnv> {
    env_fallback_from(std::env::var(IDRYX_URL_ENV_VAR).ok())
}

/// Testable core of [`discover_env_fallback`], taking the (already-read) env
/// var value directly so tests never have to mutate real process environment
/// (which `cargo test`'s parallel-by-default threads make inherently racy
/// across a shared process - matches `wardryx::env`'s own
/// `env_fallback_from` rationale).
fn env_fallback_from(idryx_url: Option<String>) -> Option<ResolvedEnv> {
    let idryx_url = idryx_url?;
    if idryx_url.trim().is_empty() {
        return None;
    }
    Some(ResolvedEnv {
        source: IdryxEnvSource::EnvFallback,
        idryx_url,
    })
}

// ---- Rescan inputs ----------------------------------------------------

/// `~/.taipan/bin/idryx` - where `taipan up --with idryx` places the binary
/// it built (`services::idryx::ensure_binary` in the `taipan` repo).
fn locate_idryx_binary() -> Option<PathBuf> {
    idryx_binary_under(&PathBuf::from(std::env::var_os("HOME")?))
}

/// Testable core of [`locate_idryx_binary`]: `home/.taipan/bin/idryx`, `None`
/// when nothing file-shaped exists there yet - never a panic over an absent
/// path.
fn idryx_binary_under(home: &Path) -> Option<PathBuf> {
    let path = home.join(".taipan").join("bin").join("idryx");
    path.is_file().then_some(path)
}

/// Best-effort: the current stack's idryx binary plus `--load` inputs for
/// [`super::IdryxHandle::rescan`]. Resolved **fresh on every call** (never
/// cached on the handle alongside [`ResolvedEnv`]), for two independent
/// reasons:
///
/// 1. A `taipan up --with idryx` that starts, or a bus file that grows,
///    AFTER the handle was constructed must still be picked up - the whole
///    point of Rescan is recomputing over the CURRENT stack, not a snapshot
///    of it taken at connect time.
/// 2. It is deliberately independent of *how* this handle's own URL was
///    resolved (`discover()` vs a caller-supplied `connect(idryx_url)`): an
///    operator who `connect()`-ed directly to a known URL may still have a
///    perfectly usable taipan environment sitting alongside it, and Rescan
///    should not become unavailable just because discovery was skipped.
///
/// Fails with a human-readable reason (never a panic, never a silently-empty
/// success - docs/PHASE3.md W2: "if the binary is not found, return an
/// honest IdryxError variant, never a fake empty success") naming exactly
/// which piece is missing.
pub fn resolve_rescan_inputs() -> Result<RescanInputs, String> {
    let idryx_bin = locate_idryx_binary()
        .ok_or_else(|| "idryx binary not found at ~/.taipan/bin/idryx".to_string())?;
    let dir = taipan_environments_dir().ok_or_else(|| "$HOME is not set".to_string())?;
    let loads = rescan_loads_in(&dir)?;
    Ok(RescanInputs { idryx_bin, loads })
}

/// Testable core of [`resolve_rescan_inputs`]'s descriptor half: the newest
/// `environments_dir` descriptor with an `idryx` service (see
/// [`newest_descriptor_with_idryx`]), projected to `(source, absolute path)`
/// pairs for [`RESCAN_SOURCES`]. An empty result (a descriptor exists but
/// names none of the four recognized sources) is `Ok(vec![])`, not an error:
/// idryx's own `detect` honestly refuses zero-input runs on its own
/// (`inputArg` in `cmd/idryx/main.go`: "provide exactly one input file,
/// --db, or --load source:path"), so that failure surfaces for real through
/// `IdryxClient::rescan` rather than being anticipated (and possibly
/// mis-anticipated) here.
fn rescan_loads_in(environments_dir: &Path) -> Result<Vec<(String, String)>, String> {
    let descriptor = newest_descriptor_with_idryx(environments_dir)
        .ok_or_else(|| "no taipan environment with an idryx service found".to_string())?;
    Ok(RESCAN_SOURCES
        .iter()
        .filter_map(|source| {
            descriptor.events.files.get(*source).map(|file| {
                let absolute = Path::new(&descriptor.events.dir).join(file);
                (source.to_string(), absolute.to_string_lossy().into_owned())
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "genaryx-ffi-idryx-env-test-{tag}-{}-{n}",
            std::process::id()
        ))
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(path, body).expect("write fixture file");
    }

    // ---- discover_taipan_in / newest_descriptor_with_idryx ----------------

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
    fn resolves_a_real_shaped_descriptor() {
        let dir = unique_dir("happy");
        write(
            &dir.join("p1full.json"),
            r#"{
                "name": "p1full",
                "created_at": "2026-07-17T00:00:00Z",
                "host": "box.local",
                "services": {
                    "gateway": {"url": "http://127.0.0.1:41000", "mode": "enforce"},
                    "idryx": {"url": "http://127.0.0.1:8081"}
                },
                "events": {
                    "dir": "/tmp/x",
                    "files": {"tokenfuse": "tokenfuse.ndjson", "wardryx": "wardryx.ndjson"}
                },
                "keys": {}
            }"#,
        );

        let resolved = discover_taipan_in(&dir).expect("must resolve the fixture descriptor");
        assert_eq!(
            resolved.source,
            IdryxEnvSource::Taipan {
                name: "p1full".to_string()
            }
        );
        assert_eq!(resolved.idryx_url, "http://127.0.0.1:8081");

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

    /// A descriptor from a `taipan up` that never started idryx at all (no
    /// `services.idryx` key) must fall through cleanly, never error - this is
    /// the "no identity plane" empty-state case (PHASE3.md).
    #[test]
    fn a_descriptor_missing_the_idryx_service_falls_through() {
        let dir = unique_dir("no-idryx");
        write(
            &dir.join("broken.json"),
            r#"{"name":"broken","services":{"cloud":{"url":"http://x"}},"events":{"dir":"","files":{}}}"#,
        );
        assert!(discover_taipan_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The core reason [`newest_descriptor_with_idryx`] exists rather than
    /// simply taking the newest file regardless of content: a newer
    /// descriptor with no idryx service must not shadow an older one that
    /// does have one.
    #[test]
    fn newer_descriptor_without_idryx_does_not_shadow_an_older_one_with_it() {
        let dir = unique_dir("shadow");
        write(
            &dir.join("older.json"),
            r#"{"name":"older","services":{"idryx":{"url":"http://127.0.0.1:1"}},
                "events":{"dir":"/tmp/older","files":{"tokenfuse":"tokenfuse.ndjson"}}}"#,
        );
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(
            &dir.join("newer.json"),
            r#"{"name":"newer","services":{"cloud":{"url":"http://127.0.0.1:2"}},
                "events":{"dir":"/tmp/newer","files":{}}}"#,
        );

        let resolved =
            discover_taipan_in(&dir).expect("must fall through to the older, usable descriptor");
        assert_eq!(
            resolved.source,
            IdryxEnvSource::Taipan {
                name: "older".to_string()
            }
        );
        assert_eq!(resolved.idryx_url, "http://127.0.0.1:1");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn newest_descriptor_with_idryx_wins_when_multiple_qualify() {
        let dir = unique_dir("multi");
        write(
            &dir.join("older.json"),
            r#"{"name":"older","services":{"idryx":{"url":"http://127.0.0.1:1"}},
                "events":{"dir":"/tmp/older","files":{}}}"#,
        );
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(
            &dir.join("newer.json"),
            r#"{"name":"newer","services":{"idryx":{"url":"http://127.0.0.1:2"}},
                "events":{"dir":"/tmp/newer","files":{}}}"#,
        );

        let resolved = discover_taipan_in(&dir).expect("must resolve one of the two");
        assert_eq!(
            resolved.source,
            IdryxEnvSource::Taipan {
                name: "newer".to_string()
            }
        );
        assert_eq!(resolved.idryx_url, "http://127.0.0.1:2");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- env fallback -------------------------------------------------

    #[test]
    fn env_fallback_requires_a_non_blank_url() {
        assert!(env_fallback_from(None).is_none());
        assert!(env_fallback_from(Some(String::new())).is_none());
        assert!(env_fallback_from(Some("   ".to_string())).is_none());

        let resolved = env_fallback_from(Some("http://127.0.0.1:8080".to_string()))
            .expect("a non-blank URL must resolve");
        assert_eq!(resolved.source, IdryxEnvSource::EnvFallback);
        assert_eq!(resolved.idryx_url, "http://127.0.0.1:8080");
    }

    // ---- idryx binary location -----------------------------------------

    #[test]
    fn idryx_binary_under_missing_home_yields_none() {
        let home = unique_dir("no-bin-home");
        assert!(idryx_binary_under(&home).is_none());
    }

    #[test]
    fn idryx_binary_under_finds_a_real_file() {
        let home = unique_dir("has-bin-home");
        let bin = home.join(".taipan").join("bin").join("idryx");
        write(&bin, "#!/bin/sh\nexit 0\n");

        let found = idryx_binary_under(&home).expect("must find the fixture binary");
        assert_eq!(found, bin);

        let _ = std::fs::remove_dir_all(&home);
    }

    // ---- rescan loads ---------------------------------------------------

    #[test]
    fn rescan_loads_in_missing_directory_is_an_honest_error_not_a_panic() {
        let dir = unique_dir("rescan-missing").join("nested");
        let err =
            rescan_loads_in(&dir).expect_err("a missing directory has no idryx-bearing descriptor");
        assert!(
            err.contains("idryx"),
            "reason should name what's missing: {err}"
        );
    }

    #[test]
    fn rescan_loads_in_filters_to_recognized_sources_and_builds_absolute_paths() {
        let dir = unique_dir("rescan-happy");
        write(
            &dir.join("env1.json"),
            r#"{
                "name": "env1",
                "services": {"idryx": {"url": "http://127.0.0.1:8081"}},
                "events": {
                    "dir": "/tmp/genaryx-events",
                    "files": {
                        "tokenfuse": "tokenfuse.ndjson",
                        "wardryx": "wardryx.ndjson",
                        "mockryx": "mockryx.ndjson",
                        "verdryx": "verdryx.ndjson",
                        "okta": "okta.json"
                    }
                }
            }"#,
        );

        let loads = rescan_loads_in(&dir).expect("must resolve the fixture descriptor's loads");
        assert_eq!(
            loads.len(),
            4,
            "okta is not a recognized stack-bus source: {loads:?}"
        );
        assert_eq!(
            loads,
            vec![
                (
                    "tokenfuse".to_string(),
                    "/tmp/genaryx-events/tokenfuse.ndjson".to_string()
                ),
                (
                    "wardryx".to_string(),
                    "/tmp/genaryx-events/wardryx.ndjson".to_string()
                ),
                (
                    "mockryx".to_string(),
                    "/tmp/genaryx-events/mockryx.ndjson".to_string()
                ),
                (
                    "verdryx".to_string(),
                    "/tmp/genaryx-events/verdryx.ndjson".to_string()
                ),
            ],
            "must preserve RESCAN_SOURCES' declared order, not the descriptor's own BTreeMap order"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rescan_loads_in_with_no_recognized_sources_is_an_empty_ok_not_an_error() {
        let dir = unique_dir("rescan-empty-sources");
        write(
            &dir.join("env1.json"),
            r#"{"name":"env1","services":{"idryx":{"url":"http://127.0.0.1:8081"}},
                "events":{"dir":"/tmp/x","files":{"okta":"okta.json"}}}"#,
        );

        let loads = rescan_loads_in(&dir)
            .expect("a qualifying descriptor with zero recognized sources is not itself an error");
        assert!(loads.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
