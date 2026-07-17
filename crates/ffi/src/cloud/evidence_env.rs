//! Best-effort environment resolution for the Evidence Center's optional
//! sources (docs/PHASE4.md W3): where to find the qryx/idryx/tokenfuse
//! binaries plus their targets, so
//! [`super::CloudHandle::evidence_env_defaults`] can pre-fill the SwiftUI
//! panel's editable fields exactly like
//! `CryptoHandle::default_scan_target`/`DrillsHandle::default_scenario_dir`
//! do for their own panels ("operator can see/set it, never enforced").
//!
//! Deliberately REUSES the existing crypto/idryx resolution rather than
//! re-implementing `~/.taipan/bin/<name>` lookups a third time:
//! [`crate::crypto::env::discover`] for qryx (bin + a default scan target)
//! and [`crate::idryx::env::resolve_rescan_inputs`] for idryx (bin + the
//! exact `--load source:path` pairs Rescan already computes - Agent-BOM
//! takes the identical shape, `IdryxClient::agent_bom`/`IdryxClient::rescan`
//! share one `(source, path)` contract). Only the TokenFuse binary + FOCUS
//! traces directory are genuinely new: TokenFuse has no existing FFI handle
//! in this crate, so this module resolves both fresh, grounded in the
//! sibling `taipan` repo's own install convention
//! (`~/Development/taipan/src/services/tokenfuse_build.rs`: `taipan up`
//! builds `cargo build --release -p tokenfuse-gateway`'s `target/release/
//! tokenfuse` and copies it to `~/.taipan/bin/tokenfuse-gateway`;
//! `~/Development/taipan/src/home.rs::traces_dir` writes FOCUS call traces to
//! `~/.taipan/environments/<name>.traces/gateway` - a SIBLING DIRECTORY
//! convention, confirmed NOT a descriptor JSON field, unlike every other
//! `services.*`/`events.*` lookup this crate's `env.rs` modules already
//! parse).
//!
//! Every resolution here is best-effort and independent: an unresolved piece
//! is `None`/an empty `Vec`, never a panic and never a reason to fail the
//! other pieces - mirrors [`super::EvidenceBuildInputs`]'s own "a source
//! whose path is `None` is simply left out" contract one level up.

use super::evidence::EvidenceLoadEntry;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Everything [`super::CloudHandle::evidence_env_defaults`] hands back in one
/// round trip - bundled rather than six separate getters (unlike
/// `CryptoHandle`/`DrillsHandle`'s many small `default_*` methods) because
/// these six pieces are all resolved together for ONE panel's initial
/// pre-fill, and a single call keeps that from being six near-simultaneous
/// FFI round trips at connect time for what is conceptually one operation.
#[derive(Debug, Clone, uniffi::Record)]
pub struct EvidenceEnvDefaultsRecord {
    /// `~/.taipan/bin/qryx`, or `None` when not found.
    pub qryx_bin: Option<String>,
    /// Always resolves to SOME path (mirrors
    /// `crypto::env::default_scan_target`'s own "never `None`" contract).
    pub qryx_scan_target: String,
    /// `~/.taipan/bin/idryx`, or `None` when not found.
    pub idryx_bin: Option<String>,
    /// The same stack-bus `--load source:path` pairs `IdryxHandle::rescan`
    /// resolves; empty when no taipan environment with an idryx service was
    /// found (a legitimate, honest empty state, not an error - see
    /// [`crate::idryx::env::resolve_rescan_inputs`]'s own doc).
    pub idryx_loads: Vec<EvidenceLoadEntry>,
    /// `~/.taipan/bin/tokenfuse-gateway`, or a sibling checkout's own build
    /// output; `None` when neither is found.
    pub tokenfuse_bin: Option<String>,
    /// `~/.taipan/environments/<name>.traces/gateway`; `None` when no
    /// environment resolves or its traces directory was never created (no
    /// gateway calls recorded yet).
    pub tokenfuse_traces_dir: Option<String>,
}

/// Resolve every optional source's defaults, independently and best-effort -
/// see the module doc.
#[must_use]
pub fn resolve_defaults() -> EvidenceEnvDefaultsRecord {
    let (qryx_bin, qryx_scan_target) = resolve_qryx();
    let (idryx_bin, idryx_loads) = resolve_idryx();

    EvidenceEnvDefaultsRecord {
        qryx_bin,
        qryx_scan_target,
        idryx_bin,
        idryx_loads,
        tokenfuse_bin: default_tokenfuse_bin().map(|p| p.display().to_string()),
        tokenfuse_traces_dir: default_tokenfuse_traces_dir().map(|p| p.display().to_string()),
    }
}

/// Reuses [`crate::crypto::env::discover`] for the binary (`None` when not
/// found) and [`crate::crypto::env::default_scan_target`] for the target
/// (always resolves) - see the module doc.
fn resolve_qryx() -> (Option<String>, String) {
    let qryx_bin = crate::crypto::env::discover().map(|r| r.qryx_bin.display().to_string());
    let qryx_scan_target = crate::crypto::env::default_scan_target()
        .display()
        .to_string();
    (qryx_bin, qryx_scan_target)
}

/// Reuses [`crate::idryx::env::resolve_rescan_inputs`] verbatim - Agent-BOM
/// needs the exact same `(idryx_bin, loads)` shape Rescan already resolves
/// (both shell `idryx` over the same stack-bus NDJSON files). `None`/empty
/// when unresolved (never a panic, never surfacing the honest `Err` reason
/// `IdryxHandle::rescan` itself would - this is a pre-fill default, not a
/// gating call, mirroring `CryptoHandle`/`DrillsHandle`'s own "always
/// resolves to SOME state" default getters).
fn resolve_idryx() -> (Option<String>, Vec<EvidenceLoadEntry>) {
    match crate::idryx::env::resolve_rescan_inputs() {
        Ok(inputs) => {
            let bin = Some(inputs.idryx_bin.display().to_string());
            let loads = inputs
                .loads
                .into_iter()
                .map(|(source, path)| EvidenceLoadEntry { source, path })
                .collect();
            (bin, loads)
        }
        Err(_) => (None, Vec::new()),
    }
}

/// `~/.taipan/bin/tokenfuse-gateway` - see the module doc. Falls back to a
/// sibling `~/Development/tokenfuse` checkout's own build output (release,
/// then debug; the crate's own `[[bin]] name = "tokenfuse"` in
/// `crates/gateway/Cargo.toml`), mirroring `drills::env::checkout_bin`'s own
/// checkout-fallback shape.
fn default_tokenfuse_bin() -> Option<PathBuf> {
    tokenfuse_bin_under(&PathBuf::from(std::env::var_os("HOME")?))
}

/// Testable core of [`default_tokenfuse_bin`].
fn tokenfuse_bin_under(home: &Path) -> Option<PathBuf> {
    let installed = home.join(".taipan").join("bin").join("tokenfuse-gateway");
    if installed.is_file() {
        return Some(installed);
    }
    let target = home.join("Development").join("tokenfuse").join("target");
    let release = target.join("release").join("tokenfuse");
    if release.is_file() {
        return Some(release);
    }
    let debug = target.join("debug").join("tokenfuse");
    debug.is_file().then_some(debug)
}

/// `~/.taipan/environments/<name>.traces/gateway` for the newest descriptor -
/// see the module doc. `None` when no environment resolves or the directory
/// was never created (no gateway calls recorded yet).
fn default_tokenfuse_traces_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    tokenfuse_traces_dir_in(&PathBuf::from(home).join(".taipan").join("environments"))
}

/// Testable core of [`default_tokenfuse_traces_dir`].
fn tokenfuse_traces_dir_in(environments_dir: &Path) -> Option<PathBuf> {
    let name = newest_environment_name(environments_dir)?;
    let dir = environments_dir
        .join(format!("{name}.traces"))
        .join("gateway");
    dir.is_dir().then_some(dir)
}

#[derive(Debug, Deserialize)]
struct Descriptor {
    name: String,
}

/// The `name` field of the most-recently-modified `<name>.json` descriptor in
/// `dir`, skipping the sibling `<name>.keys.json`/`<name>.pid.json` files -
/// mirrors `cloud::env::list_descriptor_paths`'s own scan exactly (a
/// deliberate small duplication, matching this crate's established
/// independent-evolution-over-shared-abstraction convention - see
/// `idryx::env`'s own module doc). Every descriptor always carries a
/// `services.gateway` entry (the mandatory gateway/cloud pair never degrades
/// silently, per `taipan`'s own `descriptor.rs`), so unlike
/// `idryx::env::newest_descriptor_with_idryx` this does not need to filter
/// on a specific service key.
fn newest_environment_name(dir: &Path) -> Option<String> {
    let mut candidates = list_descriptor_paths(dir);
    candidates.sort_by_key(|p| std::cmp::Reverse(modified_time(p)));
    candidates.into_iter().find_map(|p| {
        let bytes = std::fs::read(&p).ok()?;
        let descriptor: Descriptor = serde_json::from_slice(&bytes).ok()?;
        Some(descriptor.name)
    })
}

/// Every `<name>.json` descriptor in `dir`, excluding the sibling
/// `<name>.keys.json`/`<name>.pid.json` files - mirrors
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "genaryx-ffi-evidence-env-test-{tag}-{}-{n}",
            std::process::id()
        ))
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(path, body).expect("write fixture file");
    }

    /// Rust-side stand-in proving `resolve_defaults` never panics regardless
    /// of this box's real `$HOME`/`~/.taipan` state - deliberately does not
    /// control either (mirrors
    /// `idryx::tests::rescan_never_panics_regardless_of_environment`'s own
    /// rationale). Only `qryx_scan_target` is asserted on (it is the one
    /// field documented to always resolve to something); every other field
    /// legitimately varies by box.
    #[test]
    fn resolve_defaults_never_panics_and_qryx_scan_target_always_resolves() {
        let defaults = resolve_defaults();
        assert!(!defaults.qryx_scan_target.is_empty());
    }

    // ---- tokenfuse binary location -----------------------------------

    #[test]
    fn tokenfuse_bin_under_missing_home_yields_none() {
        let home = unique_dir("no-bin-home");
        assert!(tokenfuse_bin_under(&home).is_none());
    }

    #[test]
    fn tokenfuse_bin_under_finds_the_installed_binary() {
        let home = unique_dir("has-installed");
        let bin = home.join(".taipan").join("bin").join("tokenfuse-gateway");
        write(&bin, "#!/bin/sh\nexit 0\n");

        assert_eq!(tokenfuse_bin_under(&home), Some(bin));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn tokenfuse_bin_under_falls_back_to_a_checkout_release_build() {
        let home = unique_dir("has-checkout-release");
        let bin = home
            .join("Development")
            .join("tokenfuse")
            .join("target")
            .join("release")
            .join("tokenfuse");
        write(&bin, "#!/bin/sh\nexit 0\n");

        assert_eq!(tokenfuse_bin_under(&home), Some(bin));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn tokenfuse_bin_under_falls_back_to_a_checkout_debug_build_last() {
        let home = unique_dir("has-checkout-debug");
        let bin = home
            .join("Development")
            .join("tokenfuse")
            .join("target")
            .join("debug")
            .join("tokenfuse");
        write(&bin, "#!/bin/sh\nexit 0\n");

        assert_eq!(tokenfuse_bin_under(&home), Some(bin));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn tokenfuse_bin_under_prefers_the_installed_binary_over_a_checkout() {
        let home = unique_dir("prefers-installed");
        let installed = home.join(".taipan").join("bin").join("tokenfuse-gateway");
        write(&installed, "#!/bin/sh\nexit 0\n");
        let checkout = home
            .join("Development")
            .join("tokenfuse")
            .join("target")
            .join("release")
            .join("tokenfuse");
        write(&checkout, "#!/bin/sh\nexit 0\n");

        assert_eq!(tokenfuse_bin_under(&home), Some(installed));
        let _ = std::fs::remove_dir_all(&home);
    }

    // ---- tokenfuse traces dir -----------------------------------------

    #[test]
    fn tokenfuse_traces_dir_in_missing_directory_yields_none_not_a_panic() {
        let dir = unique_dir("traces-missing").join("nested");
        assert!(tokenfuse_traces_dir_in(&dir).is_none());
    }

    #[test]
    fn tokenfuse_traces_dir_in_resolves_the_newest_environments_traces_dir() {
        let dir = unique_dir("traces-happy");
        write(&dir.join("env1.json"), r#"{"name":"env1"}"#);
        let traces = dir.join("env1.traces").join("gateway");
        std::fs::create_dir_all(&traces).expect("create fixture traces dir");

        assert_eq!(tokenfuse_traces_dir_in(&dir), Some(traces));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tokenfuse_traces_dir_in_a_directory_that_was_never_created_is_none() {
        let dir = unique_dir("traces-not-created");
        write(&dir.join("env1.json"), r#"{"name":"env1"}"#);
        // Deliberately no `env1.traces/gateway` directory created - no
        // gateway calls have been recorded yet for this environment.

        assert!(tokenfuse_traces_dir_in(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tokenfuse_traces_dir_in_picks_the_newest_environment() {
        let dir = unique_dir("traces-newest");
        write(&dir.join("older.json"), r#"{"name":"older"}"#);
        std::fs::create_dir_all(dir.join("older.traces").join("gateway"))
            .expect("create older traces dir");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(&dir.join("newer.json"), r#"{"name":"newer"}"#);
        let newer_traces = dir.join("newer.traces").join("gateway");
        std::fs::create_dir_all(&newer_traces).expect("create newer traces dir");

        assert_eq!(tokenfuse_traces_dir_in(&dir), Some(newer_traces));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- newest_environment_name ---------------------------------------

    #[test]
    fn newest_environment_name_missing_directory_yields_none_not_a_panic() {
        let dir = unique_dir("name-missing").join("nested");
        assert!(newest_environment_name(&dir).is_none());
    }

    #[test]
    fn newest_environment_name_resolves_the_newest_descriptors_name() {
        let dir = unique_dir("name-happy");
        write(&dir.join("older.json"), r#"{"name":"older"}"#);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(&dir.join("newer.json"), r#"{"name":"newer"}"#);

        assert_eq!(newest_environment_name(&dir), Some("newer".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
