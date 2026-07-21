//! Remote-panel environment discovery: JUST a best-effort default
//! `wireguard-go` binary path (docs/PHASE4.md W4, decision D11).
//!
//! Unlike every other panel, the Remote panel has NO auto-discovered
//! "environment" at all - the WG peer, the SSH target, and even the
//! `wireguard-go` binary path are things the OPERATOR defines for a specific
//! campaign (docs/PHASE4.md W4 v1 scope position 2: "the operator defines a
//! remote environment ... Persist it in the panel state"), never read off a
//! `taipan up` descriptor the way Quality/Identity/Drills/Memory pull their
//! service URLs. So this module's only job is the SAME well-known,
//! fixed-location convention `crypto::env`/`identity::state` use for
//! `qryx`/`idryx`, applied to `wireguard-go`, plus a `$PATH` fallback
//! (mirrors `memory::env::discover_bin`'s two-tier shape, minus its third,
//! project-checkout tier - wireguard-go is a bundled third-party binary with
//! no TAIPANBOX source checkout to fall back to). This is purely a
//! best-effort DEFAULT the operator's environment form pre-fills with and can
//! freely override; never an authority the rest of the panel depends on
//! (`state::RemoteEnvironmentConfig::wireguard_go_bin` is its own
//! operator-owned, saved string - see `state.rs`'s module doc).
//!
//! Never panics: every filesystem/PATH step is a plain `Option` chain.

use std::path::{Path, PathBuf};

/// `~/.taipan/bin/wireguard-go`, then a `$PATH` scan - best-effort, `None`
/// when neither resolves.
#[must_use]
pub fn discover() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);

    if let Some(h) = &home {
        let well_known = well_known_bin_path(h);
        if well_known.is_file() {
            return Some(well_known);
        }
    }

    if let Some(path_var) = std::env::var_os("PATH")
        && let Some(found) = find_on_path("wireguard-go", std::env::split_paths(&path_var))
    {
        return Some(found);
    }

    None
}

fn well_known_bin_path(home: &Path) -> PathBuf {
    home.join(".taipan").join("bin").join("wireguard-go")
}

/// A dependency-free `$PATH` scan - mirrors `memory::env::find_on_path`
/// exactly (deliberately duplicated, not shared: independent panel modules,
/// same rationale `memory::state`'s doc comment gives for its own
/// duplication - this crate's `Cargo.toml` sanctions no `which`-style crate
/// for one lookup).
fn find_on_path(name: &str, mut dirs: impl Iterator<Item = PathBuf>) -> Option<PathBuf> {
    dirs.find_map(|dir| {
        let candidate = dir.join(name);
        candidate.is_file().then_some(candidate)
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
            "genaryx-remote-env-test-{tag}-{}-{n}",
            std::process::id()
        ))
    }

    #[test]
    fn well_known_bin_path_ends_with_the_expected_relative_shape() {
        let home = PathBuf::from("/home/op");
        assert_eq!(
            well_known_bin_path(&home),
            PathBuf::from("/home/op/.taipan/bin/wireguard-go")
        );
    }

    #[test]
    fn find_on_path_finds_an_existing_file_in_a_later_directory() {
        let empty_dir = unique_dir("path-empty");
        let hit_dir = unique_dir("path-hit");
        std::fs::create_dir_all(&empty_dir).expect("create empty dir");
        std::fs::create_dir_all(&hit_dir).expect("create hit dir");
        std::fs::write(hit_dir.join("wireguard-go"), "#!/bin/sh\n").expect("write fixture");

        let found = find_on_path(
            "wireguard-go",
            vec![empty_dir.clone(), hit_dir.clone()].into_iter(),
        );
        assert_eq!(found, Some(hit_dir.join("wireguard-go")));

        let _ = std::fs::remove_dir_all(&empty_dir);
        let _ = std::fs::remove_dir_all(&hit_dir);
    }

    #[test]
    fn find_on_path_returns_none_when_absent_everywhere() {
        let a = unique_dir("path-a");
        let b = unique_dir("path-b");
        std::fs::create_dir_all(&a).expect("create dir a");
        std::fs::create_dir_all(&b).expect("create dir b");

        assert!(find_on_path("wireguard-go", vec![a.clone(), b.clone()].into_iter()).is_none());

        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn discover_never_panics() {
        // Best-effort, like every other HOME/PATH-dependent resolution in
        // this codebase: only proves this resolves to a consistent Option
        // without panicking, regardless of this box's actual local state.
        let _ = discover();
    }
}
