//! Store: local SQLite (WAL) persistence of normalized events and materialized
//! state, plus a tamper-evident checkpoint chain (console-local, honestly labeled).
//!
//! Phase-0 stub. Full implementation delegated to Sonnet (task #3). Spec anchors:
//! 06 §2 tables (`events`, `source_offsets`, `runs`, `incidents`, `approvals`,
//! quarantine for malformed, `rollup_spend_1m/_1h`, `commands_journal`,
//! `chain_checkpoints`); batched inserts on a ~100ms cadence; retention ring
//! (default 30 days or 10M events). Dep already declared: `rusqlite` (bundled).

use crate::error::Result;
use crate::event::ConsoleEvent;
use std::path::Path;

/// Handle to the console's local store.
pub struct Store {
    // TODO(sonnet, task#3): hold `rusqlite::Connection` (WAL) + prepared statements.
}

impl Store {
    /// Open (or create) the store at `path`: set `PRAGMA journal_mode=WAL` and run
    /// migrations for the 06 §2 schema.
    pub fn open(_path: &Path) -> Result<Self> {
        // TODO(sonnet, task#3): open connection, enable WAL, apply migrations.
        Ok(Self {})
    }

    /// Batch-insert normalized events in a single transaction. Returns rows written.
    pub fn insert_batch(&self, events: &[ConsoleEvent]) -> Result<usize> {
        // TODO(sonnet, task#3): transactional insert into `events` + rollups;
        // malformed lines go to the quarantine table with a file+offset reference.
        let _ = events;
        Ok(0)
    }
}
