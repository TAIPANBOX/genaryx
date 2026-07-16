//! Ingest: the `EventSource` abstraction and its implementations. Every line is
//! conform-validated (via [`crate::conform`]), normalized to [`ConsoleEvent`] with
//! provenance, batch-written to the Store, and broadcast live to the shells.
//!
//! Phase-0 stub. Full FileTail delegated to Sonnet (task #4). Spec anchors:
//! 06 §2 `IngestService`, 07 §3 (per-service NDJSON files, poll fallback 250ms,
//! offset journal per file, resilience to rotation/truncation, re-open each cycle).
//! Other impls (SshTail, CloudSse, ApiPoll) follow in later phases.

use crate::error::Result;
use crate::event::ConsoleEvent;

/// A push source of agent-event lines. Impls: `FileTail`, `SshTail`, `CloudSse`, `ApiPoll`.
pub trait EventSource {
    /// Stable id for provenance and the UI (e.g. "filetail:tokenfuse").
    fn id(&self) -> &str;

    /// Poll for newly available events since the last call. Never blocks forever;
    /// returns an empty vec when there is nothing new.
    fn poll(&mut self) -> Result<Vec<ConsoleEvent>>;
}

/// Tails one or more local NDJSON files. FSEvents/inotify with a 250ms poll
/// fallback; journals per-file offsets; survives rotation and truncation by
/// re-opening on each cycle (07 §3, matching the `verdryx.events` pattern).
pub struct FileTail {
    id: String,
}

impl FileTail {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

impl EventSource for FileTail {
    fn id(&self) -> &str {
        &self.id
    }

    fn poll(&mut self) -> Result<Vec<ConsoleEvent>> {
        // TODO(sonnet, task#4): read from journaled offset, conform each line,
        // normalize to ConsoleEvent (with file+offset provenance), quarantine
        // malformed with a counter. Return the fresh batch.
        Ok(Vec::new())
    }
}
