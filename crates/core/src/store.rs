//! Store: local SQLite (WAL) persistence of normalized events and materialized
//! state, plus a tamper-evident checkpoint chain (console-local, honestly labeled).
//!
//! Phase-0 subset of 06 §2: `events`, `source_offsets`, `event_quarantine`, and
//! `rollup_spend_1m` (schema only here; population is a later reducer task).
//! Every insert runs in one transaction on one prepared statement; malformed
//! lines never silently vanish, they land in `event_quarantine` with a
//! file+offset reference and a reason (06 §0.5 fail-closed, 06 §2 quarantine).

use crate::command::CommandRecord;
use crate::error::{Error, Result};
use crate::event::ConsoleEvent;
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::path::Path;

/// Schema version written to `PRAGMA user_version`. Bump this and add a
/// version-gated step in [`migrate`] whenever a table shape changes; the
/// `CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF NOT EXISTS` statements
/// themselves stay safe to rerun regardless.
const SCHEMA_VERSION: i64 = 2;

/// Phase-0 schema (06 §2 subset): events, the per-file read-offset journal,
/// the quarantine table for malformed/non-conforming lines, and the spend
/// rollup table (created here, populated later by a reducer).
const MIGRATION_V1: &str = "
CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY,
    env TEXT NOT NULL,
    ts TEXT NOT NULL,
    source TEXT NOT NULL,
    type TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    run_id TEXT,
    severity TEXT,
    schema TEXT NOT NULL,
    on_behalf_of TEXT,
    data TEXT,
    prev_hash TEXT,
    raw TEXT NOT NULL,
    file TEXT,
    off INTEGER
);
CREATE INDEX IF NOT EXISTS idx_events_env_ts ON events(env, ts);
CREATE INDEX IF NOT EXISTS idx_events_agent_ts ON events(agent_id, ts);
CREATE INDEX IF NOT EXISTS idx_events_run_id ON events(run_id);
CREATE INDEX IF NOT EXISTS idx_events_type_ts ON events(type, ts);

CREATE TABLE IF NOT EXISTS source_offsets (
    file TEXT PRIMARY KEY,
    offset INTEGER NOT NULL,
    inode INTEGER
);

CREATE TABLE IF NOT EXISTS event_quarantine (
    id INTEGER PRIMARY KEY,
    env TEXT,
    file TEXT,
    off INTEGER,
    raw TEXT NOT NULL,
    reason TEXT NOT NULL,
    ts TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS rollup_spend_1m (
    env TEXT,
    minute TEXT,
    agent_id TEXT,
    cost_microusd INTEGER,
    calls INTEGER,
    blocked INTEGER,
    saved_microusd INTEGER,
    PRIMARY KEY(env, minute, agent_id)
);
";

/// Phase-1 schema addition (06 §2 `commands_journal`, `command::record`):
/// the durable audit row for one privileged console mutation (kill / budget
/// change / incident ack). `params` is stored as JSON text, the same
/// text-column convention [`Store::insert_batch`] uses for `data`/
/// `on_behalf_of`.
const MIGRATION_V2: &str = "
CREATE TABLE IF NOT EXISTS commands_journal (
    id INTEGER PRIMARY KEY,
    ts TEXT NOT NULL,
    operator TEXT NOT NULL,
    env TEXT NOT NULL,
    action TEXT NOT NULL,
    target TEXT NOT NULL,
    params TEXT NOT NULL,
    decision TEXT NOT NULL,
    sig_alg TEXT NOT NULL,
    sig_fpr TEXT NOT NULL,
    http_status INTEGER NOT NULL,
    verify_result TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_commands_journal_env_ts ON commands_journal(env, ts);
CREATE INDEX IF NOT EXISTS idx_commands_journal_target ON commands_journal(target);
";

/// A row read back from `events`, shaped for the shells: envelope fields plus
/// provenance, so every byte shown can point back to its source (06 §0.8).
#[derive(Debug, Clone, serde::Serialize)]
pub struct StoredEvent {
    pub id: i64,
    pub env: String,
    pub ts: String,
    pub source: String,
    pub type_: String,
    pub agent_id: String,
    pub run_id: Option<String>,
    pub severity: Option<String>,
    pub schema: String,
    pub on_behalf_of: Vec<String>,
    pub data: Option<serde_json::Value>,
    pub prev_hash: Option<String>,
    pub raw: String,
    pub file: Option<String>,
    pub off: Option<u64>,
}

/// The delegation-relevant columns of one event, as read by
/// [`Store::delegation_events`] for building the [`crate::graph::DelegationGraph`].
/// Deliberately narrower than [`StoredEvent`] (no `raw`/`data`/provenance) so a
/// full-table scan for the graph stays cheap.
#[derive(Debug, Clone)]
pub struct DelegationRow {
    pub agent_id: String,
    pub on_behalf_of: Vec<String>,
    pub ts: String,
    pub source: String,
    pub type_: String,
    pub run_id: Option<String>,
}

/// Handle to the console's local store.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (or create) the store at `path`: WAL + fail-closed pragmas, then run
    /// idempotent migrations for the 06 §2 schema.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| Error::Store(format!("open {}: {e}", path.display())))?;
        Self::from_connection(conn)
    }

    /// Open an in-memory store: same pragmas and migrations, no file on disk.
    /// Used by tests (and any throwaway preview session).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(store_err)?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        set_pragmas(&conn)?;
        migrate(&conn)?;
        Ok(Self { conn })
    }

    /// Batch-insert normalized events in a single transaction on one prepared
    /// statement. Returns rows written. Conformance is decided upstream (by
    /// [`crate::conform`]); a line that failed conformance never becomes a
    /// `ConsoleEvent`, so it never reaches this method, it goes to
    /// [`Store::quarantine`] instead.
    pub fn insert_batch(&self, events: &[ConsoleEvent]) -> Result<usize> {
        if events.is_empty() {
            return Ok(0);
        }

        // `&self` per the shared Store signature: a checked `Connection::transaction`
        // needs `&mut Connection`, so we defer to `unchecked_transaction` instead
        // (the Store owns its one connection; nothing else can interleave a nested
        // transaction on it).
        let tx = self.conn.unchecked_transaction().map_err(store_err)?;
        let mut inserted = 0usize;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO events \
                     (env, ts, source, type, agent_id, run_id, severity, schema, \
                      on_behalf_of, data, prev_hash, raw, file, off) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                )
                .map_err(store_err)?;

            for ce in events {
                // Empty on_behalf_of (the common case: no delegation chain) stores
                // as NULL rather than an empty-array literal, so absence reads
                // unambiguously in the column.
                let on_behalf_of = if ce.event.on_behalf_of.is_empty() {
                    None
                } else {
                    Some(serde_json::to_string(&ce.event.on_behalf_of).map_err(store_err)?)
                };
                let data = ce
                    .event
                    .data
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(store_err)?;
                // SQLite integers are signed 64-bit; convert at the boundary rather
                // than depend on rusqlite's `fallible_uint` feature (not enabled).
                let off = ce.provenance.offset.map(to_i64).transpose()?;

                stmt.execute(params![
                    ce.provenance.env,
                    ce.event.ts,
                    ce.event.source,
                    ce.event.event_type,
                    ce.event.agent_id,
                    ce.event.run_id,
                    ce.event.severity,
                    ce.event.schema,
                    on_behalf_of,
                    data,
                    ce.event.prev_hash,
                    ce.raw,
                    ce.provenance.file,
                    off,
                ])
                .map_err(store_err)?;
                inserted += 1;
            }
        }
        tx.commit().map_err(store_err)?;
        Ok(inserted)
    }

    /// The most recent `limit` events, newest first by `id`.
    pub fn recent_events(&self, limit: usize) -> Result<Vec<StoredEvent>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, env, ts, source, type, agent_id, run_id, severity, schema, \
                 on_behalf_of, data, prev_hash, raw, file, off \
                 FROM events ORDER BY id DESC LIMIT ?1",
            )
            .map_err(store_err)?;

        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut rows = stmt.query(params![limit]).map_err(store_err)?;

        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(store_err)? {
            out.push(stored_event_from_row(row)?);
        }
        Ok(out)
    }

    /// This agent's most recent `limit` events, newest first by `id` - the
    /// events slice of an Agent 360 card (PHASE3 W3). Uses the existing
    /// `idx_events_agent_ts` index, so it stays cheap even over a large table.
    pub fn events_for_agent(&self, agent_id: &str, limit: usize) -> Result<Vec<StoredEvent>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, env, ts, source, type, agent_id, run_id, severity, schema, \
                 on_behalf_of, data, prev_hash, raw, file, off \
                 FROM events WHERE agent_id = ?1 ORDER BY id DESC LIMIT ?2",
            )
            .map_err(store_err)?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut rows = stmt.query(params![agent_id, limit]).map_err(store_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(store_err)? {
            out.push(stored_event_from_row(row)?);
        }
        Ok(out)
    }

    /// Every event of one run, OLDEST-first (by `id`) - the chronological
    /// timeline a Run Replay scrubs through (PHASE3 W4). Uses the existing
    /// `idx_events_run_id` index; `limit` caps a pathologically long run. Note
    /// this is oldest-first, the reverse of [`Store::recent_events`] /
    /// [`Store::events_for_agent`], because replay plays forward in time.
    pub fn events_for_run(&self, run_id: &str, limit: usize) -> Result<Vec<StoredEvent>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, env, ts, source, type, agent_id, run_id, severity, schema, \
                 on_behalf_of, data, prev_hash, raw, file, off \
                 FROM events WHERE run_id = ?1 ORDER BY id ASC LIMIT ?2",
            )
            .map_err(store_err)?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut rows = stmt.query(params![run_id, limit]).map_err(store_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(store_err)? {
            out.push(stored_event_from_row(row)?);
        }
        Ok(out)
    }

    /// The delegation-relevant columns of every event, oldest-first (by `id`),
    /// for batch-building the core [`crate::graph::DelegationGraph`] (PHASE3
    /// W1). Only the columns the graph needs, so it stays cheap over a large
    /// `events` table; the live path feeds the graph one
    /// [`crate::event::AgentEvent`] at a time instead.
    pub fn delegation_events(&self) -> Result<Vec<DelegationRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT agent_id, on_behalf_of, ts, source, type, run_id \
                 FROM events ORDER BY id ASC",
            )
            .map_err(store_err)?;
        let mut rows = stmt.query([]).map_err(store_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(store_err)? {
            let obo_json: Option<String> = row.get(1).map_err(store_err)?;
            let on_behalf_of: Vec<String> = obo_json
                .map(|s| serde_json::from_str(&s))
                .transpose()
                .map_err(store_err)?
                .unwrap_or_default();
            out.push(DelegationRow {
                agent_id: row.get(0).map_err(store_err)?,
                on_behalf_of,
                ts: row.get(2).map_err(store_err)?,
                source: row.get(3).map_err(store_err)?,
                type_: row.get(4).map_err(store_err)?,
                run_id: row.get(5).map_err(store_err)?,
            });
        }
        Ok(out)
    }

    /// Total number of events ever inserted (not windowed by retention).
    pub fn event_count(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .map_err(store_err)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    /// Record a malformed or non-conforming line: a file+offset reference and
    /// the reason it was rejected, so nothing on the ingest path is silently
    /// dropped (06 §2 quarantine, 06 §0.5 fail-closed).
    pub fn quarantine(
        &self,
        env: &str,
        file: Option<&str>,
        off: Option<u64>,
        raw: &str,
        reason: &str,
        ts: &str,
    ) -> Result<()> {
        let off = off.map(to_i64).transpose()?;
        self.conn
            .execute(
                "INSERT INTO event_quarantine (env, file, off, raw, reason, ts) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![env, file, off, raw, reason, ts],
            )
            .map_err(store_err)?;
        Ok(())
    }

    /// Total number of quarantined lines.
    pub fn quarantine_count(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM event_quarantine", [], |row| {
                row.get(0)
            })
            .map_err(store_err)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    /// Last journaled read offset for `file`, if it has been seen before.
    pub fn get_offset(&self, file: &str) -> Result<Option<u64>> {
        let offset: Option<i64> = self
            .conn
            .query_row(
                "SELECT offset FROM source_offsets WHERE file = ?1",
                params![file],
                |row| row.get(0),
            )
            .optional()
            .map_err(store_err)?;
        offset.map(from_i64).transpose()
    }

    /// Journal the read offset for `file` (upsert: the latest call wins).
    pub fn set_offset(&self, file: &str, offset: u64, inode: Option<u64>) -> Result<()> {
        let offset = to_i64(offset)?;
        let inode = inode.map(to_i64).transpose()?;
        self.conn
            .execute(
                "INSERT INTO source_offsets (file, offset, inode) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(file) DO UPDATE SET offset = excluded.offset, inode = excluded.inode",
                params![file, offset, inode],
            )
            .map_err(store_err)?;
        Ok(())
    }

    /// Journal one privileged console mutation outcome (`commands_journal`,
    /// migration v2, 06 §2 / [`crate::command::record`]). `ts` is supplied by
    /// the caller so the journaled row and the emitted `console_command` bus
    /// event agree on when the command happened; `params` serializes to JSON
    /// text, the same text-column convention [`Store::insert_batch`] uses for
    /// `data`/`on_behalf_of`.
    pub fn insert_command(&self, rec: &CommandRecord, ts: &str) -> Result<()> {
        let params_json = serde_json::to_string(&rec.params).map_err(store_err)?;
        let http_status = i64::from(rec.http_status);
        self.conn
            .execute(
                "INSERT INTO commands_journal \
                 (ts, operator, env, action, target, params, decision, sig_alg, sig_fpr, \
                  http_status, verify_result) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    ts,
                    rec.operator,
                    rec.env,
                    rec.action,
                    rec.target,
                    params_json,
                    rec.decision,
                    rec.sig_alg,
                    rec.sig_fpr,
                    http_status,
                    rec.verify_result,
                ],
            )
            .map_err(store_err)?;
        Ok(())
    }

    /// Total number of journaled commands.
    pub fn commands_journal_count(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM commands_journal", [], |row| {
                row.get(0)
            })
            .map_err(store_err)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }
}

/// Set the WAL and fail-closed pragmas (spec): `journal_mode=WAL` for readers
/// that do not block on a writer, `foreign_keys=ON`, `busy_timeout=5000` so a
/// brief writer conflict blocks instead of erroring immediately, and
/// `synchronous=NORMAL` (the standard, safe pairing under WAL).
fn set_pragmas(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(store_err)?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(store_err)?;
    conn.pragma_update(None, "busy_timeout", 5000)
        .map_err(store_err)?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(store_err)?;
    Ok(())
}

/// Run the idempotent migrations up to [`SCHEMA_VERSION`] and record it in
/// `PRAGMA user_version`. Every DDL statement is `IF NOT EXISTS`, so calling
/// this against an already-migrated store is a safe no-op.
fn migrate(conn: &Connection) -> Result<()> {
    let current: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(store_err)?;

    if current < 1 {
        conn.execute_batch(MIGRATION_V1).map_err(store_err)?;
    }
    if current < 2 {
        conn.execute_batch(MIGRATION_V2).map_err(store_err)?;
    }
    // Future migrations gate on `current < N` here, in order, before the final
    // `PRAGMA user_version` write below.

    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(store_err)?;
    Ok(())
}

/// Decode one row of the `events` SELECT in [`Store::recent_events`] into a
/// [`StoredEvent`], including the JSON round-trip for `on_behalf_of`/`data`.
fn stored_event_from_row(row: &Row<'_>) -> Result<StoredEvent> {
    let on_behalf_of_json: Option<String> = row.get(9).map_err(store_err)?;
    let data_json: Option<String> = row.get(10).map_err(store_err)?;
    let off: Option<i64> = row.get(14).map_err(store_err)?;

    let on_behalf_of: Vec<String> = on_behalf_of_json
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .map_err(store_err)?
        .unwrap_or_default();
    let data = data_json
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .map_err(store_err)?;

    Ok(StoredEvent {
        id: row.get(0).map_err(store_err)?,
        env: row.get(1).map_err(store_err)?,
        ts: row.get(2).map_err(store_err)?,
        source: row.get(3).map_err(store_err)?,
        type_: row.get(4).map_err(store_err)?,
        agent_id: row.get(5).map_err(store_err)?,
        run_id: row.get(6).map_err(store_err)?,
        severity: row.get(7).map_err(store_err)?,
        schema: row.get(8).map_err(store_err)?,
        on_behalf_of,
        data,
        prev_hash: row.get(11).map_err(store_err)?,
        raw: row.get(12).map_err(store_err)?,
        file: row.get(13).map_err(store_err)?,
        off: off.map(from_i64).transpose()?,
    })
}

/// Map any displayable error (rusqlite, serde_json) into the crate's
/// fail-closed `Error::Store` variant, so callers never see a panic and never
/// have a failed write pass silently.
fn store_err<E: std::fmt::Display>(e: E) -> Error {
    Error::Store(e.to_string())
}

/// SQLite integers are signed 64-bit and rusqlite's `u64: ToSql`/`FromSql` sit
/// behind its `fallible_uint` feature (not enabled here: `crates/core`
/// activates rusqlite as `{ workspace = true }`, only the workspace-declared
/// `bundled` feature). Convert at the boundary instead, fail-closed on the
/// (practically unreachable) overflow case rather than truncating silently.
fn to_i64(v: u64) -> Result<i64> {
    i64::try_from(v).map_err(store_err)
}

fn from_i64(v: i64) -> Result<u64> {
    u64::try_from(v).map_err(store_err)
}
