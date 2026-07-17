//! Environment discovery for [`super::DrillsHandle`]: where to find the
//! `mockryx` binary, plus (independently) a default scenario directory, the
//! TokenFuse gateway URL to rehearse against, an optional api key, and a
//! well-known path to save/load the "last run" report from.
//!
//! ## The `mockryx` binary: genuinely no standard install path
//!
//! Unlike Idryx/Qryx (`taipan up --with <name>` installs to
//! `~/.taipan/bin/<name>`), mockryx is a fire-drill runner, not a service
//! `taipan up` keeps running - there is no equivalent taipan-managed
//! location for it (the task brief this module was built against says so
//! explicitly: "no standard install path"). Two tiers instead:
//!
//! 1. [`MOCKRYX_BIN_ENV_VAR`] - a genaryx-side convenience (mockryx itself
//!    has no documented env var for its OWN binary path, only for
//!    `--gateway`/`--api-key`/`--events`/`--watch-events` - `mockryx run
//!    --help`), mirroring [`crate::crypto::env::QRYX_SCAN_ROOT_ENV_VAR`]'s
//!    "not tool-documented, but a genaryx-side convenience" idiom. This is
//!    this module's read of the task brief's "a descriptor path" phrase: no
//!    `taipan up` descriptor field for a mockryx binary exists anywhere in
//!    this codebase to ground a JSON key name against, so an explicit
//!    operator/deployment override is realized as an env var instead of a
//!    guessed descriptor field - flagged here as a deliberate interpretation
//!    rather than a discovered fact, mirroring `crypto::env`'s own "flagged,
//!    deliberate asymmetry" precedent.
//! 2. `~/Development/mockryx/bin/mockryx` - a checkout's own build output
//!    directory, EXACTLY the task brief's other named option ("a checkout's
//!    `bin/mockryx`") and exactly where `crates/connectors/tests/
//!    exit_gate_test.rs`'s own `build_mockryx()` places a freshly-built
//!    binary (`go build -o bin/mockryx ./cmd/mockryx`) - so a developer box
//!    that has ever run that test, or built mockryx by hand the same way,
//!    is picked up for free.
//!
//! [`super::DrillsHandle::connect`] remains the escape hatch for any other
//! location.
//!
//! ## Scenario directory: always resolves, never gates readiness
//!
//! Mirrors [`crate::crypto::env::default_scan_target`]'s own "always
//! resolves to SOME path, never `None`, never enforced" contract: a
//! pre-filled suggestion the operator can see/override before pressing Run,
//! not a presence/absence signal (docs/PHASE4.md W2's empty-state guard names
//! only "no mockryx binary / no gateway" - not "no scenario dir").
//!
//! ## Gateway URL: the SAME `services.gateway.url` every other plane uses
//!
//! Unlike the binary (genuinely no taipan-managed convention), the gateway
//! IS a real `taipan up`-registered service - the exact `services.gateway`
//! entry already visible in [`crate::wardryx::env`]'s and
//! [`crate::cloud::env`]'s own descriptor fixtures (`{"url":
//! "http://127.0.0.1:41000", "mode": "enforce"}`). [`default_gateway`]
//! mirrors their descriptor-first, env-fallback-second shape, reusing
//! mockryx's OWN documented `MOCKRYX_GATEWAY` (docs/PHASE4.md: "`--gateway`...
//! fall back to `MOCKRYX_GATEWAY`") for the fallback tier - unlike the
//! binary, this env var IS tool-documented, so reusing its exact name is the
//! same "operator already configured this, pick it up for free" reasoning
//! [`crate::quality::env`]'s `VERDRYX_DB_ENV_VAR` gives for reusing verdryx's
//! own name. `None` (no descriptor, no env var) is a real, common outcome -
//! the Swift panel shows an editable, initially-blank gateway field rather
//! than a whole-panel empty state for it (see `super`'s module doc).
//!
//! `default_api_key` is simpler: [`MOCKRYX_API_KEY_ENV_VAR`] only (also
//! tool-documented), no descriptor lookup - there is no established
//! descriptor field for a gateway API key anywhere in this codebase's
//! fixtures to ground one against.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Genaryx-side convenience override for the mockryx binary path - see the
/// module doc's "no standard install path" section.
const MOCKRYX_BIN_ENV_VAR: &str = "MOCKRYX_BIN";
/// Genaryx-side convenience override for the scenario directory.
const MOCKRYX_SCENARIOS_DIR_ENV_VAR: &str = "MOCKRYX_SCENARIOS_DIR";
/// mockryx's OWN documented gateway env var (docs/PHASE4.md: "fall back to
/// `MOCKRYX_GATEWAY`").
const MOCKRYX_GATEWAY_ENV_VAR: &str = "MOCKRYX_GATEWAY";
/// mockryx's OWN documented api-key env var (docs/PHASE4.md: "`_API_KEY`").
const MOCKRYX_API_KEY_ENV_VAR: &str = "MOCKRYX_API_KEY";

/// Where a [`ResolvedBin`] came from, surfaced to the Swift shell (06 §0.5),
/// exported as a UniFFI enum. Named distinctly from every sibling
/// `*EnvSource` for the same flat-per-crate-namespace reason
/// `crate::idryx::env::IdryxEnvSource`'s own doc comment gives.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum DrillsEnvSource {
    /// A sibling `~/Development/mockryx/bin/mockryx` checkout build.
    Checkout,
    /// An operator-supplied path via [`MOCKRYX_BIN_ENV_VAR`] or
    /// [`super::DrillsHandle::connect`] (which always reports this variant
    /// too - mirrors `CryptoEnvSource::Explicit`'s own dual use).
    Explicit,
}

/// A resolved `mockryx` binary path plus where it came from.
#[derive(Debug, Clone)]
pub struct ResolvedBin {
    pub source: DrillsEnvSource,
    pub bin: PathBuf,
}

/// Resolve the `mockryx` binary: [`MOCKRYX_BIN_ENV_VAR`], then a sibling
/// checkout's `bin/mockryx`, or `None` for a clean "no drills plane" state -
/// see the module doc.
#[must_use]
pub fn discover_bin() -> Option<ResolvedBin> {
    env_var_bin().or_else(checkout_bin)
}

fn env_var_bin() -> Option<ResolvedBin> {
    env_var_bin_from(std::env::var(MOCKRYX_BIN_ENV_VAR).ok())
}

/// Testable core of [`env_var_bin`], taking the (already-read) value directly
/// so tests never have to mutate real process environment. Honored even when
/// the path does not (yet) exist - an explicit override should fail with an
/// honest spawn error on first real use, not be silently skipped for a
/// different binary the operator did not ask for (mirrors
/// `crate::quality::env::env_var_from`'s own "honored regardless of
/// existence" reasoning).
fn env_var_bin_from(value: Option<String>) -> Option<ResolvedBin> {
    let value = value?;
    if value.trim().is_empty() {
        return None;
    }
    Some(ResolvedBin {
        source: DrillsEnvSource::Explicit,
        bin: PathBuf::from(value),
    })
}

fn checkout_bin() -> Option<ResolvedBin> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    checkout_bin_under(&home).map(|bin| ResolvedBin {
        source: DrillsEnvSource::Checkout,
        bin,
    })
}

/// Testable core of [`checkout_bin`]: `home/Development/mockryx/bin/mockryx`,
/// `None` when nothing file-shaped exists there yet.
fn checkout_bin_under(home: &Path) -> Option<PathBuf> {
    let path = home
        .join("Development")
        .join("mockryx")
        .join("bin")
        .join("mockryx");
    path.is_file().then_some(path)
}

/// [`MOCKRYX_SCENARIOS_DIR_ENV_VAR`], then the sibling checkout's shipped
/// `scenarios/` (`crates/connectors/src/mockryx.rs`'s own doc: "the mockryx
/// checkout's shipped `scenarios/` is the usual one"), else a bare relative
/// `"scenarios"` as the absolute last resort. ALWAYS resolves to SOME path,
/// never `None` - see the module doc.
#[must_use]
pub fn default_scenario_dir() -> PathBuf {
    env_scenario_dir()
        .or_else(checkout_scenario_dir)
        .unwrap_or_else(|| PathBuf::from("scenarios"))
}

fn env_scenario_dir() -> Option<PathBuf> {
    env_scenario_dir_from(std::env::var(MOCKRYX_SCENARIOS_DIR_ENV_VAR).ok())
}

fn env_scenario_dir_from(value: Option<String>) -> Option<PathBuf> {
    let value = value?;
    if value.trim().is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

fn checkout_scenario_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let path = PathBuf::from(home)
        .join("Development")
        .join("mockryx")
        .join("scenarios");
    path.is_dir().then_some(path)
}

// ---- gateway URL (descriptor-first, env-fallback-second) ------------------
// Only the fields this module actually reads are modeled; `#[serde(default)]`
// / unknown-field tolerance throughout so a descriptor with extra fields
// never fails to parse - mirrors `crate::wardryx::env`'s own convention.

#[derive(Debug, Deserialize)]
struct DescriptorService {
    url: String,
}

#[derive(Debug, Deserialize)]
struct Descriptor {
    services: BTreeMap<String, DescriptorService>,
}

/// Resolve the TokenFuse gateway URL to rehearse drills against: prefer a
/// `taipan up` descriptor's `services.gateway.url`, fall back to
/// [`MOCKRYX_GATEWAY_ENV_VAR`], or `None` when neither resolves - a common,
/// non-gating outcome (see the module doc).
#[must_use]
pub fn default_gateway() -> Option<String> {
    descriptor_gateway().or_else(env_gateway)
}

/// `~/.taipan/environments`, or `None` when `$HOME` is not set.
fn taipan_environments_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".taipan").join("environments"))
}

fn descriptor_gateway() -> Option<String> {
    descriptor_gateway_in(&taipan_environments_dir()?)
}

/// Testable core: the newest descriptor in `dir` with a `services.gateway`
/// entry, newest-modified first (mirrors
/// `crate::idryx::env::newest_descriptor_with_idryx`'s own shape, swapped to
/// the `gateway` service key).
fn descriptor_gateway_in(dir: &Path) -> Option<String> {
    let mut candidates = list_descriptor_paths(dir);
    candidates.sort_by_key(|p| std::cmp::Reverse(modified_time(p)));
    candidates.into_iter().find_map(|p| {
        let bytes = std::fs::read(&p).ok()?;
        let descriptor: Descriptor = serde_json::from_slice(&bytes).ok()?;
        descriptor.services.get("gateway").map(|s| s.url.clone())
    })
}

/// Every `<name>.json` descriptor in `dir`, excluding the sibling
/// `<name>.keys.json` / `<name>.pid.json` files - mirrors
/// `crate::cloud::env::list_descriptor_paths` exactly.
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

fn env_gateway() -> Option<String> {
    env_gateway_from(std::env::var(MOCKRYX_GATEWAY_ENV_VAR).ok())
}

fn env_gateway_from(value: Option<String>) -> Option<String> {
    let value = value?;
    if value.trim().is_empty() {
        return None;
    }
    Some(value)
}

/// [`MOCKRYX_API_KEY_ENV_VAR`] - see the module doc for why this has no
/// descriptor tier.
#[must_use]
pub fn default_api_key() -> Option<String> {
    env_api_key_from(std::env::var(MOCKRYX_API_KEY_ENV_VAR).ok())
}

fn env_api_key_from(value: Option<String>) -> Option<String> {
    let value = value?;
    if value.trim().is_empty() {
        return None;
    }
    Some(value)
}

/// `~/.taipan/mockryx-last-report.json` - a well-known place to `--save` a
/// report to and later `report`/`load_report` it back from across app
/// restarts (see `super`'s module doc on why [`super::DrillsHandle`] uses
/// this for its own "last run, even across restarts" behavior). Falls back
/// to a bare relative filename when `$HOME` is unset - always resolves to
/// SOME path, mirroring [`default_scenario_dir`]'s own contract.
#[must_use]
pub fn default_save_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(|home| {
            PathBuf::from(home)
                .join(".taipan")
                .join("mockryx-last-report.json")
        })
        .unwrap_or_else(|| PathBuf::from("mockryx-last-report.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "genaryx-ffi-drills-env-test-{tag}-{}-{n}",
            std::process::id()
        ))
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(path, body).expect("write fixture file");
    }

    // ---- mockryx binary location -------------------------------------------

    #[test]
    fn env_var_bin_requires_a_non_blank_value() {
        assert!(env_var_bin_from(None).is_none());
        assert!(env_var_bin_from(Some(String::new())).is_none());
        assert!(env_var_bin_from(Some("   ".to_string())).is_none());

        let resolved = env_var_bin_from(Some("/custom/mockryx".to_string()))
            .expect("a non-blank path resolves");
        assert_eq!(resolved.source, DrillsEnvSource::Explicit);
        assert_eq!(resolved.bin, PathBuf::from("/custom/mockryx"));
    }

    #[test]
    fn env_var_bin_is_honored_even_when_the_path_does_not_exist_yet() {
        let resolved = env_var_bin_from(Some("/definitely/not/a/real/mockryx".to_string()))
            .expect("explicit override resolves regardless of existence");
        assert_eq!(resolved.source, DrillsEnvSource::Explicit);
    }

    #[test]
    fn checkout_bin_under_missing_home_yields_none() {
        let home = unique_dir("no-bin-home");
        assert!(checkout_bin_under(&home).is_none());
    }

    #[test]
    fn checkout_bin_under_finds_a_real_file() {
        let home = unique_dir("has-bin-home");
        let bin = home
            .join("Development")
            .join("mockryx")
            .join("bin")
            .join("mockryx");
        write(&bin, "#!/bin/sh\nexit 0\n");

        let found = checkout_bin_under(&home).expect("must find the fixture binary");
        assert_eq!(found, bin);

        let _ = std::fs::remove_dir_all(&home);
    }

    // ---- scenario directory -------------------------------------------------

    #[test]
    fn env_scenario_dir_requires_a_non_blank_value() {
        assert!(env_scenario_dir_from(None).is_none());
        assert!(env_scenario_dir_from(Some(String::new())).is_none());

        let resolved = env_scenario_dir_from(Some("/custom/scenarios".to_string()))
            .expect("a non-blank path resolves");
        assert_eq!(resolved, PathBuf::from("/custom/scenarios"));
    }

    #[test]
    fn default_scenario_dir_never_panics_and_is_never_empty() {
        let path = default_scenario_dir();
        assert!(!path.as_os_str().is_empty());
    }

    // ---- gateway URL ----------------------------------------------------

    #[test]
    fn descriptor_gateway_in_resolves_the_gateway_service_url() {
        let dir = unique_dir("gw-happy");
        write(
            &dir.join("p1full.json"),
            r#"{
                "name": "p1full",
                "services": {
                    "gateway": {"url": "http://127.0.0.1:41000", "mode": "enforce"},
                    "cloud": {"url": "http://127.0.0.1:41001"}
                }
            }"#,
        );
        let url = descriptor_gateway_in(&dir).expect("must resolve the fixture gateway url");
        assert_eq!(url, "http://127.0.0.1:41000");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn descriptor_gateway_in_missing_gateway_service_falls_through() {
        let dir = unique_dir("gw-missing");
        write(
            &dir.join("broken.json"),
            r#"{"name":"broken","services":{"cloud":{"url":"http://x"}}}"#,
        );
        assert!(descriptor_gateway_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn descriptor_gateway_in_empty_directory_yields_none_not_a_panic() {
        let dir = unique_dir("gw-empty").join("nested");
        assert!(descriptor_gateway_in(&dir).is_none());
    }

    #[test]
    fn env_gateway_requires_a_non_blank_value() {
        assert!(env_gateway_from(None).is_none());
        assert!(env_gateway_from(Some(String::new())).is_none());
        assert_eq!(
            env_gateway_from(Some("http://127.0.0.1:4100".to_string())),
            Some("http://127.0.0.1:4100".to_string())
        );
    }

    // ---- api key ----------------------------------------------------------

    #[test]
    fn env_api_key_requires_a_non_blank_value() {
        assert!(env_api_key_from(None).is_none());
        assert!(env_api_key_from(Some(String::new())).is_none());
        assert_eq!(
            env_api_key_from(Some("sk_test".to_string())),
            Some("sk_test".to_string())
        );
    }

    // ---- save path ----------------------------------------------------------

    #[test]
    fn default_save_path_never_panics_and_is_never_empty() {
        let path = default_save_path();
        assert!(!path.as_os_str().is_empty());
    }
}
