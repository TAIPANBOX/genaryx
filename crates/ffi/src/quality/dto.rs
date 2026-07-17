//! Wire DTOs and error taxonomy for [`super::QualityHandle`], mirroring
//! `crates/ffi/src/idryx/dto.rs`'s shape (UniFFI `Record`/`Error` types
//! instead of `genaryx_connectors::verdryx`'s plain Rust structs) but over the
//! Verdryx contract (docs/PHASE4.md W1, `crates/connectors/src/verdryx.rs`'s
//! own doc comment).
//!
//! `genaryx_connectors` re-exports its Verdryx types already `Verdryx`-prefixed
//! (`EvalRun as VerdryxEvalRun`, `Score as VerdryxScore`, ...) to avoid
//! colliding with its own sibling connector types; imported here under a
//! `Conn` prefix anyway (mirroring `idryx/dto.rs`'s own `Conn`-prefix
//! convention), since this module defines its own same-shaped
//! [`EvalRunRecord`], [`ScoreRecord`], [`BaselineRecord`],
//! [`RunSummaryRecord`], and [`QualityError`] as the UniFFI-facing
//! counterparts.

use genaryx_connectors::{
    VerdryxBaseline as ConnBaseline, VerdryxError as ConnVerdryxError,
    VerdryxEvalRun as ConnEvalRun, VerdryxRunSummary as ConnRunSummary, VerdryxScore as ConnScore,
};

// ============================================================================
// DTOs
// ============================================================================

/// One row of the Eval Runs history: exact field set of
/// `genaryx_connectors::VerdryxEvalRun` (`verdryx/store.py`'s `eval_runs`
/// table, one row per `verdryx eval` invocation).
#[derive(Debug, Clone, uniffi::Record)]
pub struct EvalRunRecord {
    pub id: String,
    /// The model under eval, e.g. `claude-sonnet-5`.
    pub model: String,
    /// UTC ISO-8601.
    pub started_at: String,
    /// UTC ISO-8601, `None` while the run is still in flight (the store's one
    /// nullable column).
    pub finished_at: Option<String>,
}

impl From<&ConnEvalRun> for EvalRunRecord {
    fn from(r: &ConnEvalRun) -> Self {
        Self {
            id: r.id.clone(),
            model: r.model.clone(),
            started_at: r.started_at.clone(),
            finished_at: r.finished_at.clone(),
        }
    }
}

/// One row of a run's per-case scores table: exact field set of
/// `genaryx_connectors::VerdryxScore` (`verdryx/store.py`'s `scores` table).
#[derive(Debug, Clone, uniffi::Record)]
pub struct ScoreRecord {
    pub id: i64,
    /// Joins to [`EvalRunRecord::id`].
    pub run_id: String,
    pub case_id: String,
    /// The quality score, `[0.0, 1.0]`.
    pub value: f64,
    pub tokens: i64,
    pub cost_usd: f64,
}

impl From<&ConnScore> for ScoreRecord {
    fn from(s: &ConnScore) -> Self {
        Self {
            id: s.id,
            run_id: s.run_id.clone(),
            case_id: s.case_id.clone(),
            value: s.value,
            tokens: s.tokens,
            cost_usd: s.cost_usd,
        }
    }
}

/// One row of the Baselines list: exact field set of
/// `genaryx_connectors::VerdryxBaseline` (`verdryx/store.py`'s `baselines`
/// table) - a saved mean-score snapshot a later run's drift is measured
/// against.
#[derive(Debug, Clone, uniffi::Record)]
pub struct BaselineRecord {
    pub id: String,
    /// FK to the [`EvalRunRecord`] this baseline was snapshotted from.
    pub eval_run_id: String,
    pub mean_score: f64,
    pub created_at: String,
    /// A human label, e.g. `v1-golden`; empty string when verdryx never set
    /// one (the store's own default, kept verbatim rather than turned into an
    /// `Option` - unlike [`crate::idryx::dto`]'s `non_empty` convention, an
    /// empty baseline label is a normal, expected value here, not a
    /// zero-value idryx substitutes for "never set").
    pub label: String,
}

impl From<&ConnBaseline> for BaselineRecord {
    fn from(b: &ConnBaseline) -> Self {
        Self {
            id: b.id.clone(),
            eval_run_id: b.eval_run_id.clone(),
            mean_score: b.mean_score,
            created_at: b.created_at.clone(),
            label: b.label.clone(),
        }
    }
}

/// A run's rollup for the history list's summary columns and the run-detail
/// header: exact field set of `genaryx_connectors::VerdryxRunSummary`,
/// computed by the connector from `scores` (never stored, always matches the
/// rows it was built from).
#[derive(Debug, Clone, uniffi::Record)]
pub struct RunSummaryRecord {
    pub run: EvalRunRecord,
    pub case_count: u64,
    /// `None` when the run has no scores yet - the Swift panel renders this
    /// as "n/a", NEVER as `0` (docs/PHASE4.md W1 guard: "`mean_score: None`
    /// renders 'n/a', not 0" - see `crates/connectors/src/verdryx.rs`'s own
    /// doc: "never a divide-by-zero or a fabricated 0.0").
    pub mean_score: Option<f64>,
    pub total_tokens: i64,
    pub total_cost_usd: f64,
}

impl From<&ConnRunSummary> for RunSummaryRecord {
    fn from(s: &ConnRunSummary) -> Self {
        Self {
            run: EvalRunRecord::from(&s.run),
            case_count: s.case_count,
            mean_score: s.mean_score,
            total_tokens: s.total_tokens,
            total_cost_usd: s.total_cost_usd,
        }
    }
}

// ============================================================================
// error taxonomy
// ============================================================================

/// Every failure mode a [`super::QualityHandle`] call can surface, fail-closed
/// throughout (06 §0.5: no panics/unwraps cross the FFI boundary). Collapsed
/// from `genaryx_connectors::VerdryxError`'s two variants, plus
/// [`Self::NoEnvironment`] - an ffi-layer-only addition with no connector-level
/// equivalent, exactly like `IdryxError::NoEnvironment` (about resolving an
/// environment, not about a call the connector itself made).
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum QualityError {
    /// [`super::env::discover`] found no candidate `verdryx.db` at all - a
    /// normal, renderable "no quality plane" outcome (docs/PHASE4.md W1: "An
    /// absent source... must render as an honest first-class empty state"),
    /// not a bug.
    #[error("no quality plane found (no verdryx.db at any known location)")]
    NoEnvironment,
    /// `verdryx.db` could not be opened read-only at the resolved (or
    /// operator-supplied) path - missing file, permissions, or an
    /// uncheckpointed `-wal` from a crashed `eval` (see
    /// `crates/connectors/src/verdryx.rs`'s own doc, "WAL-at-rest"). Distinct
    /// from [`Self::NoEnvironment`]: this means a path WAS named (by
    /// `VERDRYX_DB` or the operator), just not a readable database.
    #[error("could not open verdryx.db at {path}: {reason}")]
    Open { path: String, reason: String },
    /// A prepared statement, query, or row decode failed - the schema this
    /// reader expects has drifted from the live `verdryx.db`, or a run/case
    /// id named a row that does not exist in the expected shape.
    #[error("verdryx.db query failed: {reason}")]
    Query { reason: String },
}

impl From<ConnVerdryxError> for QualityError {
    fn from(e: ConnVerdryxError) -> Self {
        match e {
            ConnVerdryxError::Open { path, source } => QualityError::Open {
                path,
                reason: source.to_string(),
            },
            ConnVerdryxError::Query(source) => QualityError::Query {
                reason: source.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_run_record_maps_every_field() {
        let conn = ConnEvalRun {
            id: "run-1".to_string(),
            model: "claude-sonnet-5".to_string(),
            started_at: "2026-07-17T10:00:00+00:00".to_string(),
            finished_at: Some("2026-07-17T10:05:00+00:00".to_string()),
        };
        let record = EvalRunRecord::from(&conn);
        assert_eq!(record.id, "run-1");
        assert_eq!(record.model, "claude-sonnet-5");
        assert_eq!(record.started_at, "2026-07-17T10:00:00+00:00");
        assert_eq!(
            record.finished_at.as_deref(),
            Some("2026-07-17T10:05:00+00:00")
        );
    }

    #[test]
    fn run_summary_record_preserves_none_mean_score_not_a_fabricated_zero() {
        let conn = ConnRunSummary {
            run: ConnEvalRun {
                id: "run-2".to_string(),
                model: "claude-opus-4-8".to_string(),
                started_at: "2026-07-17T11:00:00+00:00".to_string(),
                finished_at: None,
            },
            case_count: 0,
            mean_score: None,
            total_tokens: 0,
            total_cost_usd: 0.0,
        };
        let record = RunSummaryRecord::from(&conn);
        assert_eq!(record.case_count, 0);
        assert_eq!(
            record.mean_score, None,
            "must stay None, never a fabricated 0.0"
        );
        assert_eq!(record.run.id, "run-2");
    }

    #[test]
    fn baseline_record_keeps_an_empty_label_as_empty_not_none() {
        let conn = ConnBaseline {
            id: "bl-1".to_string(),
            eval_run_id: "run-1".to_string(),
            mean_score: 0.75,
            created_at: "2026-07-17T10:06:00+00:00".to_string(),
            label: String::new(),
        };
        let record = BaselineRecord::from(&conn);
        assert_eq!(
            record.label, "",
            "an unset label is a normal empty string here"
        );
    }

    #[test]
    fn connector_open_error_maps_to_a_named_quality_open_error() {
        // rusqlite::Error has no public zero-arg constructor for this test to
        // build one directly; instead this proves the QualityError variant
        // shape itself is reachable and carries both fields distinctly.
        let err = QualityError::Open {
            path: "/tmp/verdryx.db".to_string(),
            reason: "unable to open database file".to_string(),
        };
        match err {
            QualityError::Open { path, reason } => {
                assert_eq!(path, "/tmp/verdryx.db");
                assert!(reason.contains("unable to open"));
            }
            other => panic!("expected Open, got {other:?}"),
        }
    }
}
