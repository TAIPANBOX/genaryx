//! Memory-panel console-managed state: a resolved, spawned, long-lived
//! `EngramClient` (or an honest record of why there isn't one), behind a
//! blocking mutex.
//!
//! docs/PHASE4.md W2's CRITICAL note: unlike every other panel's connector
//! (stateless per call), `EngramClient` owns ONE `engram-mcp` child process,
//! spawned once via `EngramClient::spawn` and reused for every read
//! (re-spawning per call would re-run engram's lazy embedding-model load, a
//! multi-second cost, on every `recall`). So this module's `bootstrap`
//! performs the ONE spawn+handshake and keeps the resulting `EngramClient`
//! alive in managed state for the panel's whole lifetime - it does NOT
//! discard it the way `quality::state::confirm_openable` discards its
//! one-time-confirmation `VerdryxClient`.
//!
//! `EngramClient`'s methods take `&mut self` (its underlying
//! `McpStdioClient` advances a JSON-RPC request id per call and owns a
//! single-writer/single-reader stdio pipe pair - concurrent, unsynchronized
//! calls could interleave two requests' newline-delimited JSON onto the same
//! pipe and corrupt the framing). So the client lives behind a plain
//! `std::sync::Mutex` (not `tokio::sync::Mutex`): every command in
//! `super::commands` locks it and calls exactly one method INSIDE
//! `tokio::task::spawn_blocking` (the MCP round trip is blocking
//! IO), so the lock is only ever acquired from a blocking OS thread, never
//! from the async executor - a plain blocking mutex is the simplest correct
//! tool there, no `tokio::sync::Mutex::blocking_lock` ceremony needed.
//!
//! On `Drop`, `EngramClient` (via its inner `McpStdioClient`) closes the
//! child's stdin then kill+waits it - reaping our own subprocess, which is
//! always allowed (see `mcp_stdio.rs`'s own `Drop` doc); this happens
//! automatically whenever the last `Arc` to a [`MemoryClient::client`] drops
//! (app shutdown, or a future reconnect that replaces `MemoryState.inner`).

use super::env::{self, EnvSource, ResolvedEnv};
use genaryx_connectors::EngramClient;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// A live, ready-to-use Memory connection: the long-lived `EngramClient`
/// behind a blocking mutex, plus enough of the resolved environment for the
/// status DTO and error messages. Cheap to clone (an `Arc`ed mutex plus a
/// tagged source and two paths) - mirrors every other panel's `XxxClient`
/// clone-out-of-the-lock convention.
#[derive(Clone)]
pub struct MemoryClient {
    pub source: EnvSource,
    pub engram_mcp_bin: PathBuf,
    pub db_path: PathBuf,
    pub client: Arc<Mutex<EngramClient>>,
}

/// The Memory panel's whole state machine - mirrors `IdentityInner`'s four
/// shapes. `Unreachable` here means "resolved a binary+db pair, but spawning
/// `engram-mcp` or the MCP `initialize` handshake failed", distinct from
/// `NoEnvironment`'s "never found a candidate binary/db pair at all".
pub enum MemoryInner {
    /// The initial state from [`MemoryState::pending`], until the
    /// background [`bootstrap`] task resolves.
    Bootstrapping,
    /// [`env::discover`] found nothing usable: no `engram-mcp` binary and/or
    /// no real `.engram` store resolved. The common case until an operator
    /// installs engram-mcp and/or points the console at a real store - a
    /// normal, renderable "no memory plane" state, never an error.
    NoEnvironment,
    /// A binary+db pair resolved, but spawning `engram-mcp` or the MCP
    /// handshake failed (missing dependency, corrupt binary, a store the
    /// process could not open, ...).
    Unreachable {
        source: EnvSource,
        engram_mcp_bin: PathBuf,
        db_path: PathBuf,
        reason: String,
    },
    Ready(MemoryClient),
}

/// Console-managed state wrapping [`MemoryInner`] in an async mutex, mirroring
/// every other panel's identical shape.
pub struct MemoryState {
    pub inner: tokio::sync::Mutex<MemoryInner>,
}

impl MemoryState {
    /// The synchronous, immediately-manageable starting state - `setup`
    /// calls this directly, then spawns [`bootstrap`] in the background.
    #[must_use]
    pub fn pending() -> Self {
        Self {
            inner: tokio::sync::Mutex::new(MemoryInner::Bootstrapping),
        }
    }
}

/// Resolve an environment and spawn the ONE long-lived `engram-mcp` process
/// for the panel's whole lifetime, off the async executor thread (spawning +
/// the MCP `initialize` handshake is blocking IO - see this module's doc
/// comment). Never panics, never returns anything other than a
/// [`MemoryInner`] the UI can render.
///
/// No default `agent_id` is passed at spawn time (deliberately - see
/// `super::commands`'s module doc): the server keeps its own default scope,
/// and every read command instead accepts its own per-call `agent_id`, which
/// is more useful for an operator inspecting more than one agent's memory
/// without restarting the whole process.
pub async fn bootstrap() -> MemoryInner {
    let Some(resolved) = env::discover() else {
        return MemoryInner::NoEnvironment;
    };
    spawn_client(resolved).await
}

/// Testable core of [`bootstrap`]: spawn `engram-mcp` for an already-resolved
/// environment and fold the outcome into a [`MemoryInner`].
async fn spawn_client(resolved: ResolvedEnv) -> MemoryInner {
    let bin = resolved.engram_mcp_bin.clone();
    let db_path = resolved.db_path.clone();
    let spawned = tokio::task::spawn_blocking(move || {
        let db = db_path.to_string_lossy().into_owned();
        EngramClient::spawn(&bin, &db, None)
    })
    .await;

    match spawned {
        Ok(Ok(client)) => MemoryInner::Ready(MemoryClient {
            source: resolved.source,
            engram_mcp_bin: resolved.engram_mcp_bin,
            db_path: resolved.db_path,
            client: Arc::new(Mutex::new(client)),
        }),
        Ok(Err(e)) => MemoryInner::Unreachable {
            source: resolved.source,
            engram_mcp_bin: resolved.engram_mcp_bin,
            db_path: resolved.db_path,
            reason: e.to_string(),
        },
        Err(join_err) => MemoryInner::Unreachable {
            source: resolved.source,
            engram_mcp_bin: resolved.engram_mcp_bin,
            db_path: resolved.db_path,
            reason: format!("bootstrap task failed to run: {join_err}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_starts_in_the_bootstrapping_state() {
        let state = MemoryState::pending();
        let guard = state
            .inner
            .try_lock()
            .expect("uncontended right after construction");
        assert!(matches!(&*guard, MemoryInner::Bootstrapping));
    }

    #[tokio::test]
    async fn bootstrap_never_panics_with_no_environment_available() {
        // Same rationale as every other panel's identical test: this only
        // proves `bootstrap` resolves to a `MemoryInner` rather than
        // panicking or hanging, regardless of whether this box happens to
        // have a real engram-mcp + .engram store.
        let inner = bootstrap().await;
        match inner {
            MemoryInner::Bootstrapping => {
                panic!("bootstrap must resolve past its own pending state")
            }
            MemoryInner::NoEnvironment
            | MemoryInner::Unreachable { .. }
            | MemoryInner::Ready(_) => {}
        }
    }

    #[tokio::test]
    async fn spawn_client_reports_unreachable_for_a_nonexistent_binary() {
        let resolved = ResolvedEnv {
            source: EnvSource::WellKnown,
            engram_mcp_bin: PathBuf::from("/nonexistent/engram-mcp-xyz"),
            db_path: std::env::temp_dir().join("genaryx-memory-state-test.engram"),
        };
        match spawn_client(resolved).await {
            MemoryInner::Unreachable { reason, .. } => assert!(!reason.is_empty()),
            MemoryInner::Ready(_) => {
                panic!("expected Unreachable for a nonexistent binary, got Ready")
            }
            MemoryInner::NoEnvironment => {
                panic!("expected Unreachable for a nonexistent binary, got NoEnvironment")
            }
            MemoryInner::Bootstrapping => {
                panic!("expected Unreachable for a nonexistent binary, got Bootstrapping")
            }
        }
    }

    // ---- the actual "long-lived process" wrinkle, proven end to end ----
    // A minimal, real `engram-mcp` stand-in that only answers the MCP
    // `initialize` handshake (mirrors `mcp_stdio.rs`'s own
    // `end_to_end_handshake_and_tool_call_over_a_mock_server` test), written
    // to a real executable temp file with a shebang rather than
    // `python3 -c <script>` - `EngramClient::spawn` invokes its
    // `engram_mcp_bin` argument directly with no way to pass interpreter
    // flags through it. Proves `spawn_client` produces a genuinely usable,
    // lockable `EngramClient` behind `MemoryClient.client`, not just that it
    // compiles. Unix-only (shebang + chmod); skips gracefully when python3
    // is unavailable, same convention every other live-process test in this
    // codebase follows.

    #[cfg(unix)]
    const MOCK_ENGRAM_MCP_SOURCE: &str = r#"
import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    mid = msg.get("id")
    method = msg.get("method")
    if method == "initialize":
        sys.stdout.write(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"protocolVersion":"2025-06-18","serverInfo":{"name":"mock","version":"1"},"capabilities":{}}})+"\n")
        sys.stdout.flush()
"#;

    #[cfg(unix)]
    fn which_python3() -> Option<()> {
        std::process::Command::new("python3")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()
            .filter(|s| s.success())
            .map(|_| ())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_client_reports_ready_over_a_mock_engram_mcp_process() {
        use std::os::unix::fs::PermissionsExt;

        if which_python3().is_none() {
            eprintln!("skip: python3 not found");
            return;
        }

        let dir = std::env::temp_dir().join(format!(
            "genaryx-memory-state-mock-test-{}-{}",
            std::process::id(),
            nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create scratch dir");

        let bin_path = dir.join("mock-engram-mcp");
        std::fs::write(
            &bin_path,
            format!("#!/usr/bin/env python3\n{MOCK_ENGRAM_MCP_SOURCE}"),
        )
        .expect("write mock engram-mcp script");
        let mut perms = std::fs::metadata(&bin_path)
            .expect("stat mock script")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin_path, perms).expect("chmod mock script executable");

        let db_path = dir.join(".engram");
        std::fs::write(&db_path, b"not a real sqlite file, just needs to exist")
            .expect("write fixture db");

        let resolved = ResolvedEnv {
            source: EnvSource::WellKnown,
            engram_mcp_bin: bin_path.clone(),
            db_path: db_path.clone(),
        };

        match spawn_client(resolved).await {
            MemoryInner::Ready(client) => {
                assert_eq!(client.engram_mcp_bin, bin_path);
                assert_eq!(client.db_path, db_path);
                // Proves the mutex is genuinely lockable (not poisoned, a
                // real `EngramClient` behind it) - `super::commands` locks
                // it exactly this way inside every command.
                let _guard = client
                    .client
                    .lock()
                    .expect("the freshly spawned client's mutex must not be poisoned");
            }
            MemoryInner::Unreachable { reason, .. } => {
                panic!("expected Ready over the mock server, got Unreachable: {reason}")
            }
            MemoryInner::NoEnvironment => {
                panic!("expected Ready over the mock server, got NoEnvironment")
            }
            MemoryInner::Bootstrapping => {
                panic!("expected Ready over the mock server, got Bootstrapping")
            }
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    fn nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }
}
