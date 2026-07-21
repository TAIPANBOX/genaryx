//! Crypto-panel environment discovery: the `qryx` binary, plus a default
//! on-demand scan target.
//!
//! One tier only, unlike Quality/Identity/Policy's descriptor-then-fallback
//! shape: qryx has no `taipan up` service entry at all (it is a pure CLI
//! qryx invokes on demand, docs/PHASE4.md - "Quality/Verdryx, Crypto/Qryx,
//! Drills/Mockryx are pure CLIs"), so there is no descriptor to consult, only
//! the SAME well-known, fixed-location convention
//! `identity::state::resolve_idryx_bin` uses for `~/.taipan/bin/idryx`,
//! applied to `qryx`. An operator builds/installs qryx there (mirroring how
//! `taipan up --with idryx` itself populates `~/.taipan/bin/idryx`) for the
//! console to auto-discover it; nothing here ever builds or downloads the
//! binary itself.
//!
//! The default scan target has no grounded per-environment signal to derive
//! from either - no descriptor field names a codebase root, and qryx scans
//! an arbitrary filesystem path, not a "service" with a URL. It defaults to
//! the user's home directory, which the operator is expected to override
//! with the actual path they want scanned (docs/PHASE4.md W1: "let the
//! operator see/set the path").
//!
//! Never panics: every step is a `?`-chained `Option`.

use std::path::PathBuf;

/// A resolved place to run qryx scans from.
#[derive(Debug, Clone)]
pub struct ResolvedEnv {
    pub qryx_bin: PathBuf,
    /// A starting point for the on-demand scan path field, not a claim about
    /// where any particular TAIPANBOX checkout lives - see this module's doc
    /// comment.
    pub default_target: PathBuf,
}

/// `~/.taipan/bin/qryx`, best-effort - mirrors
/// `identity::state::resolve_idryx_bin` exactly, substituting `qryx` for
/// `idryx`. `None` when `$HOME` is unset or no file exists there; either way
/// the Crypto panel renders a clean "no crypto plane" state, never a panic
/// and never a guessed alternate path.
#[must_use]
pub fn discover() -> Option<ResolvedEnv> {
    let home = std::env::var_os("HOME")?;
    let home = PathBuf::from(home);
    let qryx_bin = home.join(".taipan").join("bin").join("qryx");
    if !qryx_bin.is_file() {
        return None;
    }
    Some(ResolvedEnv {
        qryx_bin,
        default_target: home,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_never_panics() {
        // Mirrors `identity::state::resolve_idryx_bin_never_panics`'s
        // identical rationale: this only proves the function resolves to a
        // consistent Option without panicking - whether this box actually
        // has ~/.taipan/bin/qryx depends on local dev state, not this test.
        let _ = discover();
    }

    #[test]
    fn a_resolved_env_points_the_bin_and_default_target_under_the_same_home() {
        if let Some(home) = std::env::var_os("HOME") {
            let expected_bin = PathBuf::from(&home)
                .join(".taipan")
                .join("bin")
                .join("qryx");
            match discover() {
                Some(env) => {
                    assert_eq!(env.qryx_bin, expected_bin);
                    assert_eq!(env.default_target, PathBuf::from(&home));
                }
                None => {
                    // Only asserts the shape discover() WOULD have produced
                    // had the file existed - never requires it to actually
                    // exist on this box.
                    assert!(!expected_bin.is_file());
                }
            }
        }
    }
}
