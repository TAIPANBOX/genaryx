//! Where the console's event bus actually comes from.
//!
//! Every other plane (money, policy, identity, quality, crypto, drills)
//! reaches its source through a per-plane `env.rs` that reads a `taipan up`
//! descriptor under `~/.taipan/environments/<name>.json`. The bus had no such
//! module, and the consequence was not a missing feature but a wrong one:
//! both shells called [`crate::demo::generate`] unconditionally at startup and
//! then ran a background thread appending one fabricated event every two
//! seconds, which the Bus Explorer presented as a live feed. That was correct
//! for the Phase-0 exit gate ("both shells show the same live event stream
//! from the shared core") and was never replaced afterwards, so on every
//! machine, in both shells, the stream was synthetic.
//!
//! This module is the missing half. The descriptor already carries the answer:
//!
//! ```json
//! "events": { "dir": "/tmp/genaryx-events", "files": {} }
//! ```
//!
//! Unlike the per-plane modules, this one lives in the core rather than being
//! mirrored into each shell. Those mirror each other because each shell owns
//! its own connector state; the bus is different, both shells already drive
//! the same [`crate::ingest::IngestService`] from the same core, so a second
//! copy of this logic would only be a second thing to drift.
//!
//! Never touches the network, never creates anything, and returns `None`
//! rather than erroring when there is no environment: "no environment" is a
//! legitimate state that the caller renders as an honest empty (or explicitly
//! labelled demo) surface, not a failure.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// A resolved bus: which environment, and the directory whose `*.ndjson`
/// files carry its agent events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBus {
    /// The descriptor's own `name`, shown in the UI so an operator can always
    /// see which environment the console is reading.
    pub env_name: String,
    /// The `events.dir` of that descriptor. May not exist yet: a freshly
    /// created environment that has not been run has no event files, which is
    /// an honest empty bus, not an error.
    pub events_dir: PathBuf,
}

/// Resolve the bus from the newest usable `taipan up` descriptor, or `None`
/// when there is no environment on this machine.
///
/// Newest-first by modification time, matching every per-plane `env.rs`, so
/// the most recently `taipan up`'d environment wins when several exist. That
/// tie-break matters here: this machine accumulates one descriptor per live
/// campaign, and the console must follow the current one rather than whichever
/// name happens to sort first.
#[must_use]
pub fn discover() -> Option<ResolvedBus> {
    discover_in(&taipan_environments_dir()?)
}

/// `~/.taipan/environments`, honouring `TAIPAN_HOME` so an entire install can
/// be pointed at a scratch directory (stack-up honours the same variable; a
/// clean-machine test where the tools write to a scratch home and the console
/// reads the real one proves nothing).
///
/// `None` when neither `TAIPAN_HOME` nor `HOME` is set, rather than a panic
/// over a missing environment variable.
fn taipan_environments_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("TAIPAN_HOME") {
        return Some(PathBuf::from(home).join("environments"));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".taipan").join("environments"))
}

/// Testable core of [`discover`]: the first descriptor in `environments_dir`
/// (newest first) that names an events directory.
#[must_use]
pub fn discover_in(environments_dir: &Path) -> Option<ResolvedBus> {
    let mut candidates = list_descriptor_paths(environments_dir);
    candidates.sort_by_key(|p| std::cmp::Reverse(modified_time(p)));
    candidates.iter().find_map(|p| try_load(p))
}

/// Every `<name>.json` descriptor in `dir`, excluding the sibling
/// `<name>.keys.json` and `<name>.pid.json` files (a secrets file must never
/// be parsed as a descriptor). An unreadable or absent directory yields no
/// candidates rather than an error, mirroring `cloud::env`'s own rule.
fn list_descriptor_paths(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .filter(|p| {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
            !stem.ends_with(".keys") && !stem.ends_with(".pid")
        })
        .collect()
}

fn modified_time(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Parse one descriptor, keeping it only if it names an events directory.
/// Anything unreadable, unparseable or missing `events.dir` is skipped in
/// silence so one stale file cannot mask a good environment sitting next to it.
fn try_load(path: &Path) -> Option<ResolvedBus> {
    let raw = std::fs::read_to_string(path).ok()?;
    let descriptor: Descriptor = serde_json::from_str(&raw).ok()?;
    let dir = descriptor.events.dir?;
    if dir.trim().is_empty() {
        return None;
    }
    Some(ResolvedBus {
        env_name: descriptor.name,
        events_dir: PathBuf::from(dir),
    })
}

// ---- descriptor wire shape (read-only, tolerant) --------------------------
// Only the fields this module reads are modelled, and everything optional, so
// a descriptor written by a newer `taipan up` never fails to parse here.

#[derive(Debug, Default, Deserialize)]
struct DescriptorEvents {
    #[serde(default)]
    dir: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Descriptor {
    name: String,
    #[serde(default)]
    events: DescriptorEvents,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "genaryx-bus-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, body).expect("write descriptor");
        path
    }

    #[test]
    fn resolves_the_events_dir_from_a_descriptor() {
        let dir = scratch("basic");
        write(
            &dir,
            "genaryx-live.json",
            r#"{"name":"genaryx-live","services":{},"events":{"dir":"/tmp/genaryx-events"}}"#,
        );

        let resolved = discover_in(&dir).expect("descriptor should resolve");
        assert_eq!(resolved.env_name, "genaryx-live");
        assert_eq!(resolved.events_dir, PathBuf::from("/tmp/genaryx-events"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_environments_directory_is_not_an_error() {
        let dir = scratch("missing").join("does-not-exist");
        assert_eq!(discover_in(&dir), None);
    }

    #[test]
    fn a_descriptor_without_an_events_dir_is_skipped() {
        let dir = scratch("noevents");
        write(
            &dir,
            "bare.json",
            r#"{"name":"bare","services":{"cloud":{"url":"http://127.0.0.1:8080"}}}"#,
        );
        assert_eq!(discover_in(&dir), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_keys_file_is_never_parsed_as_a_descriptor() {
        // The secrets sibling has the same `.json` extension and sits in the
        // same directory; parsing it would at best fail and at worst pick up
        // a "name" from a file that is not a descriptor at all.
        let dir = scratch("keys");
        write(
            &dir,
            "genaryx-live.keys.json",
            r#"{"secrets":{"cloud_admin":"devkey"}}"#,
        );
        assert_eq!(discover_in(&dir), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unparseable_descriptor_does_not_mask_a_good_one() {
        let dir = scratch("mixed");
        write(&dir, "broken.json", "{ this is not json");
        let good = write(
            &dir,
            "good.json",
            r#"{"name":"good","events":{"dir":"/tmp/good-events"}}"#,
        );
        // Make the broken one newest, so it is tried first and must be
        // stepped over rather than ending the search.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        let _ = fs::File::open(&good)
            .and_then(|f| f.set_modified(later - std::time::Duration::from_secs(120)));
        let _ = fs::File::open(dir.join("broken.json")).and_then(|f| f.set_modified(later));

        let resolved = discover_in(&dir).expect("the good descriptor should still win");
        assert_eq!(resolved.env_name, "good");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_newest_descriptor_wins() {
        let dir = scratch("newest");
        let old = write(
            &dir,
            "old.json",
            r#"{"name":"old","events":{"dir":"/tmp/old"}}"#,
        );
        let new = write(
            &dir,
            "new.json",
            r#"{"name":"new","events":{"dir":"/tmp/new"}}"#,
        );
        let now = std::time::SystemTime::now();
        let _ = fs::File::open(&old)
            .and_then(|f| f.set_modified(now - std::time::Duration::from_secs(600)));
        let _ = fs::File::open(&new).and_then(|f| f.set_modified(now));

        let resolved = discover_in(&dir).expect("one of them should resolve");
        assert_eq!(resolved.env_name, "new");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_events_dir_string_is_not_a_bus() {
        let dir = scratch("empty");
        write(&dir, "e.json", r#"{"name":"e","events":{"dir":"   "}}"#);
        assert_eq!(discover_in(&dir), None);
        let _ = fs::remove_dir_all(&dir);
    }
}
