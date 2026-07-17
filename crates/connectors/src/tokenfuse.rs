//! `TokenfuseClient`: a thin `ToolRunner` over the TokenFuse gateway CLI, for
//! the one thing the Evidence Center needs from it (docs/PHASE4.md W3): the
//! FOCUS cost export. Grounded in `~/Development/tokenfuse/crates/gateway/src/
//! focusexport.rs` (read 2026-07-17).
//!
//! `tokenfuse focus-export --traces <dir-or-glob> --out <file.csv> [--from
//! <rfc3339>] [--to <rfc3339>]` reads the already-written Parquet call trace and
//! writes a FOCUS 1.2-style CSV (FinOps Open Cost & Usage Specification), one
//! row per LLM call, so a bank/FinOps team can load agent spend into the same
//! tooling they use for cloud spend. It is read-only and never touches the
//! enforcement hot path. Because it only writes to a file (`--out`), this client
//! runs it against a private temp path, reads the CSV back, and removes the temp
//! file - so the caller gets the bytes directly, like every other connector.
//!
//! Fail-closed (06 §0.5): a spawn failure is [`TokenfuseError::Spawn`] (the live
//! test reads it as "tokenfuse absent, skip"); a nonzero exit is
//! [`TokenfuseError::Cli`] with stderr; an unreadable output file is
//! [`TokenfuseError::Read`]. No panics. The caller resolves the binary path and
//! the traces dir (env discovery is the shell's job, like every other
//! connector).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

// ---- error -----------------------------------------------------------------

/// Every failure mode a [`TokenfuseClient`] call can surface. Fail-closed.
#[derive(Debug, thiserror::Error)]
pub enum TokenfuseError {
    /// The tokenfuse binary could not be spawned (missing, not executable).
    #[error("tokenfuse spawn {bin}: {source}")]
    Spawn {
        bin: String,
        #[source]
        source: std::io::Error,
    },

    /// `focus-export` exited nonzero (bad flags, an unreadable traces dir).
    #[error("tokenfuse focus-export exited {code}: {stderr}")]
    Cli { code: i32, stderr: String },

    /// The CSV `focus-export` should have written could not be read back.
    #[error("read focus-export output {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

// ---- client ----------------------------------------------------------------

/// A `ToolRunner` over the tokenfuse gateway CLI. Holds the resolved binary
/// path; `focus_export` is one synchronous invocation.
#[derive(Debug, Clone)]
pub struct TokenfuseClient {
    bin: PathBuf,
}

impl TokenfuseClient {
    /// Construct a client for a resolved `tokenfuse` binary path.
    pub fn new(bin: impl Into<PathBuf>) -> Self {
        Self { bin: bin.into() }
    }

    /// `tokenfuse focus-export --traces <traces_dir> --out <tmp.csv> [--from
    /// <rfc3339>] [--to <rfc3339>]` -> the FOCUS 1.2 CSV bytes. Writes to a
    /// private temp file, reads it back, and removes it, so the caller receives
    /// the bytes directly. `from`/`to` optionally window the export by call
    /// timestamp.
    pub fn focus_export(
        &self,
        traces_dir: &Path,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Vec<u8>, TokenfuseError> {
        // A unique temp output path (process id + an atomic counter; no
        // wall-clock, no extra deps), removed however this returns.
        static N: AtomicU32 = AtomicU32::new(0);
        let out_path = std::env::temp_dir().join(format!(
            "genaryx-focus-{}-{}.csv",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&out_path);

        let traces = traces_dir.to_string_lossy();
        let out_s = out_path.to_string_lossy();
        let mut args: Vec<&str> = vec![
            "focus-export",
            "--traces",
            traces.as_ref(),
            "--out",
            out_s.as_ref(),
        ];
        if let Some(f) = from {
            args.push("--from");
            args.push(f);
        }
        if let Some(t) = to {
            args.push("--to");
            args.push(t);
        }

        let output = std::process::Command::new(&self.bin)
            .args(&args)
            .output()
            .map_err(|source| TokenfuseError::Spawn {
                bin: self.bin.display().to_string(),
                source,
            });
        let output = match output {
            Ok(o) => o,
            Err(e) => {
                let _ = std::fs::remove_file(&out_path);
                return Err(e);
            }
        };
        if !output.status.success() {
            let _ = std::fs::remove_file(&out_path);
            return Err(TokenfuseError::Cli {
                code: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }

        let bytes = std::fs::read(&out_path).map_err(|source| TokenfuseError::Read {
            path: out_path.display().to_string(),
            source,
        });
        let _ = std::fs::remove_file(&out_path);
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_export_missing_binary_is_fail_closed_spawn_error() {
        let c = TokenfuseClient::new("/nonexistent/tokenfuse-binary-xyz");
        match c.focus_export(Path::new("/traces"), None, None) {
            Err(TokenfuseError::Spawn { .. }) => {}
            other => panic!("expected Spawn error, got {other:?}"),
        }
    }
}
