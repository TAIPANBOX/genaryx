//! Wire DTOs and error taxonomy for [`super::DrillsHandle`], mirroring
//! `crates/ffi/src/crypto/dto.rs`'s shape (UniFFI `Record`/`Error` types
//! instead of `genaryx_connectors`' plain Rust structs) but over the Mockryx
//! contract (docs/PHASE4.md W2, `crates/connectors/src/mockryx.rs`'s own doc
//! comment).
//!
//! Records here keep a `Drill` prefix (`DrillReportRecord`, not a bare
//! `ReportRecord`): "Report"/"Result"/"Finding" alone would collide in spirit
//! with `crate::crypto::dto::{NcscReportRecord, EvidenceReportRecord}` in the
//! same flat, six-plane UniFFI namespace - the same judgment call that
//! module's own doc explains for `Ncsc`/`Evidence`. Singular ("Drill", not
//! "Drills"), matching this crate's existing per-item-record convention
//! (`IdentityRecord`, not `IdentitiesRecord`; `AlertRecord`, not
//! `AlertsRecord`).
//!
//! ## `BTreeMap<String, String>` headers cross FFI as `Vec<HeaderEntry>`
//!
//! [`ConnMockryxFinding`]'s `expect_header`/`got_headers` are
//! `Option<BTreeMap<String, String>>`. Mirrors the map-flattening convention
//! `crate::crypto::dto`'s own module doc establishes for W1
//! (`CountEntry`/`counts_from`, there over `BTreeMap<String, i64>`): a plain
//! `(key, value)` Record, `Vec`-collected in the map's own `BTreeMap`
//! (alphabetical) order. [`HeaderEntry`] is its own type rather than a reuse
//! of [`crate::crypto::dto::CountEntry`] because the value type differs
//! (`String`, not `i64`); the `Option` wrapper around the `Vec` is preserved
//! (rather than collapsing `None` to an empty `Vec`) so the Swift panel can
//! tell "this finding never carried a header comparison at all" apart from
//! "it carried one and it was empty" - never conflating absent with empty.

use genaryx_connectors::{
    MockryxError as ConnMockryxError, MockryxFinding as ConnMockryxFinding,
    MockryxMetrics as ConnMockryxMetrics, MockryxReport as ConnMockryxReport,
    MockryxResult as ConnMockryxResult,
};
use std::collections::BTreeMap;

// ============================================================================
// map -> Vec<HeaderEntry>
// ============================================================================

/// One `(key, value)` header pair - see the module doc.
#[derive(Debug, Clone, uniffi::Record)]
pub struct HeaderEntry {
    pub key: String,
    pub value: String,
}

fn headers_from(map: Option<&BTreeMap<String, String>>) -> Option<Vec<HeaderEntry>> {
    map.map(|m| {
        m.iter()
            .map(|(key, value)| HeaderEntry {
                key: key.clone(),
                value: value.clone(),
            })
            .collect()
    })
}

// ============================================================================
// DTOs
// ============================================================================

/// Per-scenario blast-radius metrics - exact field set of
/// `genaryx_connectors::MockryxMetrics`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DrillMetricsRecord {
    pub calls: i64,
    pub budget_burned_usd: f64,
}

impl From<&ConnMockryxMetrics> for DrillMetricsRecord {
    fn from(m: &ConnMockryxMetrics) -> Self {
        Self {
            calls: m.calls,
            budget_burned_usd: m.budget_burned_usd,
        }
    }
}

/// One step mismatch - exact field set of `genaryx_connectors::MockryxFinding`.
/// Rich by design (docs/PHASE4.md W2: "findings as clear action items -
/// scenario/step, expected vs got status+headers, detail").
#[derive(Debug, Clone, uniffi::Record)]
pub struct DrillFindingRecord {
    pub scenario: String,
    pub step: String,
    pub attempt: i64,
    pub expect_status: i64,
    pub expect_header: Option<Vec<HeaderEntry>>,
    pub got_status: i64,
    pub got_headers: Option<Vec<HeaderEntry>>,
    pub detail: String,
    /// Set only for a failed `expect.event` check (unused by the 5 bundled
    /// scenarios today - `genaryx_connectors::MockryxFinding`'s own doc).
    pub expect_event_source: Option<String>,
    pub expect_event_type: Option<String>,
}

impl From<&ConnMockryxFinding> for DrillFindingRecord {
    fn from(f: &ConnMockryxFinding) -> Self {
        Self {
            scenario: f.scenario.clone(),
            step: f.step.clone(),
            attempt: f.attempt,
            expect_status: f.expect_status,
            expect_header: headers_from(f.expect_header.as_ref()),
            got_status: f.got_status,
            got_headers: headers_from(f.got_headers.as_ref()),
            detail: f.detail.clone(),
            expect_event_source: f.expect_event_source.clone(),
            expect_event_type: f.expect_event_type.clone(),
        }
    }
}

/// One scenario's outcome - exact field set of
/// `genaryx_connectors::MockryxResult`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DrillResultRecord {
    pub scenario: String,
    /// `passed` | `failed` | `skipped_not_configured`. docs/PHASE4.md W2:
    /// read "gap" from `findings`/`failed`, never from this alone - see
    /// [`DrillReportRecord::has_gaps`]'s own doc.
    pub status: String,
    pub findings: Vec<DrillFindingRecord>,
    /// Mismatches discarded because the scenario's guardrail was never
    /// observed active (only on `skipped_not_configured`); shown separately
    /// from `findings` (docs/PHASE4.md W2 guard), never merged into it.
    pub skipped_findings: Vec<DrillFindingRecord>,
    pub metrics: DrillMetricsRecord,
}

impl From<&ConnMockryxResult> for DrillResultRecord {
    fn from(r: &ConnMockryxResult) -> Self {
        Self {
            scenario: r.scenario.clone(),
            status: r.status.clone(),
            findings: r.findings.iter().map(DrillFindingRecord::from).collect(),
            skipped_findings: r
                .skipped_findings
                .iter()
                .map(DrillFindingRecord::from)
                .collect(),
            metrics: DrillMetricsRecord::from(&r.metrics),
        }
    }
}

/// The whole drill report - exact field set of
/// `genaryx_connectors::MockryxReport`, plus [`Self::has_gaps`] precomputed
/// (UniFFI Records cannot carry methods, so the connector's own
/// `MockryxReport::has_gaps()` is called once at conversion time and stored,
/// rather than re-derived on the Swift side from a copy of the same logic).
#[derive(Debug, Clone, uniffi::Record)]
pub struct DrillReportRecord {
    pub run_id: String,
    pub gateway: String,
    pub generated_at: String,
    pub results: Vec<DrillResultRecord>,
    /// docs/PHASE4.md W2: the overall verdict - any scenario that outright
    /// `failed`, or any scenario carrying findings (which, after
    /// `fail_on_skip`, can include promoted skips). A `skipped_not_configured`
    /// scenario with empty `findings` is NOT a gap on its own - see
    /// `genaryx_connectors::MockryxReport::has_gaps`'s own doc.
    pub has_gaps: bool,
}

impl From<&ConnMockryxReport> for DrillReportRecord {
    fn from(r: &ConnMockryxReport) -> Self {
        Self {
            run_id: r.run_id.clone(),
            gateway: r.gateway.clone(),
            generated_at: r.generated_at.clone(),
            results: r.results.iter().map(DrillResultRecord::from).collect(),
            has_gaps: r.has_gaps(),
        }
    }
}

// ============================================================================
// error taxonomy
// ============================================================================

/// Every failure mode a [`super::DrillsHandle`] call can surface, fail-closed
/// throughout (06 §0.5). Collapsed from `genaryx_connectors::MockryxError`'s
/// four variants, plus [`Self::NoEnvironment`] - an ffi-layer-only addition
/// with no connector-level equivalent, exactly like `CryptoError::NoEnvironment`.
///
/// CRITICAL (docs/PHASE4.md's own review-discipline guard, repeated here so
/// it stays next to the type that could violate it): a fire-drill "gap"
/// (mockryx exit `1`, real findings) is NEVER one of these error variants -
/// it is a completely normal [`DrillReportRecord`] with `has_gaps: true`.
/// Only exit `2`/spawn failure/unparseable output are errors here; a gap must
/// never be swallowed and shown as "guardrails held".
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum DrillsError {
    /// [`super::env::discover_bin`] found no `mockryx` binary anywhere it
    /// knows to look - a normal, renderable "no drills plane" outcome
    /// (docs/PHASE4.md W2: "honest empty state when no mockryx binary"), not
    /// a bug.
    #[error("no drills plane found (set MOCKRYX_BIN, or build ~/Development/mockryx/bin/mockryx)")]
    NoEnvironment,
    /// The mockryx binary could not be spawned - missing, not executable, or
    /// an operator-supplied path via [`super::DrillsHandle::connect`] that
    /// does not exist.
    #[error("could not run mockryx at {bin}: {reason}")]
    Spawn { bin: String, reason: String },
    /// mockryx exited `2` (or another non-`{0,1}` code): a real usage/config
    /// error - nothing was actually rehearsed. Carries mockryx's own stderr.
    #[error("mockryx exited {code}: {stderr}")]
    Cli { code: i32, stderr: String },
    /// A stdout/saved report failed to deserialize - this crate's DTOs have
    /// drifted from mockryx's own `report.Report` shape.
    #[error("could not parse mockryx output: {reason}")]
    Json { reason: String },
    /// [`super::DrillsHandle::load_report`]'s file could not be read.
    #[error("could not read {path}: {reason}")]
    Read { path: String, reason: String },
}

impl From<ConnMockryxError> for DrillsError {
    fn from(e: ConnMockryxError) -> Self {
        match e {
            ConnMockryxError::Spawn { bin, source } => DrillsError::Spawn {
                bin,
                reason: source.to_string(),
            },
            ConnMockryxError::Cli { code, stderr } => DrillsError::Cli { code, stderr },
            ConnMockryxError::Json(source) => DrillsError::Json {
                reason: source.to_string(),
            },
            ConnMockryxError::Read { path, source } => DrillsError::Read {
                path,
                reason: source.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(step: &str, expect_header: Option<BTreeMap<String, String>>) -> ConnMockryxFinding {
        ConnMockryxFinding {
            scenario: "wardryx-denied-tool".to_string(),
            step: step.to_string(),
            attempt: 1,
            expect_status: 403,
            expect_header,
            got_status: 200,
            got_headers: None,
            detail: "guardrail did not deny".to_string(),
            expect_event_source: None,
            expect_event_type: None,
        }
    }

    #[test]
    fn headers_from_none_stays_none_not_an_empty_vec() {
        assert!(headers_from(None).is_none());
    }

    #[test]
    fn headers_from_preserves_every_entry() {
        let mut map = BTreeMap::new();
        map.insert("x-fuse-wardryx".to_string(), "deny".to_string());
        map.insert("content-type".to_string(), "application/json".to_string());
        let entries = headers_from(Some(&map)).expect("Some map yields Some entries");
        assert_eq!(entries.len(), 2);
        assert!(
            entries
                .iter()
                .any(|e| e.key == "x-fuse-wardryx" && e.value == "deny")
        );
    }

    #[test]
    fn drill_finding_record_preserves_the_none_vs_empty_distinction() {
        let with_header = finding("a", Some(BTreeMap::new()));
        let record = DrillFindingRecord::from(&with_header);
        assert!(
            matches!(record.expect_header, Some(ref entries) if entries.is_empty()),
            "an explicitly-empty map must stay Some(empty), not collapse to None: {:?}",
            record.expect_header
        );

        let without_header = finding("b", None);
        let record = DrillFindingRecord::from(&without_header);
        assert!(record.expect_header.is_none());
    }

    #[test]
    fn drill_report_record_precomputes_has_gaps_from_a_failed_result() {
        let conn = ConnMockryxReport {
            run_id: "run-1".to_string(),
            gateway: "http://127.0.0.1:4100".to_string(),
            generated_at: "2026-07-17T12:00:00Z".to_string(),
            results: vec![ConnMockryxResult {
                scenario: "wardryx-denied-tool".to_string(),
                status: "failed".to_string(),
                findings: vec![finding("call-denied-tool", None)],
                skipped_findings: vec![],
                metrics: ConnMockryxMetrics {
                    calls: 3,
                    budget_burned_usd: 0.012,
                },
            }],
        };
        let record = DrillReportRecord::from(&conn);
        assert!(record.has_gaps, "a failed result must set has_gaps");
        assert_eq!(record.results.len(), 1);
        assert_eq!(record.results[0].findings.len(), 1);
    }

    #[test]
    fn drill_report_record_a_bare_skip_is_not_a_gap() {
        let conn = ConnMockryxReport {
            run_id: "run-2".to_string(),
            gateway: "http://127.0.0.1:4100".to_string(),
            generated_at: "2026-07-17T12:00:00Z".to_string(),
            results: vec![
                ConnMockryxResult {
                    scenario: "runaway-budget".to_string(),
                    status: "passed".to_string(),
                    findings: vec![],
                    skipped_findings: vec![],
                    metrics: ConnMockryxMetrics {
                        calls: 8,
                        budget_burned_usd: 0.0,
                    },
                },
                ConnMockryxResult {
                    scenario: "dlp-secret-leak".to_string(),
                    status: "skipped_not_configured".to_string(),
                    findings: vec![],
                    skipped_findings: vec![finding("leak", None)],
                    metrics: ConnMockryxMetrics {
                        calls: 1,
                        budget_burned_usd: 0.001,
                    },
                },
            ],
        };
        let record = DrillReportRecord::from(&conn);
        assert!(!record.has_gaps, "passed + a bare skip must not be a gap");
        assert_eq!(record.results[1].skipped_findings.len(), 1);
        assert!(record.results[1].findings.is_empty());
    }

    #[test]
    fn mockryx_cli_error_maps_to_drills_error_cli_with_code_and_stderr() {
        let err = ConnMockryxError::Cli {
            code: 2,
            stderr: "no gateway configured".to_string(),
        };
        match DrillsError::from(err) {
            DrillsError::Cli { code, stderr } => {
                assert_eq!(code, 2);
                assert_eq!(stderr, "no gateway configured");
            }
            other => panic!("expected DrillsError::Cli, got {other:?}"),
        }
    }
}
