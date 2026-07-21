//! Evidence-panel environment discovery (docs/PHASE4.md W3): the LOCAL-TOOL
//! sources the Evidence Center can gather from - qryx, idryx, TokenFuse -
//! each resolved fully independently of the others and of Cloud (Cloud is
//! not resolved here at all; the panel reuses the Money plane's already-
//! paired `CloudClient`, see `commands.rs`'s module doc for why). Mirrors
//! `crypto::env`/`drills::env`/`identity::env`'s well-known-path + taipan-
//! descriptor conventions, duplicated rather than shared (same rationale
//! `identity::env`'s own doc comment gives: independent planes evolve
//! independently, so each panel's env.rs owns its own copy of this class of
//! logic rather than coupling to a sibling's).
//!
//! - **qryx**: `~/.taipan/bin/qryx` + a default scan target (`$HOME`) - the
//!   SAME resolution `crypto::env::discover` uses (Evidence's Qryx artifact
//!   is quite literally the same qryx binary Crypto already scans with), just
//!   re-derived here rather than cross-calling that sibling module.
//! - **idryx**: `~/.taipan/bin/idryx` + `--load source:path` specs built from
//!   the newest `taipan up` descriptor's `events` section, exactly like
//!   `identity::state`'s Rescan loads - EXCEPT this does NOT require a
//!   `services.idryx` entry on the descriptor the way `identity::env` does:
//!   Agent-BOM (like Rescan) is a pure CLI call over local ndjson files, it
//!   never talks to `idryx serve`'s HTTP API at all, so gating it on that
//!   service being configured would be an artificial coupling Evidence has no
//!   use for.
//! - **tokenfuse**: `~/.taipan/bin/tokenfuse-gateway` (the installed name
//!   confirmed on a live `taipan up` box - the gateway crate's own binary
//!   target is named `tokenfuse`, `~/Development/tokenfuse/crates/gateway/
//!   Cargo.toml`, but `taipan up` installs it as `tokenfuse-gateway` under
//!   `~/.taipan/bin`, alongside `tokenfuse-cloud`) + a default traces dir:
//!   `~/Development/taipan/src/home.rs`'s own `TaipanHome::traces_dir(name,
//!   "gateway")` convention - `<environments_dir>/<name>.traces/gateway`, the
//!   sibling directory `taipan up` already writes the Parquet call trace
//!   under for the newest environment (ground-truthed directly against a
//!   live `~/.taipan/environments/*.traces/gateway` on this box, 2026-07-17).
//!
//! Never panics: every filesystem/JSON step is a `?`-chained `Option`, same
//! discipline as every sibling env.rs.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// idryx's stack-bus `--load` vocabulary Agent-BOM accepts - mirrors
/// `identity::state::ACCEPTED_LOAD_SOURCES` exactly (duplicated per this
/// module's own doc comment, not shared: `identity::state`'s copy is private
/// and this is an independent plane).
const ACCEPTED_LOAD_SOURCES: &[&str] = &["tokenfuse", "wardryx", "mockryx", "verdryx"];

/// qryx: the binary plus a default scan target - field-for-field identical to
/// `crypto::env::ResolvedEnv`.
#[derive(Debug, Clone)]
pub struct ResolvedQryx {
    pub qryx_bin: PathBuf,
    pub default_target: PathBuf,
}

/// idryx: the binary plus Agent-BOM's `--load` specs.
#[derive(Debug, Clone)]
pub struct ResolvedIdryx {
    pub idryx_bin: PathBuf,
    pub loads: Vec<(String, PathBuf)>,
}

/// TokenFuse: the gateway binary plus a default traces dir.
#[derive(Debug, Clone)]
pub struct ResolvedTokenfuse {
    pub tokenfuse_bin: PathBuf,
    /// A starting point for the operator's editable traces-dir field, not an
    /// authority - mirrors `crypto::env::ResolvedEnv::default_target`'s
    /// identical "starting point, not a claim" role. `None` when no taipan
    /// environment (hence no `<name>.traces/gateway`) was found.
    pub default_traces_dir: Option<PathBuf>,
}

/// `~/.taipan/bin/qryx`, best-effort - see this module's doc comment for why
/// this is a deliberate re-derivation of `crypto::env::discover` rather than
/// a cross-call.
#[must_use]
pub fn discover_qryx() -> Option<ResolvedQryx> {
    let home = std::env::var_os("HOME")?;
    let home = PathBuf::from(home);
    let qryx_bin = home.join(".taipan").join("bin").join("qryx");
    if !qryx_bin.is_file() {
        return None;
    }
    Some(ResolvedQryx {
        qryx_bin,
        default_target: home,
    })
}

/// `~/.taipan/bin/idryx` + Agent-BOM `--load` specs off the newest taipan
/// descriptor's events section - see this module's doc comment for why no
/// `services.idryx` entry is required. `None` only when the idryx binary
/// itself does not resolve; an empty load list is still `Some` (an honestly
/// smaller Agent-BOM input, not a failure to resolve the source itself).
#[must_use]
pub fn discover_idryx() -> Option<ResolvedIdryx> {
    let idryx_bin = well_known_bin("idryx")?;
    let loads = genaryx_core::taipan_home::environments_dir()
        .and_then(|dir| newest_descriptor_in(&dir))
        .map(|extras| agent_bom_loads(extras.events_dir.as_deref(), &extras.event_files))
        .unwrap_or_default();
    Some(ResolvedIdryx { idryx_bin, loads })
}

/// `~/.taipan/bin/tokenfuse-gateway` + the newest environment's
/// `<name>.traces/gateway` dir, when it exists.
#[must_use]
pub fn discover_tokenfuse() -> Option<ResolvedTokenfuse> {
    let tokenfuse_bin = well_known_bin("tokenfuse-gateway")?;
    let default_traces_dir = genaryx_core::taipan_home::environments_dir().and_then(|dir| {
        let extras = newest_descriptor_in(&dir)?;
        let candidate = dir.join(format!("{}.traces", extras.name)).join("gateway");
        candidate.is_dir().then_some(candidate)
    });
    Some(ResolvedTokenfuse {
        tokenfuse_bin,
        default_traces_dir,
    })
}

// ---- shared helpers ---------------------------------------------------

fn well_known_bin(name: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let candidate = PathBuf::from(home).join(".taipan").join("bin").join(name);
    candidate.is_file().then_some(candidate)
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
    #[serde(default)]
    events: DescriptorEvents,
}

/// The newest taipan descriptor's name + events section - deliberately NOT
/// gated on any `services.*` entry (see this module's doc comment).
struct DescriptorExtras {
    name: String,
    events_dir: Option<PathBuf>,
    event_files: BTreeMap<String, String>,
}

/// Testable core of the descriptor-scan path: scan `environments_dir` for
/// descriptor files (newest last-modified first) and return the first one
/// that parses, regardless of which `services.*` entries it carries.
fn newest_descriptor_in(environments_dir: &Path) -> Option<DescriptorExtras> {
    let mut candidates = list_descriptor_paths(environments_dir);
    candidates.sort_by_key(|p| std::cmp::Reverse(modified_time(p)));
    candidates.into_iter().find_map(|p| try_load_descriptor(&p))
}

fn try_load_descriptor(path: &Path) -> Option<DescriptorExtras> {
    let bytes = std::fs::read(path).ok()?;
    let d: Descriptor = serde_json::from_slice(&bytes).ok()?;
    let events_dir = if d.events.dir.trim().is_empty() {
        None
    } else {
        Some(PathBuf::from(&d.events.dir))
    };
    Some(DescriptorExtras {
        name: d.name,
        events_dir,
        event_files: d.events.files,
    })
}

/// Every `<name>.json` descriptor in `dir`, excluding the sibling
/// `<name>.keys.json` / `<name>.pid.json` files - identical filter to every
/// sibling env.rs's own `list_descriptor_paths`.
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

/// Build Agent-BOM's `--load` specs: `events_dir.join(file)` for every
/// `event_files` entry whose source idryx's stack-bus `--load` actually
/// accepts ([`ACCEPTED_LOAD_SOURCES`]), filtered to files that exist on disk
/// right now - mirrors `identity::state::resolve_rescan_loads` exactly
/// (duplicated, see this module's doc comment).
fn agent_bom_loads(
    events_dir: Option<&Path>,
    event_files: &BTreeMap<String, String>,
) -> Vec<(String, PathBuf)> {
    let Some(dir) = events_dir else {
        return Vec::new();
    };
    event_files
        .iter()
        .filter(|(source, _)| ACCEPTED_LOAD_SOURCES.contains(&source.as_str()))
        .map(|(source, file)| (source.clone(), dir.join(file)))
        .filter(|(_, path)| path.is_file())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "genaryx-evidence-env-test-{tag}-{}-{n}",
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
    fn discover_qryx_never_panics() {
        let _ = discover_qryx();
    }

    #[test]
    fn discover_idryx_never_panics() {
        let _ = discover_idryx();
    }

    #[test]
    fn discover_tokenfuse_never_panics() {
        let _ = discover_tokenfuse();
    }

    #[test]
    fn qryx_resolution_points_the_bin_and_default_target_under_the_same_home() {
        // Best-effort shape check only, mirrors `crypto::env`'s own
        // identical-rationale test: whether this box actually has the file
        // depends on local dev state, never required to exist.
        if let Some(home) = std::env::var_os("HOME") {
            let expected_bin = PathBuf::from(&home)
                .join(".taipan")
                .join("bin")
                .join("qryx");
            match discover_qryx() {
                Some(r) => {
                    assert_eq!(r.qryx_bin, expected_bin);
                    assert_eq!(r.default_target, PathBuf::from(&home));
                }
                None => assert!(!expected_bin.is_file()),
            }
        }
    }

    #[test]
    fn empty_environments_directory_yields_no_descriptor() {
        let dir = unique_dir("empty");
        std::fs::create_dir_all(&dir).expect("create dir");
        assert!(newest_descriptor_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_environments_directory_yields_no_descriptor_not_a_panic() {
        let dir = unique_dir("missing").join("nested").join("deeper");
        assert!(newest_descriptor_in(&dir).is_none());
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
        assert!(newest_descriptor_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_a_descriptor_with_no_services_section_at_all() {
        // The key divergence from `identity::env`: Agent-BOM needs no
        // `services.idryx` entry, so a descriptor carrying only `events`
        // (no `services` at all) still resolves here.
        let dir = unique_dir("no-services");
        write(
            &dir.join("bare.json"),
            r#"{"name":"bare","events":{"dir":"/tmp/x","files":{"tokenfuse":"tokenfuse.ndjson"}}}"#,
        );
        let extras = newest_descriptor_in(&dir).expect("must resolve on events alone");
        assert_eq!(extras.name, "bare");
        assert_eq!(extras.events_dir, Some(PathBuf::from("/tmp/x")));
        assert_eq!(
            extras.event_files.get("tokenfuse").map(String::as_str),
            Some("tokenfuse.ndjson")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn descriptor_with_no_events_section_still_resolves_with_no_loads() {
        let dir = unique_dir("no-events");
        write(&dir.join("bare.json"), r#"{"name":"bare"}"#);
        let extras = newest_descriptor_in(&dir).expect("must resolve on name alone");
        assert_eq!(extras.name, "bare");
        assert_eq!(extras.events_dir, None);
        assert!(extras.event_files.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn newest_descriptor_wins_when_multiple_environments_exist() {
        let dir = unique_dir("multi");
        write(&dir.join("older.json"), r#"{"name":"older"}"#);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(&dir.join("newer.json"), r#"{"name":"newer"}"#);

        let extras = newest_descriptor_in(&dir).expect("must resolve one of the two");
        assert_eq!(extras.name, "newer");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn agent_bom_loads_filters_accepted_sources_and_existing_files() {
        let dir = unique_dir("loads");
        std::fs::create_dir_all(&dir).expect("create dir");
        std::fs::write(dir.join("tokenfuse.ndjson"), "").expect("touch tokenfuse file");
        // "wardryx.ndjson" deliberately NOT written - a declared-but-missing
        // file must be skipped, never fabricated into a load spec.

        let mut files = BTreeMap::new();
        files.insert("tokenfuse".to_string(), "tokenfuse.ndjson".to_string());
        files.insert("wardryx".to_string(), "wardryx.ndjson".to_string());
        files.insert("okta".to_string(), "okta.ndjson".to_string()); // not an accepted stack-bus source

        let loads = agent_bom_loads(Some(&dir), &files);
        assert_eq!(loads.len(), 1, "got {loads:?}");
        assert_eq!(loads[0].0, "tokenfuse");
        assert_eq!(loads[0].1, dir.join("tokenfuse.ndjson"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn agent_bom_loads_is_empty_with_no_events_dir() {
        assert!(agent_bom_loads(None, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn traces_dir_is_the_descriptor_name_sibling_when_it_exists() {
        let dir = unique_dir("traces");
        write(&dir.join("p1full.json"), r#"{"name":"p1full"}"#);
        std::fs::create_dir_all(dir.join("p1full.traces").join("gateway"))
            .expect("create fixture traces dir");

        let extras = newest_descriptor_in(&dir).expect("must resolve");
        let candidate = dir.join(format!("{}.traces", extras.name)).join("gateway");
        assert!(candidate.is_dir());
        assert_eq!(candidate, dir.join("p1full.traces").join("gateway"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
