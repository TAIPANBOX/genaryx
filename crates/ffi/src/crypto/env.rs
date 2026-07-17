//! Environment discovery for [`super::CryptoHandle`]: where to find the
//! `qryx` binary, plus a default scan target the operator can see and
//! override before running a scan.
//!
//! ## The `qryx` binary
//!
//! Qryx is a CLI with no `taipan up` HTTP service to register (unlike
//! Cloud/Wardryx/Idryx), so the only thing to resolve is a binary path. This
//! mirrors [`crate::idryx::env::locate_idryx_binary`]'s own
//! `~/.taipan/bin/<name>` convention for taipan-managed tool binaries
//! EXACTLY: a single well-known path, no env-var override. (Verdryx's
//! `crate::quality::env` added an env-var tier because PHASE4.md names
//! verdryx's OWN `VERDRYX_DB` env var explicitly; qryx has no equivalent
//! documented override, so this module does not invent one - see this
//! crate's own top-level report for this as a flagged, deliberate asymmetry
//! rather than an oversight.)
//!
//! ## The default scan target
//!
//! Unlike the binary, WHAT to scan is inherently operator/deployment
//! specific: qryx scans a directory tree (a stack checkout, a service repo,
//! ...), and nothing in a `taipan up` descriptor names one (`services`/
//! `events`/`keys` cover HTTP endpoints and event files, not a filesystem
//! root to inventory - see `crates/ffi/src/idryx/env.rs`'s own descriptor
//! shape). [`default_scan_target`] resolves, in order: [`QRYX_SCAN_ROOT_ENV_VAR`]
//! (an explicit operator/deployment override), else `$HOME`. `$HOME` is
//! deliberately the fallback rather than the console's own working directory
//! (which for a packaged app is inside its bundle, not anywhere useful to
//! scan): it is always resolvable and is at least in the right neighborhood
//! on an operator's own workstation. This is ALWAYS just a pre-filled
//! default, never enforced - docs/PHASE4.md W1: "operator can see/set it" -
//! so getting it exactly right matters far less than never leaving the field
//! blank.

use std::path::{Path, PathBuf};

/// Explicit override for [`default_scan_target`] - not a qryx-documented env
/// var (qryx itself takes its target as a CLI positional, `cmd/qryx/main.go`,
/// with zero `os.Getenv` per PHASE4.md's own grounding), but a genaryx-side
/// convenience so a deployment can pre-seed the console's default without the
/// operator retyping it every session.
const QRYX_SCAN_ROOT_ENV_VAR: &str = "QRYX_SCAN_ROOT";

/// Where a [`ResolvedEnv`] came from, surfaced to the Swift shell (06 §0.5).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CryptoEnvSource {
    /// The well-known `~/.taipan/bin/qryx`.
    Taipan,
    /// An operator-supplied binary path via [`super::CryptoHandle::connect`].
    Explicit,
}

/// A fully-resolved place to build a
/// [`genaryx_connectors::QryxClient`] against, plus the scan target to
/// pre-fill the operator's target field with.
#[derive(Debug, Clone)]
pub struct ResolvedEnv {
    pub source: CryptoEnvSource,
    pub qryx_bin: PathBuf,
    pub default_scan_target: PathBuf,
}

/// Resolve the Crypto panel's environment: the well-known qryx binary plus a
/// pre-filled scan target, or `None` for a clean "no crypto plane" state when
/// no `qryx` binary can be found anywhere this module knows to look.
#[must_use]
pub fn discover() -> Option<ResolvedEnv> {
    let qryx_bin = locate_qryx_binary()?;
    Some(ResolvedEnv {
        source: CryptoEnvSource::Taipan,
        qryx_bin,
        default_scan_target: default_scan_target(),
    })
}

/// `~/.taipan/bin/qryx`. `None` when `$HOME` is not set (never a panic over a
/// missing env var).
fn locate_qryx_binary() -> Option<PathBuf> {
    qryx_binary_under(&PathBuf::from(std::env::var_os("HOME")?))
}

/// Testable core of [`locate_qryx_binary`]: `home/.taipan/bin/qryx`, `None`
/// when nothing file-shaped exists there - mirrors
/// `crate::idryx::env::idryx_binary_under`'s own shape.
fn qryx_binary_under(home: &Path) -> Option<PathBuf> {
    let path = home.join(".taipan").join("bin").join("qryx");
    path.is_file().then_some(path)
}

/// [`QRYX_SCAN_ROOT_ENV_VAR`], else `$HOME`, else `.` - see the module doc's
/// "the default scan target". Always resolves to SOME path (unlike
/// `env::discover` above, this is a pre-filled suggestion, not a
/// presence/absence signal), so callers never see `None` here.
#[must_use]
pub fn default_scan_target() -> PathBuf {
    env_scan_root().unwrap_or_else(home_fallback)
}

/// Testable core of the env-var half of [`default_scan_target`], taking the
/// (already-read) value directly so tests never have to mutate real process
/// environment.
fn env_scan_root_from(value: Option<String>) -> Option<PathBuf> {
    let value = value?;
    if value.trim().is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

fn env_scan_root() -> Option<PathBuf> {
    env_scan_root_from(std::env::var(QRYX_SCAN_ROOT_ENV_VAR).ok())
}

/// `$HOME`, or `.` when even that is unset - the console's working directory
/// is a legitimate absolute-last-resort default (better than an empty
/// string), even though it is rarely the right long-lived answer.
fn home_fallback() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "genaryx-ffi-crypto-env-test-{tag}-{}-{n}",
            std::process::id()
        ))
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(path, body).expect("write fixture file");
    }

    // ---- qryx binary location -------------------------------------------

    #[test]
    fn qryx_binary_under_missing_home_yields_none() {
        let home = unique_dir("no-bin-home");
        assert!(qryx_binary_under(&home).is_none());
    }

    #[test]
    fn qryx_binary_under_finds_a_real_file() {
        let home = unique_dir("has-bin-home");
        let bin = home.join(".taipan").join("bin").join("qryx");
        write(&bin, "#!/bin/sh\nexit 0\n");

        let found = qryx_binary_under(&home).expect("must find the fixture binary");
        assert_eq!(found, bin);

        let _ = std::fs::remove_dir_all(&home);
    }

    // ---- default scan target ---------------------------------------------

    #[test]
    fn env_scan_root_requires_a_non_blank_value() {
        assert!(env_scan_root_from(None).is_none());
        assert!(env_scan_root_from(Some(String::new())).is_none());
        assert!(env_scan_root_from(Some("   ".to_string())).is_none());

        let resolved = env_scan_root_from(Some("/repo/checkout".to_string()))
            .expect("a non-blank root resolves");
        assert_eq!(resolved, PathBuf::from("/repo/checkout"));
    }

    #[test]
    fn home_fallback_is_always_some_path_never_empty() {
        // Whatever this box's real $HOME is (or the "." fallback if truly
        // unset), the result must be a non-empty path - never panics, never
        // an empty string a scan call would silently misinterpret.
        let path = home_fallback();
        assert!(!path.as_os_str().is_empty());
    }

    #[test]
    fn default_scan_target_never_panics_and_is_never_empty() {
        let path = default_scan_target();
        assert!(!path.as_os_str().is_empty());
    }
}
