//! The Quality view's three reads over a real `verdryx.db` built with
//! verdryx's own DDL, and every state the panel can be in before it can read.

use genaryx_api::quality::commands::{
    QualityError, QualityStatusDto, quality_list_baselines, quality_list_run_summaries,
    quality_run_scores, quality_status,
};
use genaryx_api::quality::env::EnvSource;
use genaryx_api::quality::state::{QualityClient, QualityInner, QualityState};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// verdryx's `store.py` DDL, as `genaryx_connectors::verdryx`'s own tests
/// carry it.
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

fn seeded_db() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "genaryx-quality-itest-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let w = rusqlite::Connection::open(&path).expect("open writer");
    w.execute_batch(DDL).expect("ddl");
    w.execute_batch(
        "INSERT INTO eval_runs VALUES ('run-1','model-a','2026-07-17T10:00:00+00:00','2026-07-17T10:05:00+00:00');
         INSERT INTO eval_runs VALUES ('run-2','model-b','2026-07-17T11:00:00+00:00',NULL);
         INSERT INTO scores (run_id,case_id,value,tokens,cost_usd) VALUES ('run-1','c1',1.0,120,0.004);
         INSERT INTO scores (run_id,case_id,value,tokens,cost_usd) VALUES ('run-1','c2',0.5,80,0.002);
         INSERT INTO baselines VALUES ('bl-1','run-1',0.75,'2026-07-17T10:06:00+00:00','v1-golden');",
    )
    .expect("seed");
    drop(w);
    path
}

async fn ready_at(db_path: PathBuf) -> QualityState {
    let state = QualityState::pending();
    *state.inner.lock().await = QualityInner::Ready(QualityClient {
        source: EnvSource::WellKnown,
        db_path,
    });
    state
}

async fn state_of(inner: QualityInner) -> QualityState {
    let state = QualityState::pending();
    *state.inner.lock().await = inner;
    state
}

#[tokio::test]
async fn run_summaries_are_newest_first_with_their_own_totals() {
    let db = seeded_db();
    let state = ready_at(db.clone()).await;
    let runs = quality_list_run_summaries(&state).await.expect("read");
    let ids: Vec<&str> = runs.iter().map(|s| s.run.id.as_str()).collect();
    assert_eq!(ids, vec!["run-2", "run-1"], "newest started first");

    let in_flight = &runs[0];
    assert_eq!(in_flight.case_count, 0);
    assert_eq!(
        in_flight.mean_score, None,
        "a run with no scores has no mean, never zero"
    );
    assert_eq!(in_flight.run.finished_at, None);

    let done = &runs[1];
    assert_eq!(done.case_count, 2);
    assert_eq!(done.mean_score, Some(0.75));
    assert_eq!(done.total_tokens, 200);
    assert!((done.total_cost_usd - 0.006).abs() < 1e-9);
    assert!(
        !done.unanswered_supported,
        "this store has no unanswered table, and the summary says so"
    );
    let _ = std::fs::remove_file(&db);
}

#[tokio::test]
async fn a_runs_scores_come_in_evaluation_order_and_an_unknown_run_has_none() {
    let db = seeded_db();
    let state = ready_at(db.clone()).await;
    let scores = quality_run_scores("run-1".to_string(), &state)
        .await
        .expect("read");
    let cases: Vec<(&str, f64)> = scores
        .iter()
        .map(|s| (s.case_id.as_str(), s.value))
        .collect();
    assert_eq!(cases, vec![("c1", 1.0), ("c2", 0.5)]);
    assert!(
        quality_run_scores("no-such-run".to_string(), &state)
            .await
            .expect("read")
            .is_empty()
    );
    let _ = std::fs::remove_file(&db);
}

#[tokio::test]
async fn baselines_are_listed_with_their_runs_unanswered_accounting() {
    let db = seeded_db();
    let state = ready_at(db.clone()).await;
    let baselines = quality_list_baselines(&state).await.expect("read");
    assert_eq!(baselines.len(), 1);
    let b = &baselines[0];
    assert_eq!(b.baseline.id, "bl-1");
    assert_eq!(b.baseline.eval_run_id, "run-1");
    assert_eq!(b.baseline.label, "v1-golden");
    assert_eq!(b.baseline.mean_score, 0.75);
    assert_eq!(b.unanswered_count, 0);
    let _ = std::fs::remove_file(&db);
}

/// A store that vanished after the panel was ready is an error naming the
/// failure, never an empty history.
#[tokio::test]
async fn a_store_that_is_gone_is_an_error_not_an_empty_history() {
    let db = seeded_db();
    let state = ready_at(db.clone()).await;
    std::fs::remove_file(&db).expect("remove the store");
    for r in [
        quality_list_run_summaries(&state).await.map(|v| v.len()),
        quality_run_scores("run-1".to_string(), &state)
            .await
            .map(|v| v.len()),
        quality_list_baselines(&state).await.map(|v| v.len()),
    ] {
        match r {
            Err(QualityError::Verdryx { message }) => assert!(!message.is_empty()),
            other => panic!("expected a Verdryx error, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn every_state_before_ready_is_named_by_status_and_by_every_read() {
    assert!(matches!(
        quality_status(&QualityState::pending()).await,
        Ok(QualityStatusDto::Bootstrapping)
    ));
    let none = state_of(QualityInner::NoEnvironment).await;
    assert!(matches!(
        quality_status(&none).await,
        Ok(QualityStatusDto::NoEnvironment)
    ));
    let unreachable = state_of(QualityInner::Unreachable {
        source: EnvSource::WellKnown,
        db_path: PathBuf::from("/var/lib/verdryx.db"),
        reason: "file is not a database".to_string(),
    })
    .await;
    match quality_status(&unreachable).await {
        Ok(QualityStatusDto::Unreachable {
            db_path, reason, ..
        }) => {
            assert_eq!(db_path, "/var/lib/verdryx.db");
            assert_eq!(reason, "file is not a database");
        }
        other => panic!("expected Unreachable, got {other:?}"),
    }
    let db = PathBuf::from("/var/lib/verdryx.db");
    match quality_status(&ready_at(db).await).await {
        Ok(QualityStatusDto::Ready { db_path, source }) => {
            assert_eq!(db_path, "/var/lib/verdryx.db");
            assert!(matches!(source, EnvSource::WellKnown));
        }
        other => panic!("expected Ready, got {other:?}"),
    }

    let pending = QualityState::pending();
    assert!(matches!(
        quality_list_run_summaries(&pending).await,
        Err(QualityError::Bootstrapping)
    ));
    assert!(matches!(
        quality_run_scores("r".to_string(), &none).await,
        Err(QualityError::NoEnvironment)
    ));
    match quality_list_baselines(&unreachable).await {
        Err(QualityError::Unreachable { reason }) => assert_eq!(reason, "file is not a database"),
        other => panic!("expected Unreachable, got {:?}", other.err()),
    }
}
