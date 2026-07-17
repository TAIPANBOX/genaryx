//! Environment discovery for [`super::QualityHandle`]: where to find
//! `verdryx.db`.
//!
//! Verdryx is not a `taipan up` service (it has no HTTP server to register a
//! `services.verdryx` descriptor entry - PHASE4.md: "Verdryx has NO
//! JSON/machine output on any subcommand... its durable, machine-readable
//! surface is its SQLite store"), so this module cannot mirror
//! [`crate::idryx::env`]'s descriptor-first lookup. Instead it tries three
//! candidates, most-explicit first, each one a plain, cheap, local
//! file-existence check (never a network call, matching every sibling `env`
//! module's "never touches the network" contract):
//!
//! 1. [`VERDRYX_DB_ENV_VAR`] (`VERDRYX_DB`) - verdryx's OWN documented CLI env
//!    var (PHASE4.md: "Env `VERDRYX_DB` (default `verdryx.db`)"), so an
//!    operator who already has verdryx configured this way is picked up for
//!    free. Honored even if the path does not (yet) exist: an explicit
//!    override should fail with an honest "can't open THIS path" error on the
//!    first real read, not silently fall through to a different store the
//!    operator did not ask for.
//! 2. The well-known `~/.taipan/verdryx.db` - the location a
//!    `taipan up --with verdryx` is expected to populate, mirroring
//!    [`crate::idryx::env::locate_idryx_binary`]'s own `~/.taipan/bin/idryx`
//!    convention for taipan-managed artifacts. Only reported when the file is
//!    actually there (an unchecked guess would be indistinguishable from a
//!    real taipan-managed store).
//! 3. `./verdryx.db`, relative to the console's own working directory -
//!    verdryx's own CLI default (`verdryx/store.py`'s "default `verdryx.db`"),
//!    so a console launched from inside a stack checkout that already ran
//!    `verdryx eval` picks it up with zero configuration.
//!
//! No candidate resolving is not an error: [`discover`] returns `None` and
//! [`super::QualityHandle::discover`] fails closed with
//! `QualityError::NoEnvironment` - PHASE3.md's "No-idryx environment renders a
//! clean empty state, not an error" applies equally to "no quality plane"
//! here (docs/PHASE4.md W1: "An absent source... must render as an honest
//! first-class empty state").

use std::path::{Path, PathBuf};

/// Verdryx's own CLI env var (PHASE4.md: "Env `VERDRYX_DB` (default
/// `verdryx.db`)") - carries the path itself, mirroring
/// [`crate::idryx::env::IDRYX_URL_ENV_VAR`]'s "the env var IS the value"
/// idiom rather than a secret gating a fixed default.
const VERDRYX_DB_ENV_VAR: &str = "VERDRYX_DB";

/// verdryx's own CLI default filename (`verdryx/store.py`), used for both the
/// well-known-taipan-directory and working-directory candidates below.
const VERDRYX_DB_FILENAME: &str = "verdryx.db";

/// Where a [`ResolvedEnv`] came from, surfaced to the Swift shell (06 §0.5),
/// exported as a UniFFI enum. Named distinctly from
/// [`crate::idryx::env::IdryxEnvSource`] (rather than reused) for the same
/// flat-per-crate-namespace reason that type's own doc comment gives.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum QualityEnvSource {
    /// [`VERDRYX_DB_ENV_VAR`], or an operator-supplied path via
    /// [`super::QualityHandle::connect`] (which always reports this variant
    /// too - mirrors `IdryxEnvSource::EnvFallback`'s own dual use).
    Explicit,
    /// The well-known `~/.taipan/verdryx.db`.
    Taipan,
    /// `./verdryx.db`, relative to the console's working directory.
    WorkingDirectory,
}

/// A fully-resolved place to open a [`genaryx_connectors::VerdryxClient`]
/// against.
#[derive(Debug, Clone)]
pub struct ResolvedEnv {
    pub source: QualityEnvSource,
    pub db_path: PathBuf,
}

/// Resolve the Quality panel's environment: [`VERDRYX_DB_ENV_VAR`], then the
/// well-known taipan path, then verdryx's own cwd-relative default, or `None`
/// for a clean "no quality plane" state. See the module doc for the full
/// rationale behind this order.
#[must_use]
pub fn discover() -> Option<ResolvedEnv> {
    env_var_default()
        .or_else(well_known_default)
        .or_else(working_directory_default)
}

/// [`VERDRYX_DB_ENV_VAR`]. `None` when unset or blank; otherwise `Some`
/// regardless of whether the named path currently exists (see the module
/// doc's point 1).
fn env_var_default() -> Option<ResolvedEnv> {
    env_var_from(std::env::var(VERDRYX_DB_ENV_VAR).ok())
}

/// Testable core of [`env_var_default`], taking the (already-read) env var
/// value directly so tests never have to mutate real process environment -
/// mirrors [`crate::idryx::env::env_fallback_from`]'s own rationale.
fn env_var_from(value: Option<String>) -> Option<ResolvedEnv> {
    let value = value?;
    if value.trim().is_empty() {
        return None;
    }
    Some(ResolvedEnv {
        source: QualityEnvSource::Explicit,
        db_path: PathBuf::from(value),
    })
}

/// `~/.taipan/verdryx.db`, only when a real file is there. `None` when
/// `$HOME` is unset or the file does not (yet) exist.
fn well_known_default() -> Option<ResolvedEnv> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    well_known_path_under(&home).map(|db_path| ResolvedEnv {
        source: QualityEnvSource::Taipan,
        db_path,
    })
}

/// Testable core of [`well_known_default`]: `home/.taipan/verdryx.db`, `None`
/// when nothing file-shaped exists there - mirrors
/// [`crate::idryx::env::idryx_binary_under`]'s own shape.
fn well_known_path_under(home: &Path) -> Option<PathBuf> {
    let path = home.join(".taipan").join(VERDRYX_DB_FILENAME);
    path.is_file().then_some(path)
}

/// `./verdryx.db`, only when a real file is there. `None` when the current
/// directory cannot be read or the file does not (yet) exist.
fn working_directory_default() -> Option<ResolvedEnv> {
    let cwd = std::env::current_dir().ok()?;
    cwd_default_in(&cwd).map(|db_path| ResolvedEnv {
        source: QualityEnvSource::WorkingDirectory,
        db_path,
    })
}

/// Testable core of [`working_directory_default`], taking the (already-read)
/// directory directly so tests never have to `std::env::set_current_dir`
/// (which is racy under `cargo test`'s parallel-by-default threads, the same
/// reason every other candidate here has a `_under`/`_from` testable core
/// rather than touching real process state).
fn cwd_default_in(cwd: &Path) -> Option<PathBuf> {
    let path = cwd.join(VERDRYX_DB_FILENAME);
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "genaryx-ffi-quality-env-test-{tag}-{}-{n}",
            std::process::id()
        ))
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(path, body).expect("write fixture file");
    }

    // ---- env var ------------------------------------------------------

    #[test]
    fn env_var_requires_a_non_blank_value() {
        assert!(env_var_from(None).is_none());
        assert!(env_var_from(Some(String::new())).is_none());
        assert!(env_var_from(Some("   ".to_string())).is_none());

        let resolved = env_var_from(Some("/custom/verdryx.db".to_string()))
            .expect("a non-blank path resolves");
        assert_eq!(resolved.source, QualityEnvSource::Explicit);
        assert_eq!(resolved.db_path, PathBuf::from("/custom/verdryx.db"));
    }

    #[test]
    fn env_var_is_honored_even_when_the_path_does_not_exist_yet() {
        // Deliberately not creating this file: an explicit override must be
        // reported as-is (see the module doc's point 1), not silently
        // skipped for a different candidate.
        let resolved = env_var_from(Some("/definitely/not/a/real/verdryx.db".to_string()))
            .expect("explicit override resolves regardless of existence");
        assert_eq!(resolved.source, QualityEnvSource::Explicit);
    }

    // ---- well-known taipan path -----------------------------------------

    #[test]
    fn well_known_path_under_missing_home_yields_none() {
        let home = unique_dir("no-db-home");
        assert!(well_known_path_under(&home).is_none());
    }

    #[test]
    fn well_known_path_under_finds_a_real_file() {
        let home = unique_dir("has-db-home");
        let db = home.join(".taipan").join("verdryx.db");
        write(&db, "not a real sqlite file, just proving existence");

        let found = well_known_path_under(&home).expect("must find the fixture db");
        assert_eq!(found, db);

        let _ = std::fs::remove_dir_all(&home);
    }

    // ---- working-directory default ---------------------------------------

    #[test]
    fn cwd_default_in_missing_dir_yields_none() {
        let dir = unique_dir("no-cwd-db");
        assert!(cwd_default_in(&dir).is_none());
    }

    #[test]
    fn cwd_default_in_finds_a_real_file() {
        let dir = unique_dir("has-cwd-db");
        let db = dir.join("verdryx.db");
        write(&db, "not a real sqlite file, just proving existence");

        let found = cwd_default_in(&dir).expect("must find the fixture db");
        assert_eq!(found, db);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- discover() precedence -------------------------------------------

    /// A more-explicit candidate (env var) must win over a less-explicit one
    /// (well-known path) even when both would resolve - proven at the
    /// `_from`/`_under` level since `discover()` itself reads real process
    /// state (`$HOME`, cwd) that these tests must not mutate.
    #[test]
    fn env_var_takes_precedence_when_present() {
        let resolved = env_var_from(Some("/explicit/verdryx.db".to_string()))
            .expect("env var resolves")
            .source;
        assert_eq!(resolved, QualityEnvSource::Explicit);
    }
}
