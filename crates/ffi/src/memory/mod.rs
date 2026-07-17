//! `MemoryHandle`: the UniFFI Object wrapping `genaryx_connectors::EngramClient`
//! for the SwiftUI Memory surface (docs/PHASE4.md W2, "Track B
//! `crates/ffi/src/memory/`"), at parity with the Tauri shell's own Memory
//! panel (the sibling Track A).
//!
//! ## New shape vs every sibling handle: spawn happens IN THE CONSTRUCTOR
//!
//! Every other handle in this crate either never touches the network at
//! construction ([`crate::idryx::IdryxHandle`], [`crate::wardryx::WardryxHandle`],
//! [`crate::cloud::CloudHandle`] pair a device but do not read anything yet)
//! or never spawns a subprocess until an operator explicitly asks for a scan
//! ([`crate::crypto::CryptoHandle`], [`crate::quality::QualityHandle`]).
//! `MemoryHandle` is different by necessity: [`EngramClient`] owns ONE
//! long-lived `engram-mcp` child process for its whole life (re-spawning per
//! call would re-pay the embedding model's multi-second lazy load on every
//! `recall` - `EngramClient`'s own doc comment), so there is no "connect,
//! then scan later" split to make - connecting to the memory plane IS
//! spawning the process. [`MemoryHandle::discover`] and
//! [`MemoryHandle::connect`] therefore both attempt the real spawn+handshake
//! before returning, and a spawn failure surfaces as
//! [`dto::MemoryError::Spawn`]/[`dto::MemoryError::Io`] immediately - never a
//! panic, never a fake-ready handle (see the CRITICAL section of the task
//! brief this module was built against: "If `engram-mcp` can't be spawned,
//! the constructor fails closed with an honest 'no memory plane' error, NOT a
//! panic").
//!
//! ## Why the client sits behind a `Mutex`
//!
//! [`EngramClient`]'s five methods take `&mut self` (the underlying
//! `McpStdioClient` advances a monotonic JSON-RPC request id per call - its
//! own doc comment), but a UniFFI `Object`'s exported methods only ever see
//! `&self` (the generated Swift wrapper hands out one shared handle, not an
//! owned mutable value). [`McpStdioClient`] also holds a
//! `std::sync::mpsc::Receiver`, which is `Send` but not `Sync`, so
//! [`EngramClient`] itself cannot be shared across threads without help.
//! `Mutex<EngramClient>` closes both gaps at once: `Mutex<T>` is `Sync`
//! whenever `T: Send`, and it is exactly the interior-mutability wrapper
//! [`FleetHandle`](crate::FleetHandle) already uses for its own per-call
//! mutable state. Every exported method below locks once, calls the one
//! `EngramClient` method it wraps, and drops the guard - no call holds the
//! lock across an await point (there is none: everything here is
//! synchronous, blocking I/O) or across more than one MCP round trip.
//!
//! ## `recall`'s first call can take several seconds
//!
//! Engram lazily loads its local embedding model on the FIRST `recall`
//! (`EngramClient`'s own doc: "a cost paid once per process rather than once
//! per call"). This handle does nothing special about that latency (the
//! generous 60s per-call MCP deadline already accommodates it -
//! `McpStdioClient::DEFAULT_TIMEOUT`'s own doc); [`MemoryModel`] on the Swift
//! side is the layer that shows a progress state while a `recall` call is in
//! flight, since only it knows this is the FIRST query versus a later one.
//!
//! ## Fail-closed
//!
//! No panics, no `unwrap`/`expect` on the FFI-reachable path (06 §0.5). A
//! `why`/`forget` on an unknown memory id is the server's own `isError`,
//! surfaced as [`dto::MemoryError::Tool`] - never a fabricated empty result
//! (mirrors `EngramClient::why`'s own doc: "never a fabricated empty
//! provenance").

pub mod dto;
pub mod env;

pub use dto::{
    EngramCountsRecord, EngramForgetResultRecord, EngramMemoryRecord, EngramProvenanceRecord,
    EngramStatsRecord, MemoryError,
};
pub use env::MemoryEnvSource;

use genaryx_connectors::EngramClient;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

/// The Memory UniFFI Object: a resolved `engram-mcp` binary + db path, plus
/// the one long-lived [`EngramClient`] spawned against them. See the module
/// doc for why construction itself can fail, and why the client is
/// `Mutex`-guarded.
#[derive(uniffi::Object)]
pub struct MemoryHandle {
    source: MemoryEnvSource,
    engram_mcp_bin: PathBuf,
    db_path: PathBuf,
    agent_id: Option<String>,
    client: Mutex<EngramClient>,
}

#[uniffi::export]
impl MemoryHandle {
    /// Discover an `engram-mcp` binary ([`env::discover_bin`]: the well-known
    /// `~/.taipan/bin/engram-mcp`, then a `$PATH` scan), resolve a db path
    /// ([`env::default_db_path`]: always resolves to a real, non-`:memory:`
    /// path) and an optional agent scope ([`env::agent_id`]), then spawn.
    /// Fails closed with [`MemoryError::NoEnvironment`] when NO binary can be
    /// found at all - a normal, renderable "no memory plane" outcome
    /// (docs/PHASE4.md W2), not a bug. A binary that WAS found but could not
    /// actually be spawned (bad interpreter, missing Python package, unusable
    /// db path) surfaces distinctly as [`MemoryError::Spawn`]/
    /// [`MemoryError::Io`] - never silently collapsed into the same empty
    /// state (mirrors [`crate::crypto::CryptoHandle::discover`]'s own
    /// `NoEnvironment`-vs-`Spawn` distinction).
    #[uniffi::constructor]
    pub fn discover() -> Result<Self, MemoryError> {
        let resolved = env::discover_bin().ok_or(MemoryError::NoEnvironment)?;
        let db_path = env::default_db_path();
        let agent_id = env::agent_id();
        Self::spawn(resolved.source, resolved.bin, db_path, agent_id)
    }

    /// Point directly at `engram_mcp_bin`/`db_path`/`agent_id`, skipping
    /// discovery - for an engram-mcp the operator names explicitly. Always
    /// reports [`MemoryEnvSource::Explicit`], mirroring
    /// `CryptoHandle::connect`'s own escape-hatch role. Still spawns for
    /// real (see the module doc), so this can fail exactly like
    /// [`Self::discover`] can.
    #[uniffi::constructor]
    pub fn connect(
        engram_mcp_bin: String,
        db_path: String,
        agent_id: Option<String>,
    ) -> Result<Self, MemoryError> {
        Self::spawn(
            MemoryEnvSource::Explicit,
            PathBuf::from(engram_mcp_bin),
            PathBuf::from(db_path),
            agent_id,
        )
    }

    /// Where this handle resolved its `engram-mcp` binary from.
    pub fn source(&self) -> MemoryEnvSource {
        self.source.clone()
    }

    /// The resolved `engram-mcp` binary path this handle spawned.
    pub fn engram_mcp_bin(&self) -> String {
        self.engram_mcp_bin.display().to_string()
    }

    /// The engram store path this handle's `engram-mcp` process was started
    /// against (`stats().db_path` reports the same value back from inside
    /// the process, once a read has happened).
    pub fn db_path(&self) -> String {
        self.db_path.display().to_string()
    }

    /// The agent scope this handle's `engram-mcp` process was started with,
    /// or `None` for the server's own default scope.
    pub fn agent_id(&self) -> Option<String> {
        self.agent_id.clone()
    }

    // ---- reads --------------------------------------------------------

    /// `stats(agent_id)` - store-wide counts for `agent_id`, or this handle's
    /// own configured scope when `agent_id` is `None`.
    pub fn stats(&self, agent_id: Option<String>) -> Result<EngramStatsRecord, MemoryError> {
        let mut client = relock(&self.client);
        let stats = client.stats(agent_id.as_deref())?;
        Ok(EngramStatsRecord::from(&stats))
    }

    /// `recall(query, limit, mode, agent_id)` - up to `limit` memories
    /// relevant to `query`, most relevant first. `mode` is `cosine` /
    /// `spreading` / `hybrid` (an unrecognized value silently behaves as
    /// `cosine` on engram's own wire - `EngramClient::recall`'s own doc); the
    /// Swift model, not this handle, decides which mode string to pass.
    pub fn recall(
        &self,
        query: String,
        limit: u32,
        mode: String,
        agent_id: Option<String>,
    ) -> Result<Vec<EngramMemoryRecord>, MemoryError> {
        let mut client = relock(&self.client);
        let memories = client.recall(&query, limit, &mode, agent_id.as_deref())?;
        Ok(memories.iter().map(EngramMemoryRecord::from).collect())
    }

    /// `why(memory_id)` - the provenance of one memory, branching on its
    /// `kind`. An unknown id is the server's own `isError`, surfaced as
    /// [`MemoryError::Tool`] - never a fabricated result.
    pub fn why(&self, memory_id: String) -> Result<EngramProvenanceRecord, MemoryError> {
        let mut client = relock(&self.client);
        let provenance = client.why(&memory_id)?;
        Ok(EngramProvenanceRecord::from(&provenance))
    }

    // ---- the one admin mutation (irreversible - see MemoryModel.forget) ---

    /// `forget(memory_id)` - permanently erase one memory. Irreversible; the
    /// Swift panel guards this behind an explicit confirm step before ever
    /// calling it (docs/PHASE4.md W2: "guarded as irreversible").
    pub fn forget(&self, memory_id: String) -> Result<EngramForgetResultRecord, MemoryError> {
        let mut client = relock(&self.client);
        let result = client.forget(&memory_id)?;
        Ok(EngramForgetResultRecord::from(&result))
    }
}

// ---- private helpers (not exported over FFI) -------------------------------

impl MemoryHandle {
    fn spawn(
        source: MemoryEnvSource,
        engram_mcp_bin: PathBuf,
        db_path: PathBuf,
        agent_id: Option<String>,
    ) -> Result<Self, MemoryError> {
        let client = EngramClient::spawn(
            &engram_mcp_bin,
            &db_path.to_string_lossy(),
            agent_id.as_deref(),
        )?;
        Ok(Self {
            source,
            engram_mcp_bin,
            db_path,
            agent_id,
            client: Mutex::new(client),
        })
    }
}

/// Lock a poisoned-or-not mutex without panicking: a poisoned guard only
/// means some other call died mid-hold, and the [`EngramClient`] guarded here
/// stays perfectly usable in that case (its own state is a request-id counter
/// plus process handles, nothing that can be left "half updated"). A small,
/// self-contained copy of the crate root's own `relock` (see `crate::lib`'s
/// doc on [`FleetHandle`](crate::FleetHandle)) rather than a shared import -
/// mirrors this crate's established "independent evolution over a shared
/// cross-module abstraction" choice (`crate::wardryx::env`'s own doc comment
/// gives the same rationale for its own near-duplicate of `crate::cloud::env`).
fn relock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rust-side stand-in proving `MemoryHandle` never panics when discovery
    /// finds nothing - the common case in CI (no `engram-mcp` on the box).
    /// Mirrors `crypto::tests::discover_without_an_environment_is_a_clean_error_not_a_panic`.
    #[test]
    fn discover_without_an_environment_is_a_clean_error_not_a_panic() {
        match MemoryHandle::discover() {
            Ok(_)
            | Err(
                MemoryError::NoEnvironment | MemoryError::Spawn { .. } | MemoryError::Io { .. },
            ) => {}
            Err(other) => panic!("unexpected error shape from discover(): {other:?}"),
        }
    }

    /// UNLIKE every sibling handle's `connect()` (see the module doc's "spawn
    /// happens in the constructor"), this one DOES touch the filesystem/
    /// subprocess at construction time - so, against a binary that cannot
    /// possibly exist, this must fail with an honest `Spawn` error, never
    /// panic and never return a fake-ready `Ok` handle.
    #[test]
    fn connect_against_a_missing_binary_is_an_honest_spawn_error_not_a_panic() {
        let db = std::env::temp_dir().join(format!(
            "genaryx-ffi-memory-mod-test-{}.engram",
            std::process::id()
        ));
        match MemoryHandle::connect(
            "/definitely/not/a/real/engram-mcp".to_string(),
            db.to_string_lossy().into_owned(),
            None,
        ) {
            Err(MemoryError::Spawn { .. }) => {}
            Err(other) => panic!("expected MemoryError::Spawn, got {other:?}"),
            // Matched separately (rather than a shared `other => ..` arm) so this
            // branch never needs `MemoryHandle: Debug` - no sibling handle in this
            // crate derives it either (see e.g. `CryptoHandle`), since a live
            // handle is never `{:?}`-formatted outside a test like this one.
            Ok(_) => panic!("connect() unexpectedly succeeded against a nonexistent binary"),
        }
    }

    // ==========================================================================
    // live e2e: a real `engram-mcp` (from a sibling `~/Development/engram`
    // checkout's own venv), through the handle's own exported methods.
    // ==========================================================================
    // Skip-gracefully (an `eprintln!`, then an early return) whenever a real
    // engram-mcp cannot be obtained, never a hard failure over a missing
    // sibling checkout - mirrors `idryx::tests`' own live_e2e shape exactly.
    //
    // Exercises all four wrapped tools end to end against a real engram-mcp:
    // `stats` (dict return), `recall` (LIST return, the FastMCP
    // `{"result": [...]}` wrapper path), `why` on a real episodic memory (the
    // `summary_of: list[str]` shape) and on an unknown id (the server's
    // `isError` -> Tool), and `forget`. `recall` and episodic `why` both
    // failed against a real server until two connector bugs were fixed
    // (genaryx_connectors commit a01e01e): FastMCP wraps a list return's
    // `structuredContent` under `{"result": ...}`, and `Episode.summary_of`
    // (`engram/models.py:24`) is a `list[str]`, not the `Option<String>` the
    // connector first assumed. This test now pins the FIXED behavior, so a
    // regression fails loudly. (The Semantic `why` branch is not exercised:
    // seeding a real fact needs `engram reflect`, which may call an LLM - the
    // connector deliberately does not wrap `reflect`, so neither does this.)
    #[test]
    fn live_e2e_stats_recall_why_forget_over_a_real_engram_mcp() {
        let Some(bin) = resolve_test_engram_mcp_binary() else {
            return; // already explained why via eprintln! above
        };
        let Some(observe_bin) = resolve_test_engram_cli_binary() else {
            eprintln!(
                "genaryx-ffi memory live_e2e: SKIPPING: engram-mcp found but the `engram` CLI was not"
            );
            return;
        };

        let db = std::env::temp_dir().join(format!(
            "genaryx-ffi-memory-live-e2e-{}.engram",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&db);

        // Seed one real episodic memory via the `engram` CLI's own `observe`
        // subcommand (our connector deliberately does not wrap `remember` -
        // `EngramClient`'s own doc: "agents write their own memories; a
        // governance console does not fabricate them" - so seeding for this
        // test goes through engram's OWN write path, not ours).
        let observe = std::process::Command::new(&observe_bin)
            .args([
                "observe",
                db.to_string_lossy().as_ref(),
                "paid invoice INV-100 for acme corp",
                "--actors",
                "alice",
                "--tags",
                "billing",
            ])
            .output();
        let Ok(observe) = observe else {
            eprintln!("genaryx-ffi memory live_e2e: SKIPPING: could not run `engram observe`");
            let _ = std::fs::remove_file(&db);
            return;
        };
        if !observe.status.success() {
            eprintln!(
                "genaryx-ffi memory live_e2e: SKIPPING: `engram observe` failed: {}",
                String::from_utf8_lossy(&observe.stderr)
            );
            let _ = std::fs::remove_file(&db);
            return;
        }
        let seeded_id = extract_uuid(&String::from_utf8_lossy(&observe.stdout));
        let Some(seeded_id) = seeded_id else {
            eprintln!(
                "genaryx-ffi memory live_e2e: SKIPPING: could not parse a memory id from `engram observe`'s output"
            );
            let _ = std::fs::remove_file(&db);
            return;
        };

        let handle = match MemoryHandle::connect(
            bin.to_string_lossy().into_owned(),
            db.to_string_lossy().into_owned(),
            None,
        ) {
            Ok(handle) => handle,
            Err(e) => {
                eprintln!(
                    "genaryx-ffi memory live_e2e: SKIPPING: could not spawn a real engram-mcp: {e}"
                );
                let _ = std::fs::remove_file(&db);
                return;
            }
        };

        // stats: a real dict-returning tool, must round-trip cleanly.
        let stats = handle.stats(None).expect("stats against a live engram-mcp");
        assert_eq!(stats.counts.episodic, 1, "the one seeded episode");
        assert_eq!(
            stats.counts.procedural, 0,
            "never implemented - see EngramCountsRecord's own doc"
        );
        assert!(
            stats.db_size_bytes.unwrap_or(0) > 0,
            "a real file-backed store has a real size"
        );

        // why on the real seeded episode: an episodic observation, so it must
        // round-trip to the Episodic variant. This exercises the fixed
        // `summary_of: Vec<String>` shape (was `Option<String>`, which failed
        // to deserialize the real `list[str]` wire value; fixed in
        // genaryx_connectors, commit a01e01e).
        match handle.why(seeded_id.clone()) {
            Ok(EngramProvenanceRecord::Episodic { content, .. }) => {
                assert!(
                    content.contains("invoice"),
                    "the seeded episode's content should round-trip: {content}"
                );
            }
            other => panic!("expected Ok(Episodic) for the seeded episode, got {other:?}"),
        }

        // why on an unknown id: the server's own isError fires BEFORE any
        // structuredContent is ever deserialized, so this path is unaffected
        // by finding 2 and must still cleanly surface as Tool.
        match handle.why("does-not-exist-xyz".to_string()) {
            Err(MemoryError::Tool { .. }) => {}
            other => panic!("expected MemoryError::Tool for an unknown id, got {other:?}"),
        }

        // recall: list-returning. Exercises the fixed FastMCP list-wrapping
        // path (`parse_tool_result` now returns the bare array, not the
        // `{"result": [...]}` wrapper; fixed in genaryx_connectors, commit
        // a01e01e). With a single seeded memory a related query returns it.
        let hits = handle
            .recall("invoice".to_string(), 5, "cosine".to_string(), None)
            .expect("recall against a live engram-mcp");
        assert!(!hits.is_empty(), "the one seeded memory should be recalled");
        assert!(
            hits.iter().any(|m| m.content.contains("invoice")),
            "the seeded invoice memory should be among the hits"
        );

        // forget: dict-returning with no summary_of field, unaffected by
        // either finding, must round-trip cleanly - do this LAST, it is
        // destructive.
        let forgotten = handle
            .forget(seeded_id)
            .expect("forget against a live engram-mcp");
        assert!(forgotten.deleted);
        assert_eq!(forgotten.kind, "episodic");

        let _ = std::fs::remove_file(&db);
        eprintln!(
            "genaryx-ffi memory live_e2e: PASSED (stats/recall/why/forget all real against engram-mcp)"
        );
    }

    fn resolve_test_engram_mcp_binary() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok()?;
        let bin = PathBuf::from(home).join("Development/engram/.venv/bin/engram-mcp");
        if bin.is_file() {
            Some(bin)
        } else {
            eprintln!(
                "genaryx-ffi memory live_e2e: SKIPPING: no engram-mcp venv found at \
                 ~/Development/engram/.venv/bin/engram-mcp"
            );
            None
        }
    }

    fn resolve_test_engram_cli_binary() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok()?;
        let bin = PathBuf::from(home).join("Development/engram/.venv/bin/engram");
        bin.is_file().then_some(bin)
    }

    /// Best-effort UUID extraction from `engram observe`'s human-readable
    /// stdout (`"Observed: <uuid>\n  \"<content>\"\n..."`) - never
    /// force-unwrapped; `None` just skips the test (see the call site).
    fn extract_uuid(text: &str) -> Option<String> {
        text.split_whitespace()
            .find(|tok| tok.len() == 36 && tok.chars().filter(|c| *c == '-').count() == 4)
            .map(str::to_string)
    }
}
