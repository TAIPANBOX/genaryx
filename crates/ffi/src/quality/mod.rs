//! `QualityHandle`: the UniFFI Object wrapping
//! `genaryx_connectors::VerdryxClient` for the SwiftUI Quality surface
//! (docs/PHASE4.md W1, "Track B `crates/ffi/src/quality/`"), at parity with
//! the Tauri shell's own Quality panel (the sibling Track A). Structurally
//! this is the simplest handle in this crate so far, for one reason: Verdryx
//! is a **synchronous SQLite reader** (`VerdryxClient::open` is a plain,
//! local, blocking call - `crates/connectors/src/verdryx.rs`'s own doc: "not
//! a long-lived server holding the DB open"), so - unlike [`crate::idryx::IdryxHandle`]'s
//! REST reads or [`crate::wardryx::WardryxHandle`]'s bearer calls - this
//! handle needs no owned `tokio::runtime::Runtime` and no `block_on` at all.
//! It is the same shape as `IdryxClient::rescan` (already sync) rather than
//! `IdryxClient::list_identities` (async, `block_on`-wrapped).
//!
//! ## No persistent connection: open fresh, per call
//!
//! [`VerdryxClient`] wraps a `rusqlite::Connection`, which is `Send` but
//! **not `Sync`** (its own module doc: "the shells open one per read
//! context, mirroring how verdryx itself uses a single connection"). Rather
//! than wrap a held-open client in a `Mutex` (the [`crate::FleetHandle`]
//! pattern for its own non-`Sync` `Store`), this handle stores only the
//! resolved, trivially `Send + Sync` `db_path`/`source` and opens a fresh
//! [`VerdryxClient`] inside EVERY exported method, dropping it before the
//! call returns. Two deliberate benefits, not just a `Sync`-avoidance
//! workaround: every read always sees whatever is on disk AT THAT MOMENT
//! (a `verdryx eval` that finishes mid-session is visible on the very next
//! call, with no cache to invalidate), and there is no lock an operator's
//! reads can ever contend on. `VerdryxClient::open` is cheap - a read-only
//! SQLite open plus a busy-timeout - so paying it per call is not a
//! meaningful cost at this data volume.
//!
//! ## Fail-closed, and what "absent" means here
//!
//! No panics, no `unwrap`/`expect`. [`QualityHandle::discover`] fails closed
//! with [`QualityError::NoEnvironment`] when [`env::discover`] cannot even
//! NAME a candidate `verdryx.db` (docs/PHASE4.md W1: "An absent source (no
//! `verdryx.db`...) must render as an HONEST first-class empty state, never a
//! fake-empty-success"). A candidate that WAS named (`VERDRYX_DB`, or an
//! operator-supplied path via [`QualityHandle::connect`]) but does not open
//! surfaces distinctly, as [`QualityError::Open`], on the first read that
//! tries it - never silently downgraded to the same "no environment" empty
//! state, since those are different facts the operator needs to see
//! differently (PHASE3.md's `IdryxConnection` precedent: `.noEnvironment` and
//! `.connectFailed(reason:)` are rendered as two distinct empty states, not
//! collapsed into one).
//!
//! ## Drift alerts are NOT read here
//!
//! The Quality panel's drift alerts come from the live `quality_drift` bus
//! event (source `verdryx`, high severity - docs/PHASE4.md W1 grounding:
//! "Its LIVE drift signal is the `quality_drift` bus event... already in the
//! console's Store/bus that `FleetHandle` exposes"), NOT from this handle.
//! The SwiftUI `QualityModel`/`QualityView` filter `FleetModel`'s existing
//! event feed by `source == "verdryx"` / `eventType == "quality_drift"`,
//! exactly the way `PolicyView`'s Decision Stream filters the same shared
//! feed by `source == "wardryx"` - never a second read through this handle.
//! So this Object exports no drift-related method at all.

pub mod dto;
pub mod env;

pub use dto::{BaselineRecord, EvalRunRecord, QualityError, RunSummaryRecord, ScoreRecord};
pub use env::QualityEnvSource;

use env::ResolvedEnv;
use genaryx_connectors::VerdryxClient;
use std::path::PathBuf;

/// The Quality UniFFI Object: a resolved `verdryx.db` location, opened fresh
/// per call. See the module doc for why no `VerdryxClient`/`Connection` is
/// held on `self`.
#[derive(uniffi::Object)]
pub struct QualityHandle {
    source: QualityEnvSource,
    db_path: PathBuf,
}

#[uniffi::export]
impl QualityHandle {
    /// Discover which `verdryx.db` to read: [`env::discover`]'s three-tier
    /// fallback (`VERDRYX_DB`, the well-known taipan path, verdryx's own cwd
    /// default). Fails closed with [`QualityError::NoEnvironment`] when none
    /// resolves - a normal, renderable "no quality plane" outcome, not a bug
    /// (see the module doc). Never opens the database itself: exactly like
    /// `IdryxHandle::discover`/`connect` never touch the network at
    /// construction, this never touches the filesystem beyond the cheap
    /// existence checks `env::discover` already performed.
    #[uniffi::constructor]
    pub fn discover() -> Result<Self, QualityError> {
        let resolved = env::discover().ok_or(QualityError::NoEnvironment)?;
        Ok(Self::build(resolved))
    }

    /// Point directly at `db_path`, skipping discovery - for a `verdryx.db`
    /// the operator names explicitly (a text field in the empty state, or a
    /// test harness). Always reports [`QualityEnvSource::Explicit`], mirroring
    /// `IdryxHandle::connect`'s own dual use of `EnvFallback`.
    #[uniffi::constructor]
    pub fn connect(db_path: String) -> Self {
        Self::build(ResolvedEnv {
            source: QualityEnvSource::Explicit,
            db_path: PathBuf::from(db_path),
        })
    }

    /// Where this handle resolved its `verdryx.db` from.
    pub fn source(&self) -> QualityEnvSource {
        self.source.clone()
    }

    /// The resolved `verdryx.db` path this handle reads.
    pub fn db_path(&self) -> String {
        self.db_path.display().to_string()
    }

    // ---- reads (the whole surface - Quality is read-only this wave) -------

    /// Every `eval_runs` row, newest-started first - the Eval Runs history
    /// list's base rows (model, started/finished). Per-run summaries (case
    /// count, mean score, total cost) are a SEPARATE call per run
    /// ([`Self::run_summary`]) rather than folded in here: the connector has
    /// no bulk-summary query, and fetching summaries is the caller's choice
    /// of how many rows deep to pay for (the Swift model bounds this - see
    /// `QualityModel.swift`).
    pub fn list_eval_runs(&self) -> Result<Vec<EvalRunRecord>, QualityError> {
        let client = self.open()?;
        Ok(client
            .list_eval_runs()?
            .iter()
            .map(EvalRunRecord::from)
            .collect())
    }

    /// The most-recently-started run, or `None` on an empty store - the Eval
    /// Runs history's default selection.
    pub fn latest_run(&self) -> Result<Option<EvalRunRecord>, QualityError> {
        let client = self.open()?;
        Ok(client.latest_run()?.as_ref().map(EvalRunRecord::from))
    }

    /// Every per-case [`ScoreRecord`] for one run, in evaluation order - the
    /// run-detail view's scores table.
    pub fn scores_for_run(&self, run_id: String) -> Result<Vec<ScoreRecord>, QualityError> {
        let client = self.open()?;
        Ok(client
            .scores_for_run(&run_id)?
            .iter()
            .map(ScoreRecord::from)
            .collect())
    }

    /// Every `baselines` row, newest-created first - the Baselines list.
    pub fn list_baselines(&self) -> Result<Vec<BaselineRecord>, QualityError> {
        let client = self.open()?;
        Ok(client
            .list_baselines()?
            .iter()
            .map(BaselineRecord::from)
            .collect())
    }

    /// The [`RunSummaryRecord`] rollup for one run id, or `None` if no such
    /// run exists - feeds BOTH the history list's per-row summary columns and
    /// the run-detail header. `mean_score` is `None` (never a fabricated
    /// `0.0`) when the run has no scores yet; the Swift panel renders that as
    /// "n/a" (docs/PHASE4.md W1 guard).
    pub fn run_summary(&self, run_id: String) -> Result<Option<RunSummaryRecord>, QualityError> {
        let client = self.open()?;
        Ok(client
            .run_summary(&run_id)?
            .as_ref()
            .map(RunSummaryRecord::from))
    }
}

// ---- private helpers (not exported over FFI) -------------------------------

impl QualityHandle {
    fn build(resolved: ResolvedEnv) -> Self {
        Self {
            source: resolved.source,
            db_path: resolved.db_path,
        }
    }

    /// Open a fresh [`VerdryxClient`] against `self.db_path` - see the module
    /// doc's "open fresh, per call". A failed open becomes [`QualityError::Open`]
    /// via `From<ConnVerdryxError>`, carrying the exact path this handle
    /// resolved (or the operator supplied), never a panic.
    fn open(&self) -> Result<VerdryxClient, QualityError> {
        Ok(VerdryxClient::open(&self.db_path)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rust-side stand-in proving `QualityHandle` never panics when discovery
    /// finds nothing - the common case in CI (no `verdryx.db` anywhere on the
    /// box). Mirrors
    /// `idryx::tests::discover_without_an_environment_is_a_clean_error_not_a_panic`.
    #[test]
    fn discover_without_an_environment_is_a_clean_error_not_a_panic() {
        match QualityHandle::discover() {
            Ok(_) | Err(QualityError::NoEnvironment | QualityError::Open { .. }) => {}
            Err(other) => panic!("unexpected error shape from discover(): {other:?}"),
        }
    }

    /// `connect()` never touches the filesystem at construction time (only
    /// `Self::open()`, called from each read method, does) - must succeed
    /// even against a path nothing has ever written. Mirrors
    /// `idryx::tests::connect_never_touches_the_network_even_against_an_unreachable_url`.
    #[test]
    fn connect_never_touches_the_filesystem_at_construction_time() {
        let handle = QualityHandle::connect("/definitely/not/a/real/verdryx.db".to_string());
        assert_eq!(handle.db_path(), "/definitely/not/a/real/verdryx.db");
        assert!(matches!(handle.source(), QualityEnvSource::Explicit));
    }

    /// A read against a non-existent db must surface an honest
    /// [`QualityError::Open`], never a panic and never a fake-empty `Vec`.
    #[test]
    fn read_against_a_missing_db_is_an_honest_open_error_not_a_panic_or_fake_empty() {
        let handle = QualityHandle::connect("/definitely/not/a/real/verdryx.db".to_string());
        match handle.list_eval_runs() {
            Err(QualityError::Open { path, .. }) => {
                assert_eq!(path, "/definitely/not/a/real/verdryx.db");
            }
            other => panic!("expected QualityError::Open, got {other:?}"),
        }
    }

    // ==========================================================================
    // live e2e: a real verdryx.db (writer connection, the exact DDL
    // `crates/connectors/src/verdryx.rs`'s own tests use), read back through
    // this handle's exported methods end to end.
    // ==========================================================================

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

    fn seed_db() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "genaryx-ffi-quality-test-{}-{}.db",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let w = rusqlite::Connection::open(&path).expect("open writer");
        w.execute_batch(DDL).expect("ddl");
        w.execute(
            "INSERT INTO eval_runs VALUES ('run-1','claude-sonnet-5','2026-07-17T10:00:00+00:00','2026-07-17T10:05:00+00:00')",
            [],
        )
        .unwrap();
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
    fn end_to_end_over_a_real_verdryx_db() {
        let path = seed_db();
        let handle = QualityHandle::connect(path.to_string_lossy().into_owned());

        let runs = handle.list_eval_runs().expect("list_eval_runs");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].id, "run-2", "newest-started first");
        assert!(runs[0].finished_at.is_none());

        let latest = handle.latest_run().expect("latest_run").expect("some run");
        assert_eq!(latest.id, "run-2");

        let scores = handle
            .scores_for_run("run-1".to_string())
            .expect("scores_for_run");
        assert_eq!(scores.len(), 2);
        assert_eq!(scores[0].case_id, "c1");

        let baselines = handle.list_baselines().expect("list_baselines");
        assert_eq!(baselines.len(), 1);
        assert_eq!(baselines[0].label, "v1-golden");

        let summary = handle
            .run_summary("run-1".to_string())
            .expect("run_summary")
            .expect("run-1 exists");
        assert_eq!(summary.case_count, 2);
        assert_eq!(summary.mean_score, Some(0.75));

        // run-2 has no scores: honest None mean, not a fabricated 0.0.
        let empty_summary = handle
            .run_summary("run-2".to_string())
            .expect("run_summary")
            .expect("run-2 exists");
        assert_eq!(empty_summary.case_count, 0);
        assert_eq!(empty_summary.mean_score, None);

        // A run id that does not exist -> Ok(None), not an error.
        assert!(
            handle
                .run_summary("nope".to_string())
                .expect("no err")
                .is_none()
        );

        cleanup(&path);
    }
}
