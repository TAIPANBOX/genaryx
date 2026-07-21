//! Identity-panel environment discovery: which Idryx to talk to.
//!
//! Mirrors `crate::policy::env`/`crate::money::env` in shape (scan the SAME
//! `taipan up` descriptor directory, newest first), deliberately duplicated
//! rather than shared (07 §4.4's Idryx connector is its own independent
//! plane, same "parallel, not coupled" convention `crates/connectors/src/idryx.rs`
//! already keeps relative to `wardryx.rs`/`cloud_rest.rs`), but narrower in
//! two ways idryx itself is:
//!
//! 1. **One source, not two.** Idryx has no bearer at all (07 §4.4: "every
//!    `serve` route... is unauthenticated"), so unlike Policy/Money there is
//!    no key to gate an env-var fallback on for a hand-started idryx - this
//!    module only ever discovers from a `taipan up` descriptor. No usable
//!    descriptor (or one with no `services.idryx` entry - the common case
//!    for an environment brought up without `--with idryx`) resolves to
//!    `None`, and the caller (`super::state::bootstrap`) renders a clean "no
//!    identity plane" state - never an error.
//! 2. **Also carries the taipan events section.** Unlike Policy/Money's
//!    `ResolvedEnv`, this module's also reads the descriptor's `events`
//!    section (`events.dir`/`events.files` -
//!    `~/Development/taipan/src/descriptor.rs`'s `EventsSection`) alongside
//!    the idryx URL, since it comes off the exact same JSON file already
//!    being parsed. `super::state::bootstrap` turns this into Rescan's
//!    `--load` specs; a descriptor with no (or a blank) events section
//!    simply yields `events_dir: None`, so Rescan ends up with zero loads
//!    rather than this module inventing a path.
//!
//! This module never touches the network and never panics: every
//! filesystem/JSON step is a `?`-chained `Option`, so one malformed or
//! half-written descriptor falls through to the next candidate instead of
//! taking down discovery - same discipline `policy::env`/`money::env` keep.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where a [`ResolvedEnv`] came from, surfaced to the UI. A single variant
/// today (see this module's doc comment for why there is no env-fallback
/// counterpart to `policy::env::EnvSource::EnvFallback`) - kept as a tagged
/// enum rather than a bare struct so `identityTypes.ts`'s `EnvSource` stays
/// structurally parallel to `policyTypes.ts`/`moneyTypes.ts`'s, in case a
/// second discovery path is ever added here too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum EnvSource {
    /// Discovered from `~/.taipan/environments/<name>.json`.
    Taipan { name: String },
}

/// A fully-resolved place to talk to Idryx, plus whatever the SAME
/// descriptor's `events` section carried - see this module's doc comment for
/// why the events data rides along here instead of a separate resolver.
#[derive(Debug, Clone)]
pub struct ResolvedEnv {
    pub source: EnvSource,
    pub idryx_url: String,
    /// `events.dir` off the same descriptor, when present and non-blank -
    /// `None` otherwise (an older/malformed descriptor, or one written
    /// before `events` existed). Only consumed by
    /// `super::state::resolve_rescan_loads`.
    pub events_dir: Option<PathBuf>,
    /// `events.files` verbatim: source name -> ndjson filename relative to
    /// `events_dir` (e.g. `{"tokenfuse": "tokenfuse.ndjson"}`, plus
    /// `"wardryx"` when that service was started too). Empty when the
    /// descriptor carries no events section at all.
    pub event_files: BTreeMap<String, String>,
}

// ---- descriptor wire shapes (read-only mirror) -----------------------------
// Deliberately duplicated from `policy::env`/`money::env`'s own private
// structs rather than shared - see this module's doc comment.

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

/// Resolve the Identity panel's environment: the newest `taipan up`
/// descriptor with a usable `services.idryx` entry, or `None` for a clean
/// "no identity plane" state.
#[must_use]
pub fn discover() -> Option<ResolvedEnv> {
    let dir = genaryx_core::taipan_home::environments_dir()?;
    discover_taipan_in(&dir)
}

/// Testable core of the discovery path: scan `environments_dir` for
/// descriptor files (newest last-modified first), and return the first one
/// that yields a usable Idryx URL. A descriptor with no `services.idryx`
/// entry at all (no `--with idryx`) simply yields `None` for that
/// candidate, same as any other missing field.
fn discover_taipan_in(environments_dir: &Path) -> Option<ResolvedEnv> {
    let mut candidates = list_descriptor_paths(environments_dir);
    candidates.sort_by_key(|p| std::cmp::Reverse(modified_time(p)));
    candidates.into_iter().find_map(|p| try_load_descriptor(&p))
}

/// Every `<name>.json` descriptor in `dir`, excluding the sibling
/// `<name>.keys.json` / `<name>.pid.json` files - identical filter to
/// `policy::env::list_descriptor_paths`/`money::env::list_descriptor_paths`.
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

/// Load and resolve one descriptor: read the `idryx` service URL (falling
/// through to the next candidate when absent - no `--with idryx`) plus the
/// `events` section riding along on the same file. `None` at any load/parse
/// step falls through rather than erroring - mirrors
/// `policy::env::try_load_descriptor` field-for-field, minus the key
/// resolution idryx has no use for.
fn try_load_descriptor(path: &Path) -> Option<ResolvedEnv> {
    let bytes = std::fs::read(path).ok()?;
    let descriptor: Descriptor = serde_json::from_slice(&bytes).ok()?;

    let idryx_url = descriptor.services.get("idryx")?.url.clone();
    let events_dir = if descriptor.events.dir.trim().is_empty() {
        None
    } else {
        Some(PathBuf::from(&descriptor.events.dir))
    };

    Some(ResolvedEnv {
        source: EnvSource::Taipan {
            name: descriptor.name,
        },
        idryx_url,
        events_dir,
        event_files: descriptor.events.files,
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
            "genaryx-identity-env-test-{tag}-{}-{n}",
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
    fn resolves_a_real_shaped_descriptor_including_the_events_section() {
        let dir = unique_dir("happy");
        write(
            &dir.join("p1full.json"),
            r#"{
                "name": "p1full",
                "created_at": "2026-07-16T00:00:00Z",
                "host": "box.local",
                "services": {
                    "cloud": {"url": "http://127.0.0.1:41001"},
                    "idryx": {"url": "http://127.0.0.1:41003"}
                },
                "events": {"dir": "/tmp/taipan-events", "files": {"tokenfuse": "tokenfuse.ndjson", "wardryx": "wardryx.ndjson"}},
                "keys": {"cloud_admin_ref": "taipan/p1full/cloud_admin"}
            }"#,
        );

        let resolved = discover_taipan_in(&dir).expect("must resolve the fixture descriptor");
        assert_eq!(
            resolved.source,
            EnvSource::Taipan {
                name: "p1full".to_string()
            }
        );
        assert_eq!(resolved.idryx_url, "http://127.0.0.1:41003");
        assert_eq!(
            resolved.events_dir,
            Some(PathBuf::from("/tmp/taipan-events"))
        );
        assert_eq!(
            resolved.event_files.get("tokenfuse").map(String::as_str),
            Some("tokenfuse.ndjson")
        );
        assert_eq!(resolved.event_files.len(), 2);

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
    fn a_descriptor_with_no_idryx_service_falls_through() {
        // The common case today - an environment brought up without
        // `--with idryx` has a `cloud` service (and maybe `wardryx`) but no
        // `idryx` one at all.
        let dir = unique_dir("no-idryx");
        write(
            &dir.join("plain.json"),
            r#"{"name":"plain","services":{"cloud":{"url":"http://x"}},"events":{"dir":"/tmp/x","files":{}}}"#,
        );
        assert!(discover_taipan_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_descriptor_with_no_events_section_still_resolves_with_no_rescan_data() {
        // idryx itself needs no key/secret, so a bare `services.idryx.url`
        // is enough to resolve a connection even when (an older, or
        // hand-written) descriptor carries no `events` section at all -
        // Rescan just ends up with nothing to load from, not an error.
        let dir = unique_dir("no-events");
        write(
            &dir.join("bare.json"),
            r#"{"name":"bare","services":{"idryx":{"url":"http://127.0.0.1:8081"}}}"#,
        );
        let resolved = discover_taipan_in(&dir).expect("must resolve on url alone");
        assert_eq!(resolved.idryx_url, "http://127.0.0.1:8081");
        assert_eq!(resolved.events_dir, None);
        assert!(resolved.event_files.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn newest_descriptor_wins_when_multiple_environments_exist() {
        let dir = unique_dir("multi");
        write(
            &dir.join("older.json"),
            r#"{"name":"older","services":{"idryx":{"url":"http://127.0.0.1:1"}}}"#,
        );
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(
            &dir.join("newer.json"),
            r#"{"name":"newer","services":{"idryx":{"url":"http://127.0.0.1:2"}}}"#,
        );

        let resolved = discover_taipan_in(&dir).expect("must resolve one of the two");
        assert_eq!(
            resolved.source,
            EnvSource::Taipan {
                name: "newer".to_string()
            }
        );
        assert_eq!(resolved.idryx_url, "http://127.0.0.1:2");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
