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
const SCHEMA_VERSION: i64 = 6;

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

/// Phase-3 schema addition: the two columns a store that OUTLIVES its process
/// needs, and neither is cosmetic.
///
/// `dedupe` is what makes a durable store possible at all. `FileTail` resets to
/// offset 0 when a file it is tailing gets shorter, which is correct (that is
/// what a truncation looks like) and is exactly what `stack-up` does on every
/// start. Against a scratch store that costs nothing, because the store died
/// with the process; against a store that survives, it re-inserts every line
/// the file still holds. The key is per (env, file, offset, raw), so replaying
/// the same bytes from the same place is a no-op while two genuinely identical
/// lines at two different offsets both land.
///
/// `ts_ms` is the event's own timestamp as epoch milliseconds, parsed once at
/// insert. `ts` is an RFC 3339 STRING, and the producers do not agree on its
/// shape: TokenFuse writes milliseconds, wardryx writes whole seconds. Today
/// every one of them writes UTC with a `Z`, so a lexicographic comparison
/// happens to sort correctly, and a window built on that would keep working
/// right up until one producer emits `+01:00` and starts landing in the wrong
/// day. An integer parsed at the boundary cannot drift that way.
///
/// A row whose `ts` will not parse still lands, with `ts_ms` NULL. It is then
/// absent from every window, which is why [`Store::undated_count`] exists: a
/// caller can say how many events it could not place in time rather than
/// quietly reporting a smaller number.
const MIGRATION_V3: &str = "
ALTER TABLE events ADD COLUMN dedupe TEXT;
ALTER TABLE events ADD COLUMN ts_ms INTEGER;
CREATE UNIQUE INDEX IF NOT EXISTS idx_events_dedupe ON events(dedupe);
CREATE INDEX IF NOT EXISTS idx_events_ts_ms ON events(ts_ms);
";

/// Phase-3 follow-up: the fingerprint that actually detects a replaced file.
///
/// The inode alone does not, and CI proved it: on Linux a `remove` followed by
/// a `create` routinely gets the SAME inode number back, so a rotated file
/// looked like the file we were already reading and the tail resumed at an
/// offset belonging to bytes that no longer existed. The test caught it because
/// it asserted the count; without that assertion this would have shipped as
/// silent, permanent event loss on every rotation.
///
/// `head_sha` is a hash of the bytes this console has ALREADY consumed, capped
/// at [`HEAD_FINGERPRINT_BYTES`]. On resume the same span is re-read and
/// compared: same bytes, same file, resume; different bytes, a different file,
/// start from the top. That is filesystem-independent, which the inode is not.
const MIGRATION_V4: &str = "
ALTER TABLE source_offsets ADD COLUMN head_sha TEXT;
";

/// The index a per-agent time series needs.
///
/// `idx_events_agent_ts` already exists and is on the TEXT `ts`, which is what
/// the per-agent event list uses. A profile groups one agent's events by DAY,
/// and days come from `ts_ms`; without this the query is a scan of every event
/// the agent ever produced, filtered afterwards.
const MIGRATION_V5: &str = "
CREATE INDEX IF NOT EXISTS idx_events_agent_ts_ms ON events(agent_id, ts_ms);
";

/// The index the Statistics aggregate needs.
///
/// `type_counts_since` groups every agent's events by type over a window, and
/// the two indexes that already exist each miss half of it: `idx_events_type_ts`
/// leads on `type`, and `idx_events_agent_ts_ms` has no `type` at all, so
/// SQLite scanned the table and built a temporary b-tree for the grouping.
///
/// @measured `crates/api/tests/stats_scale.rs`, 2026-08-11, on 42 agents x 100
/// events/day x 90 days (378,000 rows): 268 ms without this index, 117 ms with
/// it, and 21 MB added to a 260 MB store. The column order is the GROUP BY's,
/// with `ts_ms` last so the window is a range scan inside each group.
const MIGRATION_V6: &str = "
CREATE INDEX IF NOT EXISTS idx_events_agent_type_ts_ms ON events(agent_id, type, ts_ms);
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

/// The three columns the Statistics fold needs from an event whose `data` it
/// must open: who it is about, what it was, and the payload itself.
///
/// Narrow for the same reason [`DelegationRow`] is, and it matters more here:
/// the stats detail read pulls a double-digit percentage of the bus at once,
/// and `raw` (the producer's whole original line) is the bulk of every row.
/// Nothing in that fold reads it.
///
/// @measured `crates/api/tests/stats_scale.rs`, 2026-08-11, 52,920 rows out of
/// 378,000: 211 ms as `StoredEvent`, 205 ms as this. So this buys MEMORY and
/// bytes moved, NOT time. The query is scan-bound, and dropping `raw` from the
/// projection does not change how many rows SQLite has to walk. Said plainly
/// because the opposite was assumed here before the bench ran.
#[derive(Debug, Clone)]
pub struct EventDetail {
    pub agent_id: String,
    pub type_: String,
    pub data: Option<serde_json::Value>,
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

/// What the offset journal remembers about one tailed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceState {
    pub offset: u64,
    /// The inode the offset was read from, when the platform reports one.
    ///
    /// Kept, but NOT sufficient on its own: Linux reuses inode numbers, so a
    /// rotated file frequently comes back with the one its predecessor had.
    /// A DIFFERING inode is still conclusive evidence of a different file, so
    /// it stays as a cheap first check; an identical one proves nothing and
    /// [`SourceState::head_sha`] is what decides.
    pub inode: Option<u64>,
    /// Hash of the bytes already consumed from this file, capped at
    /// [`HEAD_FINGERPRINT_BYTES`]. `None` on a store written before this
    /// column existed, which a caller treats as "cannot tell".
    pub head_sha: Option<String>,
}

/// How much of a quarantined line to keep for the operator to recognize its
/// producer by. Enough for the envelope's head (schema, ts, source, type,
/// agent_id), which is where the fault almost always is, and not the payload.
pub const QUARANTINE_EXCERPT_CHARS: usize = 220;

/// One reason lines were refused, with how many and one example.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct QuarantineReason {
    /// The conformer's own words, joined. Not this crate's paraphrase of them:
    /// an operator fixing a producer needs the message the validator produced.
    pub reason: String,
    pub count: u64,
    /// The newest refusal under this reason, RFC 3339, or `None` when the row
    /// carried no timestamp.
    pub last_ts: Option<String>,
    /// Which file and byte offset one of these came from, so the producer is
    /// findable on disk rather than guessable.
    pub example_file: Option<String>,
    pub example_offset: Option<u64>,
    /// The head of one refused line, capped at [`QUARANTINE_EXCERPT_CHARS`].
    pub raw_excerpt: Option<String>,
}

/// The head of `s`, at most `max` CHARACTERS, with an ellipsis when cut.
///
/// Chars rather than bytes: a cut in the middle of a multi-byte sequence would
/// panic on a slice, and these lines carry real names.
fn excerpt(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}...")
}

/// One agent's count of one event type over a window, with the newest `ts`
/// string in that group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTypeCount {
    pub agent_id: String,
    pub type_: String,
    pub count: u64,
    /// `MAX(ts)`, so the producer's own timestamp string rather than a
    /// normalized one. Compared lexicographically, which is what a caller
    /// folding these rows by hand was already doing and is correct for the
    /// RFC 3339 spellings on this bus.
    pub last_ts: String,
}

/// One agent's events of one type on one UTC day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayTypeCount {
    /// UTC day index: epoch milliseconds divided by a day. Comparable and
    /// sortable; turn it back into a date at the edge, not here.
    pub day: i64,
    pub type_: String,
    pub count: u64,
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
    /// statement. Returns rows ACTUALLY written. Conformance is decided upstream
    /// (by [`crate::conform`]); a line that failed conformance never becomes a
    /// `ConsoleEvent`, so it never reaches this method, it goes to
    /// [`Store::quarantine`] instead.
    ///
    /// A line already stored under the same [`dedupe_key`] is skipped, and the
    /// returned count says so. That difference is the point: a caller reporting
    /// "42 ingested" when 40 of them were replays of bytes it already held
    /// would be describing work rather than data, and this store now outlives
    /// the process that fills it, so replays are the normal case rather than
    /// the exotic one.
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
                    "INSERT OR IGNORE INTO events \
                     (env, ts, source, type, agent_id, run_id, severity, schema, \
                      on_behalf_of, data, prev_hash, raw, file, off, dedupe, ts_ms) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
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

                let dedupe = dedupe_key(
                    &ce.provenance.env,
                    ce.provenance.file.as_deref(),
                    ce.provenance.offset,
                    &ce.raw,
                );
                let ts_ms = parse_ts_ms(&ce.event.ts);

                // `execute` returns rows CHANGED, which is 0 for a line this
                // store already holds. Summing that rather than counting the
                // loop is what keeps the reported number about data.
                inserted += stmt
                    .execute(params![
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
                        dedupe,
                        ts_ms,
                    ])
                    .map_err(store_err)?;
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

    /// Every event of one run, OLDEST-first by `id` (insertion order) - the
    /// timeline a Run Replay scrubs through (PHASE3 W4). Uses the existing
    /// `idx_events_run_id` index; `limit` caps a pathologically long run. This
    /// is the reverse of [`Store::recent_events`] / [`Store::events_for_agent`],
    /// so replay reads forward.
    ///
    /// Ordering caveat (found wiring Run Replay): `id` (insertion order) is NOT
    /// the same as wall-clock `ts` order across sources, because the ingest
    /// pipeline drains one bus file's whole backlog before the next - so a run
    /// spanning several sources lands grouped by source, not interleaved by
    /// time. A caller that wants true chronological playback sorts the returned
    /// rows by `ts` itself (both shells' Run Replay do). This method deals in
    /// stored order and stays cheap; it does not re-sort.
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

    /// Every event at or after `cutoff_ms`, newest first by `ts_ms`.
    ///
    /// The window is on the EVENT's own clock, not on when this console read
    /// the line: an operator asking "what happened in the last 24 hours" means
    /// the estate's 24 hours, and a console restarted an hour ago would
    /// otherwise answer for one hour and call it a day.
    ///
    /// Rows whose `ts` did not parse are absent, necessarily: they have no
    /// place on a timeline. [`Store::undated_count`] is how a caller says how
    /// many those were instead of quietly returning a smaller number.
    pub fn events_since(&self, cutoff_ms: i64, limit: usize) -> Result<Vec<StoredEvent>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, env, ts, source, type, agent_id, run_id, severity, schema, \
                 on_behalf_of, data, prev_hash, raw, file, off \
                 FROM events WHERE ts_ms IS NOT NULL AND ts_ms >= ?1 \
                 ORDER BY ts_ms DESC LIMIT ?2",
            )
            .map_err(store_err)?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut rows = stmt.query(params![cutoff_ms, limit]).map_err(store_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(store_err)? {
            out.push(stored_event_from_row(row)?);
        }
        Ok(out)
    }

    /// Every agent's event counts by type over a window, as ONE aggregate.
    ///
    /// `cutoff_ms` of `None` means every event the store holds, undated rows
    /// included; a `Some` window is on the event's own clock and undated rows
    /// are in no window at all ([`Store::undated_count`] is how a caller says
    /// how many those were).
    ///
    /// # WHY THIS EXISTS RATHER THAN COUNTING ROWS IN THE CALLER
    ///
    /// The Statistics panel used to read the N most recent rows and tally them
    /// in Rust, with N a cap chosen when this store was a scratch file holding
    /// a few thousand lines. Durable history turned that cap into a silent
    /// truncation. Measured on 2026-08-11: 42 agents at 100 events a day is
    /// 378,000 rows over ninety days, the frontend's cap was 20,000, so a
    /// question about thirty days was answered from about five per cent of it.
    /// The panel then said "counted from 20,000 events", which is true about
    /// itself and wrong about the question, and nothing in the shape let a
    /// reader tell "20,000 happened" from "we stopped at 20,000".
    ///
    /// An aggregate has no cap to hit. SQLite counts the rows and hands back
    /// one per (agent, type), which is tens of rows where the scan was hundreds
    /// of thousands, and the count is the whole window every time.
    pub fn type_counts_since(&self, cutoff_ms: Option<i64>) -> Result<Vec<AgentTypeCount>> {
        let sql = match cutoff_ms {
            Some(_) => {
                "SELECT agent_id, type, COUNT(*), MAX(ts) FROM events \
                 WHERE ts_ms IS NOT NULL AND ts_ms >= ?1 GROUP BY agent_id, type"
            }
            // `?1` is still bound, to one query shape rather than two: the
            // predicate is a constant true and SQLite drops it.
            None => {
                "SELECT agent_id, type, COUNT(*), MAX(ts) FROM events \
                 WHERE ?1 IS NULL GROUP BY agent_id, type"
            }
        };
        let mut stmt = self.conn.prepare(sql).map_err(store_err)?;
        let mut rows = stmt.query(params![cutoff_ms]).map_err(store_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(store_err)? {
            let count: i64 = row.get(2).map_err(store_err)?;
            out.push(AgentTypeCount {
                agent_id: row.get(0).map_err(store_err)?,
                type_: row.get(1).map_err(store_err)?,
                count: u64::try_from(count).unwrap_or(0),
                last_ts: row
                    .get::<_, Option<String>>(3)
                    .map_err(store_err)?
                    .unwrap_or_default(),
            });
        }
        Ok(out)
    }

    /// The `data` of the events a caller cannot answer from a count alone,
    /// newest first by insertion id, capped at `limit`.
    ///
    /// The companion to [`Store::type_counts_since`]. Some facts live in an
    /// event's `data` and cannot be aggregated: which detector fired, the two
    /// budget amounts, the agents one console block halted. Those need the row.
    ///
    /// Two ways to ask for them, and the second is the one that matters:
    ///
    /// - `types` names event types outright.
    /// - `json_pairs` names (field, field) pairs, and matches any event whose
    ///   `data` carries BOTH. The caller that reads budget overshoots does not
    ///   know every type that might carry amounts, and must not: the live
    ///   TokenFuse gateway records them on `breaker_tripped`, which is an
    ///   enforcement event and not a budget one. A named-types-only read would
    ///   answer "not recorded" for a real breach the moment a producer put the
    ///   amounts on a fourth type, which is the same silent-wrong this whole
    ///   method exists to remove.
    ///
    /// Field names are BOUND as parameters, never interpolated, so this stays
    /// safe if a caller ever computes them.
    ///
    /// The cap remains because "the events needing a row are a small minority"
    /// is a property of today's bus rather than a guarantee. Returning exactly
    /// `limit` rows is how the caller learns it was truncated, so it can say so
    /// instead of reporting the smaller number as a fact.
    pub fn events_of_types_since(
        &self,
        types: &[&str],
        json_pairs: &[(&str, &str)],
        cutoff_ms: Option<i64>,
        limit: usize,
    ) -> Result<Vec<EventDetail>> {
        if types.is_empty() && json_pairs.is_empty() {
            return Ok(Vec::new());
        }

        let mut binds: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let mut wants: Vec<String> = Vec::new();

        if !types.is_empty() {
            let holes = types
                .iter()
                .map(|t| {
                    binds.push(Box::new((*t).to_string()));
                    format!("?{}", binds.len())
                })
                .collect::<Vec<_>>()
                .join(",");
            wants.push(format!("type IN ({holes})"));
        }
        for (budget, spent) in json_pairs {
            binds.push(Box::new(format!("$.{budget}")));
            let b = binds.len();
            binds.push(Box::new(format!("$.{spent}")));
            let s = binds.len();
            // `json_valid` first: `json_extract` raises on a malformed value,
            // and one unparseable row must not take the whole panel down.
            wants.push(format!(
                "(data IS NOT NULL AND json_valid(data) \
                 AND json_extract(data, ?{b}) IS NOT NULL \
                 AND json_extract(data, ?{s}) IS NOT NULL)"
            ));
        }

        binds.push(Box::new(cutoff_ms));
        let cutoff = binds.len();
        binds.push(Box::new(i64::try_from(limit).unwrap_or(i64::MAX)));
        let lim = binds.len();

        let sql = format!(
            "SELECT agent_id, type, data FROM events WHERE ({}) \
             AND (?{cutoff} IS NULL OR (ts_ms IS NOT NULL AND ts_ms >= ?{cutoff})) \
             ORDER BY id DESC LIMIT ?{lim}",
            wants.join(" OR ")
        );

        let mut stmt = self.conn.prepare(&sql).map_err(store_err)?;
        let refs: Vec<&dyn rusqlite::ToSql> = binds.iter().map(|b| b.as_ref()).collect();
        let mut rows = stmt.query(refs.as_slice()).map_err(store_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(store_err)? {
            let data_json: Option<String> = row.get(2).map_err(store_err)?;
            out.push(EventDetail {
                agent_id: row.get(0).map_err(store_err)?,
                type_: row.get(1).map_err(store_err)?,
                data: data_json
                    .map(|s| serde_json::from_str(&s))
                    .transpose()
                    .map_err(store_err)?,
            });
        }
        Ok(out)
    }

    /// One agent's events bucketed by UTC day and by type, oldest day first.
    ///
    /// Raw (day, type, count) rather than anything categorised: which types
    /// count as "blocked" or "odd" is the console's reading of SPEC 6.2 and it
    /// lives in `genaryx_api::stats`, in one place. A store that also held an
    /// opinion about it would be a second copy to drift.
    ///
    /// Days with no events produce NO ROW. That is not the same as a zero, and
    /// the caller has to fill them: a median taken over only the days an agent
    /// was busy is a median of its busy days, which is exactly the number that
    /// makes a quiet agent's first bad day look normal.
    pub fn daily_type_counts(&self, agent_id: &str, since_ms: i64) -> Result<Vec<DayTypeCount>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT ts_ms / 86400000 AS day, type, COUNT(*) \
                 FROM events \
                 WHERE agent_id = ?1 AND ts_ms IS NOT NULL AND ts_ms >= ?2 \
                 GROUP BY day, type ORDER BY day ASC",
            )
            .map_err(store_err)?;
        let mut rows = stmt.query(params![agent_id, since_ms]).map_err(store_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(store_err)? {
            let count: i64 = row.get(2).map_err(store_err)?;
            out.push(DayTypeCount {
                day: row.get(0).map_err(store_err)?,
                type_: row.get(1).map_err(store_err)?,
                count: u64::try_from(count).unwrap_or(0),
            });
        }
        Ok(out)
    }

    /// The UTC day of an agent's FIRST stored event, or `None` when it has
    /// none. What "how long have we watched this agent" is measured from: an
    /// agent registered yesterday has no normal, however quiet it looks.
    pub fn first_day_for_agent(&self, agent_id: &str) -> Result<Option<i64>> {
        let day: Option<i64> = self
            .conn
            .query_row(
                "SELECT MIN(ts_ms) / 86400000 FROM events \
                 WHERE agent_id = ?1 AND ts_ms IS NOT NULL",
                params![agent_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(store_err)?
            .flatten();
        Ok(day)
    }

    /// How many stored events carry no usable timestamp, and so appear in no
    /// window. Small or zero on a healthy bus; a number that climbs means a
    /// producer is writing a `ts` this build cannot read, which is worth
    /// showing rather than absorbing.
    pub fn undated_count(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM events WHERE ts_ms IS NULL", [], |r| {
                r.get(0)
            })
            .map_err(store_err)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    /// The oldest and newest event timestamps this store holds, in epoch
    /// milliseconds, or `None` when it holds no dated event at all.
    ///
    /// What a window label is built from. "Last 30 days" over a store that has
    /// only been running for two is a true statement and a misleading one, and
    /// the only cure is saying how far back the data actually goes.
    pub fn ts_span(&self) -> Result<Option<(i64, i64)>> {
        let row: Option<(Option<i64>, Option<i64>)> = self
            .conn
            .query_row(
                "SELECT MIN(ts_ms), MAX(ts_ms) FROM events WHERE ts_ms IS NOT NULL",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(store_err)?;
        Ok(match row {
            Some((Some(lo), Some(hi))) => Some((lo, hi)),
            _ => None,
        })
    }

    /// Drop events and quarantined lines older than `cutoff_ms`. Returns
    /// `(events, quarantined)` actually removed.
    ///
    /// Undated events are NEVER pruned by age, because there is no age to
    /// compare: dropping them would be deleting on a guess. They are the one
    /// thing in this store that can only grow, which is the other half of why
    /// [`Store::undated_count`] is surfaced.
    ///
    /// Quarantine is pruned on its own `ts`, which this console wrote itself
    /// when it rejected the line, so it is always parseable.
    pub fn prune_before(&self, cutoff_ms: i64) -> Result<(u64, u64)> {
        let events = self
            .conn
            .execute(
                "DELETE FROM events WHERE ts_ms IS NOT NULL AND ts_ms < ?1",
                params![cutoff_ms],
            )
            .map_err(store_err)?;
        let cutoff_rfc = crate::store::ms_to_rfc3339(cutoff_ms);
        let quarantined = self
            .conn
            .execute(
                "DELETE FROM event_quarantine WHERE ts < ?1",
                params![cutoff_rfc],
            )
            .map_err(store_err)?;
        Ok((events as u64, quarantined as u64))
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

    /// Quarantined lines grouped by the reason they were refused, worst first.
    ///
    /// # WHY A READ PATH EXISTS AT ALL
    ///
    /// It did not, until 2026-08-11, and that is the whole point of this
    /// method. Lines that fail conformance have always been stored here with
    /// their file, offset, raw bytes and reason, on the principle in this
    /// module's own doc that a malformed line must never silently vanish. Then
    /// nothing ever read them back. [`Store::quarantine_count`] existed and no
    /// caller used it, and the only report anywhere was one `eprintln!` at
    /// startup, to stderr, once.
    ///
    /// So the line did not vanish, and the operator could not tell the
    /// difference. The real case that proved it: the `aws-comparable-176`
    /// benchmark campaign emitted all twelve of its events with
    /// `agent_id: "aws-comparable-agent"`, no `agent://` prefix, and on this
    /// console that producer's whole run lands here and shows up as an agent
    /// that did nothing. A panel counting zero blocks for it would be exactly
    /// right about what is on the bus and exactly wrong about what happened.
    ///
    /// One EXAMPLE per reason rather than every row: the reasons repeat (a
    /// producer with a bad id emits the same fault on every line), and what an
    /// operator needs is which producer, which file, and how many, not twelve
    /// copies of one sentence. `raw_excerpt` is capped at
    /// [`QUARANTINE_EXCERPT_CHARS`] because these are whole event lines and the
    /// point is to recognize the producer, not to read its payload here.
    pub fn quarantine_by_reason(&self, limit: usize) -> Result<Vec<QuarantineReason>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT reason, COUNT(*), MAX(ts), \
                 MAX(file), MAX(off), MAX(raw) \
                 FROM event_quarantine GROUP BY reason \
                 ORDER BY COUNT(*) DESC, MAX(ts) DESC LIMIT ?1",
            )
            .map_err(store_err)?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut rows = stmt.query(params![limit]).map_err(store_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(store_err)? {
            let count: i64 = row.get(1).map_err(store_err)?;
            let raw: Option<String> = row.get(5).map_err(store_err)?;
            out.push(QuarantineReason {
                reason: row.get(0).map_err(store_err)?,
                count: u64::try_from(count).unwrap_or(0),
                last_ts: row.get::<_, Option<String>>(2).map_err(store_err)?,
                example_file: row.get(3).map_err(store_err)?,
                example_offset: row
                    .get::<_, Option<i64>>(4)
                    .map_err(store_err)?
                    .and_then(|v| u64::try_from(v).ok()),
                raw_excerpt: raw.map(|r| excerpt(&r, QUARANTINE_EXCERPT_CHARS)),
            });
        }
        Ok(out)
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
        Ok(self.get_source_state(file)?.map(|s| s.offset))
    }

    /// The journaled offset for `file` AND the inode it was read from.
    ///
    /// The inode is what makes a durable offset safe to resume from. An offset
    /// alone says "I had read 5000 bytes"; it does not say 5000 bytes of WHAT.
    /// A file rotated away and replaced while the console was down has a fresh
    /// inode and its own byte 5000, and resuming there would skip everything
    /// before it and then read from the middle of a line. `FileTail`'s existing
    /// shorter-than-my-offset check catches the case where the replacement is
    /// smaller and catches nothing when it is larger, which is the case that
    /// silently loses events.
    pub fn get_source_state(&self, file: &str) -> Result<Option<SourceState>> {
        let row: Option<(i64, Option<i64>, Option<String>)> = self
            .conn
            .query_row(
                "SELECT offset, inode, head_sha FROM source_offsets WHERE file = ?1",
                params![file],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(store_err)?;
        let Some((offset, inode, head_sha)) = row else {
            return Ok(None);
        };
        Ok(Some(SourceState {
            offset: from_i64(offset)?,
            inode: inode.map(|i| i as u64),
            head_sha,
        }))
    }

    /// Journal the read offset for `file` (upsert: the latest call wins).
    pub fn set_offset(
        &self,
        file: &str,
        offset: u64,
        inode: Option<u64>,
        head_sha: Option<&str>,
    ) -> Result<()> {
        let offset = to_i64(offset)?;
        let inode = inode.map(to_i64).transpose()?;
        self.conn
            .execute(
                "INSERT INTO source_offsets (file, offset, inode, head_sha) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(file) DO UPDATE SET offset = excluded.offset, \
                 inode = excluded.inode, head_sha = excluded.head_sha",
                params![file, offset, inode, head_sha],
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
    if current < 3 {
        // `ALTER TABLE ... ADD COLUMN` has no `IF NOT EXISTS`, so unlike the
        // two migrations above this one is not safe to re-run. The version gate
        // is what makes it run once; a store already at 3 skips it entirely.
        conn.execute_batch(MIGRATION_V3).map_err(store_err)?;
    }
    if current < 4 {
        conn.execute_batch(MIGRATION_V4).map_err(store_err)?;
    }
    if current < 5 {
        conn.execute_batch(MIGRATION_V5).map_err(store_err)?;
    }
    if current < 6 {
        conn.execute_batch(MIGRATION_V6).map_err(store_err)?;
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

/// How much of a file's already-consumed head is fingerprinted.
///
/// 64 KiB is far more than any event file's first lines and small enough to
/// re-read on every resume without noticing. When the journaled offset is
/// smaller than this, the fingerprint covers the ENTIRE consumed prefix and the
/// check is exact rather than a heuristic, which is the ordinary case for these
/// files.
///
/// The residual gap, stated rather than left to be found: a replacement whose
/// first 64 KiB are byte-identical to the old file's and which then diverges
/// would be resumed rather than re-read. Nothing in this estate rewrites a log
/// that way, and the alternative (hashing the whole prefix each poll) costs
/// real work on every tick to close a case nobody has.
pub const HEAD_FINGERPRINT_BYTES: u64 = 64 * 1024;

/// Fingerprint of the first `min(offset, HEAD_FINGERPRINT_BYTES)` bytes of
/// `path`, or `None` when the file cannot be read or is shorter than `offset`
/// (which is itself a signal the caller handles separately).
///
/// This is what tells "the file I was reading" from "a different file at the
/// same path". The inode cannot: Linux reuses inode numbers, so a rotated file
/// routinely comes back wearing its predecessor's.
pub fn head_fingerprint(path: &Path, offset: u64) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    if offset == 0 {
        return None;
    }
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len < offset {
        // Shorter than what we have read: a truncation, which `FileTail`
        // already treats as "start again". No fingerprint to compare.
        return None;
    }
    let span = offset.min(HEAD_FINGERPRINT_BYTES);
    let mut buf = vec![0u8; span as usize];
    file.read_exact(&mut buf).ok()?;
    let mut h = Sha256::new();
    h.update(&buf);
    Some(format!("{:x}", h.finalize()))
}

/// The uniqueness of one stored line: where it came from and what it said.
///
/// `env` is in the key because two environments legitimately hold byte-identical
/// lines and are not the same event. `file` and `offset` are in it because two
/// genuinely identical lines at two positions in a file are two events and both
/// must land. `raw` is in it because after a truncation the same offset holds
/// different bytes, and only the content can tell those apart.
///
/// A record with no file position (a future non-file source) hashes its env and
/// content alone, so an SSE stream replaying the same line twice would dedupe.
/// That is the safe direction for a source with no position to compare.
///
/// THE TRADE-OFF, stated because it is a real one and it goes both ways.
/// Including the offset means a file that is rotated and then rewritten with
/// the SAME lines at DIFFERENT positions stores them twice. Excluding it would
/// mean two byte-identical lines in one file collapse into one. The second is
/// the likelier of the two here: wardryx stamps `ts` to the whole second, so
/// two identical `policy_deny` lines inside one second are an ordinary thing
/// under load, and losing one would under-report an enforcement. A producer
/// replaying its own old log verbatim into a fresh file is not something
/// anything in this estate does. So the offset stays in, and the failure this
/// design can still have is over-counting a rotation nobody performs, not
/// under-counting a block that happened.
pub fn dedupe_key(env: &str, file: Option<&str>, offset: Option<u64>, raw: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    // A separator that cannot occur in any of the parts, so ("a","bc") and
    // ("ab","c") cannot hash alike.
    h.update(env.as_bytes());
    h.update([0u8]);
    h.update(file.unwrap_or("").as_bytes());
    h.update([0u8]);
    h.update(offset.map(|o| o.to_string()).unwrap_or_default().as_bytes());
    h.update([0u8]);
    h.update(raw.as_bytes());
    format!("{:x}", h.finalize())
}

/// An RFC 3339 timestamp as epoch milliseconds, or `None` when it will not
/// parse.
///
/// `None` rather than a fallback to "now": an event stamped with something this
/// build cannot read is an event of unknown time, and filing it under the
/// moment it happened to be ingested would put it in windows it does not belong
/// to and make a producer's broken clock invisible.
pub fn parse_ts_ms(ts: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Epoch milliseconds back to the RFC 3339 spelling the text columns use.
pub fn ms_to_rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or(chrono::DateTime::UNIX_EPOCH)
        .to_rfc3339()
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
