//! Environment discovery for [`super::MemoryHandle`]: where to find the
//! `engram-mcp` binary, plus (independently) the engram SQLite store to
//! point it at.
//!
//! ## The `engram-mcp` binary: no `taipan up`-managed install yet
//!
//! Unlike Idryx/Qryx (installed to the well-known `~/.taipan/bin/<name>` by
//! `taipan up --with <name>`), engram-mcp is a Python console script
//! (docs/PHASE4.md W2: "a `engram-mcp` on PATH / a venv" is how the binary is
//! found) - there is no equivalent taipan-managed location for it yet. So
//! [`discover_bin`] tries two tiers:
//!
//! 1. The well-known `~/.taipan/bin/engram-mcp` anyway - IF `taipan up` ever
//!    grows an `--with engram` install step, this starts working with zero
//!    changes here, exactly like every other tool this crate resolves.
//! 2. A `$PATH` scan for a file literally named `engram-mcp` - this is the
//!    normal way a `pip install`'d or activated-venv console script becomes
//!    reachable without hardcoding a location, and is the one docs/PHASE4.md
//!    itself names.
//!
//! No env-var override tier here (unlike [`crate::quality::env`]'s
//! `VERDRYX_DB` or this very module's own [`ENGRAM_MCP_DB_ENV_VAR`] below):
//! engram-mcp has no documented env var for ITS OWN binary path (only for its
//! db/agent-id/events), so this module does not invent one - mirrors
//! [`crate::crypto::env`]'s own documented asymmetry ("qryx has no
//! equivalent documented override, so this module does not invent one").
//! [`super::MemoryHandle::connect`] remains the escape hatch for an operator
//! who knows a binary path this module's two tiers do not find.
//!
//! ## The engram db path: always resolves, never gates readiness
//!
//! `db_path` is baked into [`genaryx_connectors::EngramClient::spawn`] at
//! construction time (unlike Qryx's per-call scan target), but it is NOT a
//! presence/absence signal the way the binary is: `engram-mcp --db <path>`
//! happily creates a fresh store at a path that does not exist yet (normal
//! SQLite semantics), so requiring the file to pre-exist would wrongly block
//! a legitimate first run. [`default_db_path`] therefore always resolves to
//! SOME real (non-`:memory:`) path - a pre-filled default, mirroring
//! [`crate::crypto::env::default_scan_target`]'s own "always resolves, never
//! `None`, never enforced" contract - rather than a second
//! [`super::dto::MemoryError::NoEnvironment`] gate. A path that turns out to
//! be genuinely unusable (bad permissions, missing parent directory) still
//! surfaces honestly: [`genaryx_connectors::EngramClient::spawn`] fails and
//! [`super::MemoryHandle::discover`] reports that as
//! [`super::dto::MemoryError::Spawn`]/[`super::dto::MemoryError::Io`], never
//! silently swallowed.
//!
//! Three tiers, most-explicit first:
//!
//! 1. [`ENGRAM_MCP_DB_ENV_VAR`] (`ENGRAM_MCP_DB`) - engram-mcp's OWN
//!    documented env var (docs/PHASE4.md: "env `ENGRAM_MCP_DB` = the
//!    `.engram` SQLite path"). Reused here purely as a DISCOVERY convenience,
//!    the same way [`crate::quality::env`]'s `VERDRYX_DB_ENV_VAR` reuses
//!    verdryx's own name: the resolved value flows into an explicit `--db`
//!    flag on spawn (never inherited environment - `EngramClient::spawn`'s
//!    own doc: "everything engram-mcp needs is passed as an explicit flag...
//!    never smuggled through inherited env"), so reading it here does not
//!    reopen that door. Honored even when the path does not exist yet (see
//!    above).
//! 2. The well-known `~/.taipan/engram.engram` - `engram`'s own CLI names its
//!    store files with a literal `.engram` extension (`engram observe <path>
//!    <content>`'s own `--help`: "path to .engram file"; verified against a
//!    real `~/Development/engram` checkout), so this mirrors that convention
//!    rather than inventing a `.db` suffix that would not match what an
//!    operator sees from the `engram` CLI itself.
//! 3. `./engram.engram`, relative to the console's working directory -
//!    mirrors [`crate::quality::env`]'s own cwd-relative fallback, in case
//!    the console is launched from inside a directory that already has one.
//!
//! Agent scope ([`agent_id`]) is simpler still: engram-mcp's own
//! `ENGRAM_MCP_AGENT_ID` (docs/PHASE4.md), read as a plain optional value -
//! no file-based resolution, no gating; `None` just means "the server's own
//! default agent scope" ([`genaryx_connectors::EngramClient::stats`]'s own
//! doc).

use std::path::{Path, PathBuf};

/// engram-mcp's own documented db-path env var (docs/PHASE4.md: "env
/// `ENGRAM_MCP_DB`") - see the module doc's "always resolves" section.
const ENGRAM_MCP_DB_ENV_VAR: &str = "ENGRAM_MCP_DB";
/// engram-mcp's own documented agent-scope env var (docs/PHASE4.md).
const ENGRAM_MCP_AGENT_ID_ENV_VAR: &str = "ENGRAM_MCP_AGENT_ID";
/// The `.engram`-suffixed filename convention the `engram` CLI itself uses -
/// see the module doc.
const ENGRAM_DB_FILENAME: &str = "engram.engram";
const ENGRAM_MCP_BIN_NAME: &str = "engram-mcp";

/// Where a [`ResolvedBin`] came from, surfaced to the Swift shell (06 §0.5),
/// exported as a UniFFI enum. Named distinctly from every sibling
/// `*EnvSource` (rather than reused) for the same flat-per-crate-namespace
/// reason `crate::idryx::env::IdryxEnvSource`'s own doc comment gives.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum MemoryEnvSource {
    /// The well-known `~/.taipan/bin/engram-mcp`.
    Taipan,
    /// Found on `$PATH` (covers an activated venv's own `bin/` directory).
    PathEnv,
    /// An operator-supplied path via [`super::MemoryHandle::connect`].
    Explicit,
}

/// A resolved `engram-mcp` binary path plus where it came from.
#[derive(Debug, Clone)]
pub struct ResolvedBin {
    pub source: MemoryEnvSource,
    pub bin: PathBuf,
}

/// Resolve the `engram-mcp` binary: the well-known taipan path, then a
/// `$PATH` scan, or `None` for a clean "no memory plane" state - see the
/// module doc.
#[must_use]
pub fn discover_bin() -> Option<ResolvedBin> {
    taipan_bin().or_else(path_bin)
}

/// `~/.taipan/bin/engram-mcp`, only when a real file is there. `None` when
/// `$HOME` is unset or nothing is there yet.
fn taipan_bin() -> Option<ResolvedBin> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    taipan_bin_under(&home).map(|bin| ResolvedBin {
        source: MemoryEnvSource::Taipan,
        bin,
    })
}

/// Testable core of [`taipan_bin`]: `home/.taipan/bin/engram-mcp`, `None`
/// when nothing file-shaped exists there - mirrors
/// `crate::idryx::env::idryx_binary_under`'s own shape.
fn taipan_bin_under(home: &Path) -> Option<PathBuf> {
    let path = home.join(".taipan").join("bin").join(ENGRAM_MCP_BIN_NAME);
    path.is_file().then_some(path)
}

/// A `$PATH` scan for `engram-mcp`, first directory wins. `None` when `$PATH`
/// is unset or no directory on it has the binary.
fn path_bin() -> Option<ResolvedBin> {
    path_bin_from(std::env::var_os("PATH")?).map(|bin| ResolvedBin {
        source: MemoryEnvSource::PathEnv,
        bin,
    })
}

/// Testable core of [`path_bin`], taking the (already-read) `$PATH` value
/// directly so tests never have to mutate real process environment - mirrors
/// every other `_from`/`_under` testable core in this crate.
fn path_bin_from(path_var: std::ffi::OsString) -> Option<PathBuf> {
    std::env::split_paths(&path_var).find_map(|dir| {
        let candidate = dir.join(ENGRAM_MCP_BIN_NAME);
        candidate.is_file().then_some(candidate)
    })
}

/// [`ENGRAM_MCP_DB_ENV_VAR`], then the well-known taipan path, then
/// engram's own cwd-relative default. ALWAYS resolves to a real
/// (non-`:memory:`) path - see the module doc's "always resolves" section.
#[must_use]
pub fn default_db_path() -> PathBuf {
    env_db_path().unwrap_or_else(|| well_known_db_path().unwrap_or_else(cwd_relative_db_path))
}

/// [`ENGRAM_MCP_DB_ENV_VAR`]. `None` when unset, blank, or the literal
/// `:memory:` sentinel (a non-persistent store the panel must never silently
/// pick - see the module doc's "a real file, not `:memory:`" guard).
fn env_db_path() -> Option<PathBuf> {
    env_db_path_from(std::env::var(ENGRAM_MCP_DB_ENV_VAR).ok())
}

fn env_db_path_from(value: Option<String>) -> Option<PathBuf> {
    let value = value?;
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == ":memory:" {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

/// `~/.taipan/engram.engram` - a pre-filled suggestion, not required to
/// exist yet (unlike [`taipan_bin_under`], this is not a presence/absence
/// signal - see the module doc). `None` only when `$HOME` itself is unset.
fn well_known_db_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".taipan").join(ENGRAM_DB_FILENAME))
}

/// `./engram.engram`, relative to the console's current directory - the
/// absolute-last-resort fallback when even `$HOME` is unset, mirroring
/// `crate::crypto::env::home_fallback`'s own "always some path, never empty"
/// reasoning.
fn cwd_relative_db_path() -> PathBuf {
    PathBuf::from(ENGRAM_DB_FILENAME)
}

/// [`ENGRAM_MCP_AGENT_ID_ENV_VAR`], or `None` for "the server's own default
/// agent scope" - see the module doc.
#[must_use]
pub fn agent_id() -> Option<String> {
    agent_id_from(std::env::var(ENGRAM_MCP_AGENT_ID_ENV_VAR).ok())
}

fn agent_id_from(value: Option<String>) -> Option<String> {
    let value = value?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "genaryx-ffi-memory-env-test-{tag}-{}-{n}",
            std::process::id()
        ))
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(path, body).expect("write fixture file");
    }

    // ---- engram-mcp binary location ---------------------------------------

    #[test]
    fn taipan_bin_under_missing_home_yields_none() {
        let home = unique_dir("no-bin-home");
        assert!(taipan_bin_under(&home).is_none());
    }

    #[test]
    fn taipan_bin_under_finds_a_real_file() {
        let home = unique_dir("has-bin-home");
        let bin = home.join(".taipan").join("bin").join("engram-mcp");
        write(&bin, "#!/bin/sh\nexit 0\n");

        let found = taipan_bin_under(&home).expect("must find the fixture binary");
        assert_eq!(found, bin);

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn path_bin_from_finds_the_binary_in_any_listed_directory() {
        let empty_dir = unique_dir("path-empty");
        let hit_dir = unique_dir("path-hit");
        std::fs::create_dir_all(&empty_dir).expect("create empty dir");
        write(&hit_dir.join("engram-mcp"), "#!/bin/sh\nexit 0\n");

        let path_var = std::env::join_paths([&empty_dir, &hit_dir]).expect("join paths");
        let found = path_bin_from(path_var).expect("must find engram-mcp on the synthetic PATH");
        assert_eq!(found, hit_dir.join("engram-mcp"));

        let _ = std::fs::remove_dir_all(&empty_dir);
        let _ = std::fs::remove_dir_all(&hit_dir);
    }

    #[test]
    fn path_bin_from_with_no_match_anywhere_is_none() {
        let dir = unique_dir("path-miss");
        std::fs::create_dir_all(&dir).expect("create dir");
        let path_var = std::env::join_paths([&dir]).expect("join paths");
        assert!(path_bin_from(path_var).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- default db path ---------------------------------------------------

    #[test]
    fn env_db_path_requires_a_non_blank_non_memory_value() {
        assert!(env_db_path_from(None).is_none());
        assert!(env_db_path_from(Some(String::new())).is_none());
        assert!(env_db_path_from(Some("   ".to_string())).is_none());
        assert!(
            env_db_path_from(Some(":memory:".to_string())).is_none(),
            "the :memory: sentinel must never be picked as a real db path"
        );

        let resolved = env_db_path_from(Some("/custom/store.engram".to_string()))
            .expect("a real path resolves");
        assert_eq!(resolved, PathBuf::from("/custom/store.engram"));
    }

    #[test]
    fn env_db_path_is_honored_even_when_the_path_does_not_exist_yet() {
        // Deliberately not creating this file: engram-mcp creates the store
        // on first use, so an explicit override must be reported as-is.
        let resolved = env_db_path_from(Some("/definitely/not/a/real/store.engram".to_string()))
            .expect("explicit override resolves regardless of existence");
        assert_eq!(
            resolved,
            PathBuf::from("/definitely/not/a/real/store.engram")
        );
    }

    #[test]
    fn well_known_db_path_is_always_some_when_home_is_set() {
        // This box's real $HOME - just proving the shape, not creating a file.
        if std::env::var_os("HOME").is_some() {
            let path = well_known_db_path().expect("HOME is set on this box");
            assert!(path.ends_with(".taipan/engram.engram"));
        }
    }

    #[test]
    fn default_db_path_never_panics_and_is_never_memory() {
        let path = default_db_path();
        assert!(!path.as_os_str().is_empty());
        assert_ne!(path, PathBuf::from(":memory:"));
    }

    // ---- agent id -----------------------------------------------------------

    #[test]
    fn agent_id_from_requires_a_non_blank_value() {
        assert!(agent_id_from(None).is_none());
        assert!(agent_id_from(Some(String::new())).is_none());
        assert!(agent_id_from(Some("   ".to_string())).is_none());
        assert_eq!(
            agent_id_from(Some("agent://acme/support".to_string())),
            Some("agent://acme/support".to_string())
        );
    }
}
