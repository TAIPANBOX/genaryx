//! Routines-plane environment discovery: where stack-up's `routines.sh`
//! keeps its recorded runs.
//!
//! Resolution order, matching `routines.sh`'s own constants verbatim
//! (`~/Development/stack-up/routines.sh`:
//! `STACK_UP_HOME="${STACK_UP_HOME:-$HOME/.stack-up}"`,
//! `ROUTINES_DIR="$STACK_UP_HOME/routines"`):
//!
//! 1. `$STACK_UP_HOME/routines`, when `STACK_UP_HOME` is set (an explicit
//!    override - the same variable `routines.sh` itself honors, so a console
//!    pointed at a scratch install and a `routines.sh` pointed at the SAME
//!    scratch install agree on where to look, exactly the reasoning
//!    `genaryx_core::taipan_home`'s own doc comment gives for honoring
//!    `TAIPAN_HOME`).
//! 2. `~/.stack-up/routines` otherwise - `routines.sh`'s own default.
//!
//! This is deliberately NOT `genaryx_core::taipan_home` and consults no
//! `taipan up` descriptor at all: routines is a stack-up concept
//! (`$STACK_UP_HOME`), not a taipan-up plane (`$TAIPAN_HOME`) - the two home
//! directories are siblings on disk, not the same thing, and reading the
//! wrong one would point this plane at a directory `routines.sh` never
//! writes to.
//!
//! Never fails: [`discover`] always returns a resolved path (falling back to
//! a relative `.stack-up/routines` on the vanishingly rare box where even
//! `HOME` is unset, mirroring `crate::onboard::commands::resolve_passports_dir`'s
//! identical last resort) plus an honest `exists` flag. "The directory is
//! not there yet" is this plane's normal, expected state right after a fresh
//! `stack-up` clone, before `routines.sh run`/`install` has ever executed -
//! never an error.

use std::ffi::OsString;
use std::path::PathBuf;

/// The resolved routines directory, plus whether it actually exists yet.
#[derive(Debug, Clone)]
pub struct ResolvedRoutinesDir {
    pub path: PathBuf,
    pub exists: bool,
}

/// Resolve `$STACK_UP_HOME/routines` (or its default) and report whether it
/// exists - the one entry point [`super::commands`] calls.
#[must_use]
pub fn discover() -> ResolvedRoutinesDir {
    let path = resolve_dir(std::env::var_os("STACK_UP_HOME"), std::env::var_os("HOME"));
    let exists = path.is_dir();
    ResolvedRoutinesDir { path, exists }
}

/// Testable core of [`discover`]'s path resolution, parameterized over the
/// two environment variables it reads so tests never have to mutate real
/// process-wide env vars (which run in parallel under Rust's default test
/// harness and would make such a test flaky) - callers construct the two
/// `Option<OsString>` inputs directly instead.
///
/// An empty `STACK_UP_HOME` counts as absent, exactly as `routines.sh`'s own
/// `"${STACK_UP_HOME:-$HOME/.stack-up}"` treats it: the bash `:-` form falls
/// back to the default when the variable is unset OR null. Honoring an empty
/// value here instead would resolve to a cwd-relative `routines` while
/// `routines.sh` resolves to `$HOME/.stack-up/routines`, so the console and
/// the script would disagree on where to look - the one thing this plane
/// exists to keep aligned.
fn resolve_dir(stack_up_home: Option<OsString>, home: Option<OsString>) -> PathBuf {
    if let Some(dir) = stack_up_home.filter(|d| !d.is_empty()) {
        return PathBuf::from(dir).join("routines");
    }
    match home.filter(|h| !h.is_empty()) {
        Some(home) => PathBuf::from(home).join(".stack-up").join("routines"),
        None => PathBuf::from(".stack-up").join("routines"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_stack_up_home_wins_even_when_home_is_also_set() {
        let path = resolve_dir(
            Some(OsString::from("/scratch/stack-up-home")),
            Some(OsString::from("/home/someone")),
        );
        assert_eq!(path, PathBuf::from("/scratch/stack-up-home/routines"));
    }

    #[test]
    fn falls_back_to_home_dot_stack_up_when_no_override_is_set() {
        let path = resolve_dir(None, Some(OsString::from("/home/someone")));
        assert_eq!(path, PathBuf::from("/home/someone/.stack-up/routines"));
    }

    #[test]
    fn falls_back_to_a_relative_default_when_neither_is_set() {
        let path = resolve_dir(None, None);
        assert_eq!(path, PathBuf::from(".stack-up/routines"));
    }

    #[test]
    fn an_empty_stack_up_home_override_falls_through_to_home() {
        // An empty `STACK_UP_HOME` counts as absent, matching `routines.sh`'s
        // own `"${STACK_UP_HOME:-$HOME/.stack-up}"`: bash's `:-` falls back to
        // the default when the variable is unset OR null. Honoring the empty
        // value instead would resolve here to a cwd-relative `routines` while
        // `routines.sh` resolves to `$HOME/.stack-up/routines`, so the console
        // and the script would look in different places.
        let path = resolve_dir(Some(OsString::new()), Some(OsString::from("/home/someone")));
        assert_eq!(path, PathBuf::from("/home/someone/.stack-up/routines"));
    }

    #[test]
    fn an_empty_home_with_no_override_falls_through_to_the_relative_default() {
        // The same null-is-absent reasoning applied to `HOME`: an empty `HOME`
        // must not produce an absolute `/.stack-up/routines` rooted at the
        // filesystem root, which is neither what bash's `:-` yields nor a path
        // this plane could sensibly read.
        let path = resolve_dir(None, Some(OsString::new()));
        assert_eq!(path, PathBuf::from(".stack-up/routines"));
    }

    #[test]
    fn discover_never_panics_and_reports_existence_honestly() {
        let resolved = discover();
        // Only proves this resolves to something renderable and that `exists`
        // agrees with a direct filesystem check against that SAME path - same
        // rationale as `quality::env`'s `discover_never_panics` test.
        assert_eq!(resolved.exists, resolved.path.is_dir());
    }
}
