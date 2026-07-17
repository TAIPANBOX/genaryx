//! `VerdryxClient`: a read-only reader for Verdryx's quality-plane store
//! (docs/PHASE4.md W1) - the eval-quality plane the console's Quality panel
//! renders from. Grounded in the verdryx Python source (`~/Development/verdryx`,
//! read 2026-07-17), which is the load-bearing fact that shapes this connector:
//!
//! ## Verdryx has no machine output, so this is a store reader, not a CLI wrapper
//!
//! Unlike [`crate::QryxClient`] (a `--format`-JSON CLI) and the REST clients,
//! **Verdryx exposes no JSON/`--format`/`--json` on any subcommand** - `eval`,
//! `baseline`, `drift`, and `cost-per-correct` print human text only
//! (`verdryx/cli.py`). Its durable, machine-readable surface is its SQLite store
//! (`verdryx/store.py`) plus the `verdryx.*` events it emits to the shared bus.
//! So the Quality panel's history and per-case scores come from reading
//! `verdryx.db` directly here (the console already links `rusqlite` for its own
//! [`genaryx_core`] Store), and its live drift alerts come from the
//! `quality_drift` bus event (tailed by genaryx-core), NOT from this reader.
//!
//! ## Strictly read-only, and why WAL-at-rest is the expectation
//!
//! This opens `verdryx.db` with `SQLITE_OPEN_READ_ONLY` and issues only
//! `SELECT`s - the console never writes another service's store (it mutates
//! planes only through signed commands to Cloud/Wardryx, never by touching a
//! peer's database). Verdryx is a **batch CLI** (run `eval`, write scores, exit),
//! not a long-lived server holding the DB open, so by the time the console reads
//! it the DB is at rest and its WAL is checkpointed on clean exit - a read-only
//! open succeeds. If verdryx crashed mid-`eval` and left an uncheckpointed
//! `-wal`, a read-only open can fail; that surfaces as [`VerdryxError::Open`]
//! (fail-closed: the panel shows "can't read verdryx.db," it does NOT silently
//! read a torn/stale snapshot by forcing an `immutable=1` open).
//!
//! ## Schema (verdryx/store.py:19-45, verbatim)
//!
//! - `eval_runs(id TEXT PK, model TEXT, started_at TEXT, finished_at TEXT NULL)`
//! - `scores(id INTEGER PK, run_id TEXT ->eval_runs.id, case_id TEXT, value REAL,
//!   tokens INTEGER, cost_usd REAL)`, index `idx_scores_run_id`.
//! - `baselines(id TEXT PK, eval_run_id TEXT ->eval_runs.id, mean_score REAL,
//!   created_at TEXT, label TEXT)`.
//!
//! All timestamps are canonical UTC ISO-8601 strings (`store.py:_iso` ->
//! `datetime.astimezone(UTC).isoformat()`); kept as `String` for the panel to
//! format. `finished_at` is `NULL` while a run is in flight, so it is
//! `Option<String>`.
//!
//! ## Fail-closed (06 §0.5)
//!
//! No panics, no `unwrap`/`expect`. A failed open becomes [`VerdryxError::Open`]
//! (carrying the path); any query/row-decode failure becomes
//! [`VerdryxError::Query`]. `run_summary`'s aggregate over a run with no scores
//! returns a summary with `case_count == 0` and a `mean_score` of `None`, never
//! a divide-by-zero or a fabricated 0.0 that a panel could misread as "scored
//! zero."

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

// ---- error -----------------------------------------------------------------

/// Every failure mode a [`VerdryxClient`] call can surface. Fail-closed: an
/// open failure and a query/decode failure are distinct variants, never a
/// panic or a silently-empty result.
#[derive(Debug, thiserror::Error)]
pub enum VerdryxError {
    /// `verdryx.db` could not be opened read-only (missing file, permissions,
    /// or an uncheckpointed `-wal` from a crashed `eval`). Carries the path so
    /// the panel can tell the operator exactly which store it could not read.
    #[error("open verdryx db {path}: {source}")]
    Open {
        path: String,
        #[source]
        source: rusqlite::Error,
    },

    /// A prepared statement, query, or row decode failed - the schema this
    /// reader expects (`store.py:19-45`) has drifted from the live `verdryx.db`,
    /// or a row held an unexpected type.
    #[error("query verdryx db: {0}")]
    Query(#[from] rusqlite::Error),
}

// ---- DTOs (exact rows, verdryx/store.py:19-45) ------------------------------

/// One row of `eval_runs` (`store.py:20-25`). One invocation of `verdryx eval`
/// against a dataset with a given model.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct EvalRun {
    pub id: String,
    /// The model under eval, e.g. `claude-sonnet-5` (`eval_runs.model`).
    pub model: String,
    /// UTC ISO-8601 when the run started (`eval_runs.started_at`).
    pub started_at: String,
    /// UTC ISO-8601 when the run finished, or `None` while still in flight
    /// (`eval_runs.finished_at`, the one nullable column).
    pub finished_at: Option<String>,
}

/// One row of `scores` (`store.py:27-34`): one case's quality score within a
/// run, with its token/cost accounting.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Score {
    /// The autoincrement row id (`scores.id`).
    pub id: i64,
    /// FK to [`EvalRun::id`] (`scores.run_id`).
    pub run_id: String,
    /// The dataset case this score is for (`scores.case_id`).
    pub case_id: String,
    /// The quality score in `[0.0, 1.0]` (`scores.value`).
    pub value: f64,
    /// Tokens consumed scoring this case (`scores.tokens`, default 0).
    pub tokens: i64,
    /// USD cost of scoring this case (`scores.cost_usd`, default 0.0).
    pub cost_usd: f64,
}

/// One row of `baselines` (`store.py:38-44`): a saved mean-score snapshot a
/// later run's `drift` is measured against.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Baseline {
    pub id: String,
    /// FK to the [`EvalRun`] this baseline was snapshotted from
    /// (`baselines.eval_run_id`).
    pub eval_run_id: String,
    /// The pooled mean score at snapshot time (`baselines.mean_score`).
    pub mean_score: f64,
    /// UTC ISO-8601 when the baseline was saved (`baselines.created_at`).
    pub created_at: String,
    /// A human label, e.g. `v1-golden`; empty string when unset
    /// (`baselines.label`, default `''`).
    pub label: String,
}

/// A derived per-run rollup the Quality panel's headline shows. Computed here
/// (not stored) by aggregating [`Score`]s, so it always matches the rows.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RunSummary {
    pub run: EvalRun,
    /// Number of scored cases in the run.
    pub case_count: u64,
    /// Mean of `scores.value` over the run, or `None` when the run has no
    /// scores yet (never a fabricated 0.0).
    pub mean_score: Option<f64>,
    pub total_tokens: i64,
    pub total_cost_usd: f64,
}

// ---- client ----------------------------------------------------------------

/// A read-only reader for `verdryx.db`. Holds one `rusqlite` connection opened
/// `SQLITE_OPEN_READ_ONLY`; every method is a single `SELECT`. Not `Send`-shared
/// across threads by design (a `Connection` is `!Sync`); the shells open one per
/// read context, mirroring how `verdryx` itself uses a single connection.
#[derive(Debug)]
pub struct VerdryxClient {
    conn: Connection,
}

impl VerdryxClient {
    /// Open `verdryx.db` at `path` strictly read-only. Sets a 5s busy-timeout so
    /// a concurrent (rare, batch-CLI) writer briefly holding the write lock does
    /// not immediately fail the read. Returns [`VerdryxError::Open`] on any open
    /// failure (see the module doc on WAL-at-rest).
    pub fn open(path: impl AsRef<Path>) -> Result<Self, VerdryxError> {
        let path_ref = path.as_ref();
        let path_str = path_ref.display().to_string();
        let conn = Connection::open_with_flags(
            path_ref,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|source| VerdryxError::Open {
            path: path_str.clone(),
            source,
        })?;
        conn.busy_timeout(Duration::from_millis(5000))
            .map_err(|source| VerdryxError::Open {
                path: path_str,
                source,
            })?;
        Ok(Self { conn })
    }

    /// Every `eval_runs` row, newest-started first.
    pub fn list_eval_runs(&self) -> Result<Vec<EvalRun>, VerdryxError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, model, started_at, finished_at FROM eval_runs ORDER BY started_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(EvalRun {
                id: row.get(0)?,
                model: row.get(1)?,
                started_at: row.get(2)?,
                finished_at: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// The most-recently-started run, or `None` on an empty store.
    pub fn latest_run(&self) -> Result<Option<EvalRun>, VerdryxError> {
        Ok(self.list_eval_runs()?.into_iter().next())
    }

    /// Every [`Score`] for one run, in insertion order (`scores.id` asc, which
    /// is case-evaluation order within the run).
    pub fn scores_for_run(&self, run_id: &str) -> Result<Vec<Score>, VerdryxError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_id, case_id, value, tokens, cost_usd FROM scores \
             WHERE run_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([run_id], |row| {
            Ok(Score {
                id: row.get(0)?,
                run_id: row.get(1)?,
                case_id: row.get(2)?,
                value: row.get(3)?,
                tokens: row.get(4)?,
                cost_usd: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Every `baselines` row, newest-created first.
    pub fn list_baselines(&self) -> Result<Vec<Baseline>, VerdryxError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, eval_run_id, mean_score, created_at, label FROM baselines \
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Baseline {
                id: row.get(0)?,
                eval_run_id: row.get(1)?,
                mean_score: row.get(2)?,
                created_at: row.get(3)?,
                label: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// A [`RunSummary`] for one run id, or `None` if no such run exists. The
    /// aggregate is computed in SQL over `scores`; a run with zero scores yields
    /// `case_count == 0` and `mean_score == None` (SQL `AVG` of no rows is
    /// `NULL`), never a divide-by-zero or a fabricated mean.
    pub fn run_summary(&self, run_id: &str) -> Result<Option<RunSummary>, VerdryxError> {
        let run = {
            let mut stmt = self.conn.prepare(
                "SELECT id, model, started_at, finished_at FROM eval_runs WHERE id = ?1",
            )?;
            let mut rows = stmt.query_map([run_id], |row| {
                Ok(EvalRun {
                    id: row.get(0)?,
                    model: row.get(1)?,
                    started_at: row.get(2)?,
                    finished_at: row.get(3)?,
                })
            })?;
            match rows.next() {
                Some(r) => r?,
                None => return Ok(None),
            }
        };

        let (case_count, mean_score, total_tokens, total_cost_usd) = self.conn.query_row(
            "SELECT COUNT(*), AVG(value), COALESCE(SUM(tokens), 0), COALESCE(SUM(cost_usd), 0.0) \
             FROM scores WHERE run_id = ?1",
            [run_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, Option<f64>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, f64>(3)?,
                ))
            },
        )?;

        Ok(Some(RunSummary {
            run,
            case_count,
            mean_score,
            total_tokens,
            total_cost_usd,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build a real verdryx.db (writer connection, store.py's exact DDL), then
    // read it back through a strictly-read-only VerdryxClient - proving the SQL
    // + DTO mapping and the read-only open path against genuine SQLite. A live
    // reader against a real `verdryx eval` DB lives in tests/, skip-gracefully.

    // store.py:19-45, verbatim.
    const DDL: &str = "
        CREATE TABLE eval_runs (id TEXT PRIMARY KEY, model TEXT NOT NULL,
            started_at TEXT NOT NULL, finished_at TEXT);
        CREATE TABLE scores (id INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id TEXT NOT NULL REFERENCES eval_runs(id), case_id TEXT NOT NULL,
            value REAL NOT NULL, tokens INTEGER NOT NULL DEFAULT 0,
            cost_usd REAL NOT NULL DEFAULT 0.0);
        CREATE INDEX idx_scores_run_id ON scores(run_id);
        CREATE TABLE baselines (id TEXT PRIMARY KEY,
            eval_run_id TEXT NOT NULL REFERENCES eval_runs(id), mean_score REAL NOT NULL,
            created_at TEXT NOT NULL, label TEXT NOT NULL DEFAULT '');
    ";

    fn seed_db() -> std::path::PathBuf {
        // Unique temp path (process id + an atomic counter; no wall-clock, no
        // extra deps). Default rollback-journal mode: the writer is dropped
        // before the reader opens, so the file is self-contained and the
        // read-only open trivially succeeds.
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "genaryx-verdryx-test-{}-{}.db",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let w = Connection::open(&path).expect("open writer");
        w.execute_batch(DDL).expect("ddl");
        w.execute(
            "INSERT INTO eval_runs VALUES ('run-1','claude-sonnet-5','2026-07-17T10:00:00+00:00','2026-07-17T10:05:00+00:00')",
            [],
        )
        .unwrap();
        // A run still in flight: finished_at NULL.
        w.execute(
            "INSERT INTO eval_runs VALUES ('run-2','claude-opus-4-8','2026-07-17T11:00:00+00:00',NULL)",
            [],
        )
        .unwrap();
        w.execute("INSERT INTO scores (run_id,case_id,value,tokens,cost_usd) VALUES ('run-1','c1',1.0,120,0.004)", []).unwrap();
        w.execute("INSERT INTO scores (run_id,case_id,value,tokens,cost_usd) VALUES ('run-1','c2',0.5,80,0.002)", []).unwrap();
        w.execute(
            "INSERT INTO baselines VALUES ('bl-1','run-1',0.75,'2026-07-17T10:06:00+00:00','v1-golden')",
            [],
        )
        .unwrap();
        drop(w);
        path
    }

    fn cleanup(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn eval_runs_read_newest_first_with_null_finished() {
        let path = seed_db();
        let c = VerdryxClient::open(&path).expect("open ro");
        let runs = c.list_eval_runs().expect("list");
        assert_eq!(runs.len(), 2);
        // Newest started first: run-2 (11:00) before run-1 (10:00).
        assert_eq!(runs[0].id, "run-2");
        assert_eq!(runs[0].model, "claude-opus-4-8");
        assert!(
            runs[0].finished_at.is_none(),
            "in-flight run has NULL finished_at"
        );
        assert_eq!(runs[1].id, "run-1");
        assert_eq!(
            runs[1].finished_at.as_deref(),
            Some("2026-07-17T10:05:00+00:00")
        );
        assert_eq!(c.latest_run().unwrap().unwrap().id, "run-2");
        cleanup(&path);
    }

    #[test]
    fn scores_and_summary_aggregate() {
        let path = seed_db();
        let c = VerdryxClient::open(&path).expect("open ro");
        let scores = c.scores_for_run("run-1").expect("scores");
        assert_eq!(scores.len(), 2);
        assert_eq!(scores[0].case_id, "c1");
        assert_eq!(scores[0].value, 1.0);

        let s = c
            .run_summary("run-1")
            .expect("summary")
            .expect("run exists");
        assert_eq!(s.case_count, 2);
        assert_eq!(s.mean_score, Some(0.75)); // (1.0 + 0.5) / 2
        assert_eq!(s.total_tokens, 200);
        assert!((s.total_cost_usd - 0.006).abs() < 1e-9);
        cleanup(&path);
    }

    #[test]
    fn summary_of_run_with_no_scores_is_none_mean_not_zero() {
        let path = seed_db();
        let c = VerdryxClient::open(&path).expect("open ro");
        // run-2 has no scores: count 0, mean None (never a fabricated 0.0).
        let s = c
            .run_summary("run-2")
            .expect("summary")
            .expect("run exists");
        assert_eq!(s.case_count, 0);
        assert_eq!(s.mean_score, None);
        assert_eq!(s.total_tokens, 0);
        assert_eq!(s.total_cost_usd, 0.0);
        // A run id that does not exist -> Ok(None), not an error.
        assert!(c.run_summary("nope").expect("no err").is_none());
        cleanup(&path);
    }

    #[test]
    fn baselines_read() {
        let path = seed_db();
        let c = VerdryxClient::open(&path).expect("open ro");
        let bls = c.list_baselines().expect("baselines");
        assert_eq!(bls.len(), 1);
        assert_eq!(bls[0].id, "bl-1");
        assert_eq!(bls[0].eval_run_id, "run-1");
        assert_eq!(bls[0].mean_score, 0.75);
        assert_eq!(bls[0].label, "v1-golden");
        cleanup(&path);
    }

    #[test]
    fn open_missing_db_is_fail_closed() {
        let path = std::env::temp_dir().join("genaryx-verdryx-does-not-exist.db");
        let _ = std::fs::remove_file(&path);
        match VerdryxClient::open(&path) {
            Err(VerdryxError::Open { .. }) => {}
            other => panic!("expected Open error, got {other:?}"),
        }
    }
}
