//! Console commands for the Memory view: `memory_status` plus three plain
//! reads over the long-lived `engram-mcp` process (docs/PHASE4.md W2):
//!
//! - [`memory_stats`] - store-wide counts + fact validity + entities/
//!   reflections + vector-index size + db path/size.
//! - [`memory_recall`] - a ranked query.
//! - [`memory_why`] - one memory's provenance.
//!
//! Plus [`memory_forget`], the one admin mutation.
//!
//! Read-only in the sense that matters here (no `genaryx_core::command::record`
//! journal entry, same as Identity/Quality): Engram is an agent-facing memory
//! store, not a TAIPANBOX governance plane this console mutates as an
//! operator action, so nothing here writes a `console_command` onto the
//! console's own bus. [`memory_forget`] is the one real exception to
//! "read-only" - it is a genuine, irreversible delete - but even it carries
//! no journal entry: `EngramClient::forget`'s frozen signature takes only a
//! `memory_id`, with no `reason`/journal-friendly shape to record, so the
//! guard is entirely a frontend confirm ceremony (`ConfirmButton`), not a
//! server-side one.
//!
//! Every call locks the long-lived `EngramClient` (see `state.rs`'s module
//! doc for why a blocking mutex) and runs its one MCP round trip inside
//! `tokio::task::spawn_blocking` - mirrors
//! `identity::commands::identity_rescan`'s identical "blocking work never
//! runs straight on the async executor" discipline, doubly so here since
//! `recall`'s first call can block for several seconds loading engram's
//! embedding model (docs/PHASE4.md W2's CRITICAL note).

use super::env::EnvSource;
use super::pool;
use super::state::{MemoryClient, MemoryInner, MemoryState};
use genaryx_connectors::{
    EngramClient, EngramForgetResult, EngramMemory, EngramProvenance, EngramStats, McpError,
};
use serde::Serialize;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

// ============================================================================
// DTOs
// ============================================================================

/// Whole-panel connection state, for the frontend to render up front (never
/// inferred from a read command's error shape) - mirrors
/// `identity::commands::IdentityStatusDto`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MemoryStatusDto {
    Bootstrapping,
    NoEnvironment,
    Unreachable {
        source: EnvSource,
        engram_mcp_bin: String,
        db_path: String,
        reason: String,
    },
    Ready {
        source: EnvSource,
        engram_mcp_bin: String,
        db_path: String,
    },
}

/// Every error a memory command can return - mirrors
/// `identity::commands::IdentityError`'s shape: `McpError` carries no
/// HTTP-style status to preserve either, just a message (same rationale
/// `crypto::commands::CryptoError::Qryx` follows for `QryxError`).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MemoryError {
    Bootstrapping,
    NoEnvironment,
    Unreachable {
        reason: String,
    },
    /// Any Engram-side failure: a spawn/IO/protocol/RPC problem, or a
    /// `McpError::Tool` (e.g. `why` on an unknown memory id - "memory not
    /// found" surfaces here verbatim, never a fabricated empty provenance).
    Mcp {
        message: String,
    },
}

impl From<McpError> for MemoryError {
    fn from(e: McpError) -> Self {
        MemoryError::Mcp {
            message: e.to_string(),
        }
    }
}

// ============================================================================
// helpers
// ============================================================================

/// Resolve the current [`MemoryClient`] out of managed state, or the
/// appropriate [`MemoryError`] when the panel is not ready. Only holds the
/// state lock long enough to clone the (cheap, `Arc`-backed) client out -
/// mirrors `identity::commands::ready_client` exactly.
async fn ready_client(state: &&MemoryState) -> Result<MemoryClient, MemoryError> {
    let guard = state.inner.lock().await;
    match &*guard {
        MemoryInner::Ready(client) => Ok(client.clone()),
        MemoryInner::Bootstrapping => Err(MemoryError::Bootstrapping),
        MemoryInner::NoEnvironment => Err(MemoryError::NoEnvironment),
        MemoryInner::Unreachable { reason, .. } => Err(MemoryError::Unreachable {
            reason: reason.clone(),
        }),
    }
}

/// Pure `MemoryInner` -> `MemoryStatusDto` mapping, factored out of
/// [`memory_status`] so it is directly unit-testable - same rationale as
/// `identity::commands::status_dto`.
fn status_dto(inner: &MemoryInner) -> MemoryStatusDto {
    match inner {
        MemoryInner::Bootstrapping => MemoryStatusDto::Bootstrapping,
        MemoryInner::NoEnvironment => MemoryStatusDto::NoEnvironment,
        MemoryInner::Unreachable {
            source,
            engram_mcp_bin,
            db_path,
            reason,
        } => MemoryStatusDto::Unreachable {
            source: source.clone(),
            engram_mcp_bin: engram_mcp_bin.display().to_string(),
            db_path: db_path.display().to_string(),
            reason: reason.clone(),
        },
        MemoryInner::Ready(client) => MemoryStatusDto::Ready {
            source: client.source.clone(),
            engram_mcp_bin: client.engram_mcp_bin.display().to_string(),
            db_path: client.db_path.display().to_string(),
        },
    }
}

/// Lock the long-lived `EngramClient`, mapping mutex poisoning (a previous
/// call panicked while holding it - should never happen, since every method
/// here is a `Result`-returning `?`-chain with no panicking path, but this
/// stays fail-closed rather than unwrapping) into the SAME [`McpError`] shape
/// every other failure in this module already flows through.
fn lock_engram(m: &Mutex<EngramClient>) -> Result<MutexGuard<'_, EngramClient>, McpError> {
    m.lock()
        .map_err(|_| McpError::Protocol("engram client mutex poisoned".to_string()))
}

/// Run a blocking Engram MCP call off the async executor thread - shared by
/// every command below. `f` locks and uses the long-lived client (see
/// `state.rs`'s module doc for why it is never called on the async
/// executor).
async fn run_blocking<T, F>(f: F) -> Result<T, MemoryError>
where
    F: FnOnce() -> Result<T, McpError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| MemoryError::Mcp {
            message: format!("memory task failed to run: {e}"),
        })?
        .map_err(MemoryError::from)
}

// ============================================================================
// commands
// ============================================================================

/// Whole-panel connection state. Never fails: every outcome of
/// [`super::state::bootstrap`] is a renderable [`MemoryStatusDto`] variant.
pub async fn memory_status(state: &MemoryState) -> Result<MemoryStatusDto, ()> {
    let guard = state.inner.lock().await;
    Ok(status_dto(&guard))
}

/// `stats` - store-wide counts (episodic/semantic/procedural - `procedural`
/// is always 0 in this Engram version, the frontend labels it "not
/// implemented", never a real zero), fact validity (active vs superseded),
/// entities, reflections, vector-index size, and the db path/size
/// (docs/PHASE4.md W2 Memory position 1).
pub async fn memory_stats(
    agent_id: Option<String>,
    state: &MemoryState,
) -> Result<EngramStats, MemoryError> {
    let scope = read_scope(agent_id)?;
    let client = ready_client(&state).await?;
    run_blocking(move || {
        let target = target_client(&client, scope.as_deref())?;
        let mut engram = lock_engram(&target)?;
        engram.stats(scope.as_deref())
    })
    .await
}

/// `recall` - up to `limit` memories relevant to `query`, most relevant
/// first (docs/PHASE4.md W2 Memory position 2). Never runs on its own; the
/// frontend labels the result "as of last query". `mode` is
/// `cosine`|`spreading`|`hybrid`.
pub async fn memory_recall(
    query: String,
    limit: u32,
    mode: String,
    agent_id: Option<String>,
    state: &MemoryState,
) -> Result<Vec<EngramMemory>, MemoryError> {
    let scope = read_scope(agent_id)?;
    let client = ready_client(&state).await?;
    run_blocking(move || {
        let target = target_client(&client, scope.as_deref())?;
        let mut engram = lock_engram(&target)?;
        engram.recall(&query, limit, &mode, scope.as_deref())
    })
    .await
}

/// `why` - the provenance of one memory: a semantic fact's triple +
/// extraction chain, or an episodic memory's content + encoding/access
/// metadata, discriminated by [`EngramProvenance`]'s `kind` (docs/PHASE4.md
/// W2 Memory position 3). An unknown id is the connector's own honest
/// `McpError::Tool` ("memory not found"), surfaced as [`MemoryError::Mcp`] -
/// never a fabricated empty result.
pub async fn memory_why(
    memory_id: String,
    state: &MemoryState,
) -> Result<EngramProvenance, MemoryError> {
    let client = ready_client(&state).await?;
    run_blocking(move || lock_engram(&client.client)?.why(&memory_id)).await
}

/// `forget` - permanently delete one memory (docs/PHASE4.md W2 Memory
/// position 6, the optional admin action). Irreversible; the frontend gates
/// this behind an explicit confirm ceremony (`ConfirmButton`) rather than any
/// server-side confirmation token - `EngramClient::forget`'s frozen signature
/// carries no such field to check, and (see this module's doc comment)
/// nothing here journals a `console_command` either.
pub async fn memory_forget(
    memory_id: String,
    state: &MemoryState,
) -> Result<EngramForgetResult, MemoryError> {
    let client = ready_client(&state).await?;
    run_blocking(move || lock_engram(&client.client)?.forget(&memory_id)).await
}

/// The scope a read asks for. An empty name is the whole-file view. A name must
/// be one printable word (see [`pool::valid_agent_scope`]); anything else is
/// refused here, before any server is started for it.
pub(crate) fn read_scope(agent_id: Option<String>) -> Result<Option<String>, MemoryError> {
    match agent_id {
        None => Ok(None),
        Some(id) if id.is_empty() => Ok(None),
        Some(id) if pool::valid_agent_scope(&id) => Ok(Some(id)),
        Some(_) => Err(MemoryError::Mcp {
            message: "an agent name must be one printable word; no engram-mcp was started for it"
                .to_string(),
        }),
    }
}

/// The server a read goes to: the whole-file server when no agent is asked for,
/// otherwise the pool's server for that agent, started on its first read.
fn target_client(
    client: &MemoryClient,
    scope: Option<&str>,
) -> Result<Arc<Mutex<EngramClient>>, McpError> {
    let Some(scope) = scope else {
        return Ok(Arc::clone(&client.client));
    };
    let mut pool = client
        .scoped
        .lock()
        .map_err(|_| McpError::Protocol("scoped engram pool mutex poisoned".to_string()))?;
    pool.client_for(scope, Instant::now(), || {
        let db = client.db_path.to_string_lossy().into_owned();
        EngramClient::spawn(&client.engram_mcp_bin, &db, Some(scope)).map(Mutex::new)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const BOT: &str = "agent://acme.example/support/bot";

    #[test]
    fn an_empty_agent_is_the_whole_file_view() {
        assert_eq!(read_scope(None).expect("no agent"), None);
        assert_eq!(
            read_scope(Some(String::new())).expect("an empty name"),
            None
        );
    }

    #[test]
    fn a_bad_agent_is_refused_before_any_server_starts() {
        assert!(read_scope(Some("--db".to_string())).is_err());
        assert!(read_scope(Some("two words".to_string())).is_err());
        assert_eq!(
            read_scope(Some(BOT.to_string())).expect("an agent id"),
            Some(BOT.to_string())
        );
    }

    #[test]
    fn status_dto_maps_bootstrapping_and_no_environment_directly() {
        assert!(matches!(
            status_dto(&MemoryInner::Bootstrapping),
            MemoryStatusDto::Bootstrapping
        ));
        assert!(matches!(
            status_dto(&MemoryInner::NoEnvironment),
            MemoryStatusDto::NoEnvironment
        ));
    }

    #[test]
    fn status_dto_unreachable_preserves_paths_and_reason() {
        let unreachable = MemoryInner::Unreachable {
            source: EnvSource::WellKnown,
            engram_mcp_bin: PathBuf::from("/tmp/engram-mcp"),
            db_path: PathBuf::from("/tmp/.engram"),
            reason: "spawn failed".to_string(),
        };
        match status_dto(&unreachable) {
            MemoryStatusDto::Unreachable {
                engram_mcp_bin,
                db_path,
                reason,
                ..
            } => {
                assert_eq!(engram_mcp_bin, "/tmp/engram-mcp");
                assert_eq!(db_path, "/tmp/.engram");
                assert_eq!(reason, "spawn failed");
            }
            other => panic!("expected Unreachable, got {other:?}"),
        }
    }

    // `status_dto`'s `Ready` branch (three plain `.display().to_string()`
    // field reads) is exercised by
    // `state::tests::spawn_client_reports_ready_over_a_mock_engram_mcp_process`
    // instead of a hand-built fixture here: unlike every other panel's
    // `XxxClient` (a cheap, always-succeeding constructor -
    // `QryxClient::new`/`IdryxClient::new` never touch a process),
    // `EngramClient`'s only constructor is `spawn`, which launches and
    // handshakes with a real subprocess (see `state.rs`'s module doc) - so
    // building a `MemoryClient` for a unit test means actually spawning
    // something, which is `state.rs`'s test to own (it already owns
    // `spawn_client`, the function that does the spawning).

    #[test]
    fn memory_error_from_mcp_error_carries_a_message() {
        // A real `McpError` from a genuine failed spawn - same fixture
        // pattern `crypto::commands`'s tests use for `QryxError`.
        let env = std::collections::BTreeMap::new();
        let err = genaryx_connectors::McpStdioClient::spawn(
            "/nonexistent/engram-mcp-binary-xyz",
            &[],
            &env,
        )
        .expect_err("a nonexistent binary must fail to spawn");
        let mapped = MemoryError::from(err);
        let MemoryError::Mcp { message } = mapped else {
            panic!("expected a Mcp-shaped MemoryError")
        };
        assert!(!message.is_empty());
    }
}
