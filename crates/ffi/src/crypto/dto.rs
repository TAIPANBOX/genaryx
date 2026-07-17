//! Wire DTOs and error taxonomy for [`super::CryptoHandle`], mirroring
//! `crates/ffi/src/idryx/dto.rs`'s shape (UniFFI `Record`/`Error` types
//! instead of `genaryx_connectors::qryx`'s plain Rust structs) but over the
//! Qryx contract (docs/PHASE4.md W1, `crates/connectors/src/qryx.rs`'s own
//! doc comment).
//!
//! `genaryx_connectors` re-exports its Qryx types already `Qryx`-prefixed
//! where a name would otherwise collide (`Signature as QryxSignature`);
//! imported here under a `Conn` prefix throughout anyway (mirroring
//! `idryx/dto.rs`'s own convention), since this module defines its own
//! same-shaped `*Record` counterparts.
//!
//! ## Representing untyped/map-shaped wire fields across FFI
//!
//! Two wire shapes UniFFI Records cannot carry directly:
//!
//! - **`BTreeMap<String, i64>`** (`coverage_by_source`, `by_severity`): no
//!   established convention exists yet elsewhere in this crate (grepped: no
//!   sibling `dto.rs` crosses a map over FFI at all - every existing `env.rs`
//!   module keeps its own `BTreeMap`-shaped descriptor parsing internal,
//!   never exported). This module establishes one: [`CountEntry`], a plain
//!   `(key, count)` Record, `Vec`-collected in the map's own `BTreeMap`
//!   (alphabetical) order - typed and self-describing on the Swift side
//!   (`entry.key`/`entry.count`) rather than a second JSON-string field the
//!   panel would have to decode just to read two small maps.
//! - **`Vec<serde_json::Value>`** (`EvidenceReport::assets`) and the whole
//!   CBOM report (`QryxClient::scan_cbom`'s return type IS `serde_json::Value`,
//!   untyped by the connector's own deliberate choice - see its doc: "a
//!   large, display-only shape the panel renders as a table, not something
//!   this connector reasons over"): serialized to a plain JSON `String` at
//!   the FFI boundary ([`EvidenceReportRecord::assets_json`],
//!   [`super::CryptoHandle::scan_cbom`]'s return type). The Swift panel
//!   decodes it with `JSONSerialization`, the same best-effort,
//!   never-force-unwrapped idiom `UiEvent.wardryxFields`/`RawJsonView`
//!   already use for the bus's own raw NDJSON lines - not a second typed
//!   Record tree for a schema (CycloneDX 1.6) this crate does not otherwise
//!   need to validate or reason over.

use genaryx_connectors::{
    EvidenceReport as ConnEvidenceReport, EvidenceSummary as ConnEvidenceSummary,
    NcscDiscovery as ConnNcscDiscovery, NcscFinding as ConnNcscFinding,
    NcscFullMigration as ConnNcscFullMigration, NcscPriority as ConnNcscPriority,
    NcscReport as ConnNcscReport, QryxError as ConnQryxError, QryxSignature as ConnSignature,
    VerifyOutcome as ConnVerifyOutcome,
};
use std::collections::BTreeMap;

// ============================================================================
// map -> Vec<CountEntry>
// ============================================================================

/// One `(key, count)` pair - see the module doc's "Representing untyped/
/// map-shaped wire fields across FFI".
#[derive(Debug, Clone, uniffi::Record)]
pub struct CountEntry {
    pub key: String,
    pub count: i64,
}

fn counts_from(map: &BTreeMap<String, i64>) -> Vec<CountEntry> {
    map.iter()
        .map(|(key, count)| CountEntry {
            key: key.clone(),
            count: *count,
        })
        .collect()
}

// ============================================================================
// DTOs: --format ncsc
// ============================================================================

/// One quantum-vulnerable asset finding: exact field set of
/// `genaryx_connectors::NcscFinding`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NcscFindingRecord {
    pub algorithm: String,
    pub asset_type: String,
    /// Lowercase severity string (`Theme.severityColor` on the Swift side
    /// already accepts this shape, matching every other severity field this
    /// shell renders - `AlertRecord.severity`, `UiEvent.severity`).
    pub severity: String,
    pub occurrences: i64,
    pub locations: Vec<String>,
    pub externally_facing: bool,
    pub long_lived_data: bool,
    pub planned: bool,
}

impl From<&ConnNcscFinding> for NcscFindingRecord {
    fn from(f: &ConnNcscFinding) -> Self {
        Self {
            algorithm: f.algorithm.clone(),
            asset_type: f.asset_type.clone(),
            severity: f.severity.clone(),
            occurrences: f.occurrences,
            locations: f.locations.clone(),
            externally_facing: f.externally_facing,
            long_lived_data: f.long_lived_data,
            planned: f.planned,
        }
    }
}

/// The 2028 "complete discovery" milestone: exact field set of
/// `genaryx_connectors::NcscDiscovery`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NcscDiscoveryRecord {
    /// `on-track` | `at-risk` | `not-started`.
    pub verdict: String,
    pub coverage_by_source: Vec<CountEntry>,
    pub total_inventoried: i64,
    pub quantum_vulnerable_count: i64,
    pub migration_plan_exists: bool,
    pub migration_plan_note: String,
    pub quantum_vulnerable_findings: Vec<NcscFindingRecord>,
}

impl From<&ConnNcscDiscovery> for NcscDiscoveryRecord {
    fn from(d: &ConnNcscDiscovery) -> Self {
        Self {
            verdict: d.verdict.clone(),
            coverage_by_source: counts_from(&d.coverage_by_source),
            total_inventoried: d.total_inventoried,
            quantum_vulnerable_count: d.quantum_vulnerable_count,
            migration_plan_exists: d.migration_plan_exists,
            migration_plan_note: d.migration_plan_note.clone(),
            quantum_vulnerable_findings: d
                .quantum_vulnerable_findings
                .iter()
                .map(NcscFindingRecord::from)
                .collect(),
        }
    }
}

/// The 2031 "highest-priority systems" milestone: exact field set of
/// `genaryx_connectors::NcscPriority`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NcscPriorityRecord {
    pub verdict: String,
    pub criteria: String,
    pub count: i64,
    /// ALWAYS `0` - qryx tracks no cross-run remediation state (see
    /// `genaryx_connectors::NcscPriority::migrated_count`'s own doc: "progress
    /// is the `--baseline` drift / evidence-trail's job, not this report's").
    /// Carried through verbatim, never inflated or hidden; the Swift panel
    /// MUST label this "not tracked", never as real migrated progress
    /// (docs/PHASE4.md W1 guard).
    pub migrated_count: i64,
    pub remaining_count: i64,
    pub note: String,
    pub findings: Vec<NcscFindingRecord>,
}

impl From<&ConnNcscPriority> for NcscPriorityRecord {
    fn from(p: &ConnNcscPriority) -> Self {
        Self {
            verdict: p.verdict.clone(),
            criteria: p.criteria.clone(),
            count: p.count,
            migrated_count: p.migrated_count,
            remaining_count: p.remaining_count,
            note: p.note.clone(),
            findings: p.findings.iter().map(NcscFindingRecord::from).collect(),
        }
    }
}

/// The 2035 "all systems" milestone: exact field set of
/// `genaryx_connectors::NcscFullMigration`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NcscFullMigrationRecord {
    pub verdict: String,
    pub count: i64,
    pub findings: Vec<NcscFindingRecord>,
}

impl From<&ConnNcscFullMigration> for NcscFullMigrationRecord {
    fn from(m: &ConnNcscFullMigration) -> Self {
        Self {
            verdict: m.verdict.clone(),
            count: m.count,
            findings: m.findings.iter().map(NcscFindingRecord::from).collect(),
        }
    }
}

/// `--format ncsc`: the whole PQC migration-timeline report the Crypto
/// panel's hero renders - exact field set of `genaryx_connectors::NcscReport`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NcscReportRecord {
    pub standard: String,
    pub generated_at: String,
    pub root: String,
    pub discovery_2028: NcscDiscoveryRecord,
    pub highest_priority_2031: NcscPriorityRecord,
    pub full_migration_2035: NcscFullMigrationRecord,
}

impl From<&ConnNcscReport> for NcscReportRecord {
    fn from(r: &ConnNcscReport) -> Self {
        Self {
            standard: r.standard.clone(),
            generated_at: r.generated_at.clone(),
            root: r.root.clone(),
            discovery_2028: NcscDiscoveryRecord::from(&r.discovery_2028),
            highest_priority_2031: NcscPriorityRecord::from(&r.highest_priority_2031),
            full_migration_2035: NcscFullMigrationRecord::from(&r.full_migration_2035),
        }
    }
}

// ============================================================================
// DTOs: --format evidence
// ============================================================================

/// A detached evidence signature: exact field set of
/// `genaryx_connectors::QryxSignature`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct EvidenceSignatureRecord {
    /// `ed25519` | `ecdsa-p256` | `ml-dsa-44` | `ml-dsa-65` | `ml-dsa-87`.
    pub alg: String,
    pub value: String,
    pub public_key: String,
}

impl From<&ConnSignature> for EvidenceSignatureRecord {
    fn from(s: &ConnSignature) -> Self {
        Self {
            alg: s.alg.clone(),
            value: s.value.clone(),
            public_key: s.public_key.clone(),
        }
    }
}

/// The compliance rollup in an [`EvidenceReportRecord`]: exact field set of
/// `genaryx_connectors::EvidenceSummary`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct EvidenceSummaryRecord {
    pub compliant: i64,
    pub non_compliant: i64,
    pub issues: i64,
    pub total: i64,
    pub score_pct: i64,
    pub by_severity: Vec<CountEntry>,
}

impl From<&ConnEvidenceSummary> for EvidenceSummaryRecord {
    fn from(s: &ConnEvidenceSummary) -> Self {
        Self {
            compliant: s.compliant,
            non_compliant: s.non_compliant,
            issues: s.issues,
            total: s.total,
            score_pct: s.score_pct,
            by_severity: counts_from(&s.by_severity),
        }
    }
}

/// `--format evidence`: a CNSA 2.0 compliance attestation - exact field set of
/// `genaryx_connectors::EvidenceReport`, `assets` re-serialized to a JSON
/// string (see the module doc).
#[derive(Debug, Clone, uniffi::Record)]
pub struct EvidenceReportRecord {
    pub tool: String,
    pub version: String,
    pub standard: String,
    pub generated_at: String,
    pub root: String,
    pub summary: EvidenceSummaryRecord,
    /// `assets` as a JSON array string - see the module doc's "Representing
    /// untyped/map-shaped wire fields across FFI".
    pub assets_json: String,
    /// `"sha256:<hex>"`, the security-critical field, kept as a plain typed
    /// `String` (matches the connector's own choice to type this one field
    /// out of an otherwise-untyped report).
    pub digest: String,
    /// Present only when built with `--sign-key`; `None` for W1's unsigned
    /// bundles (docs/PHASE4.md W1: "unsigned fine for W1").
    pub signature: Option<EvidenceSignatureRecord>,
}

impl EvidenceReportRecord {
    /// Fallible (unlike every other `From` in this module) because `assets`
    /// must be re-serialized to a string: see the module doc. In practice
    /// this cannot fail for a report that itself just came from
    /// `serde_json::from_slice` (a `Value` tree already round-tripped through
    /// valid JSON always re-serializes), but this returns a real
    /// [`CryptoError`] rather than `.expect()`-ing that fact away - 06 §0.5,
    /// no panics cross the FFI boundary, ever, even for a "cannot actually
    /// happen" case.
    pub(super) fn from_conn(r: &ConnEvidenceReport) -> Result<Self, CryptoError> {
        let assets_json = serde_json::to_string(&r.assets).map_err(|e| CryptoError::Json {
            reason: e.to_string(),
        })?;
        Ok(Self {
            tool: r.tool.clone(),
            version: r.version.clone(),
            standard: r.standard.clone(),
            generated_at: r.generated_at.clone(),
            root: r.root.clone(),
            summary: EvidenceSummaryRecord::from(&r.summary),
            assets_json,
            digest: r.digest.clone(),
            signature: r.signature.as_ref().map(EvidenceSignatureRecord::from),
        })
    }
}

/// The result of [`super::CryptoHandle::verify_evidence`]: exact field set of
/// `genaryx_connectors::VerifyOutcome`. `verified: false` is a real "not
/// verified" answer, not an error - see the connector's own doc.
#[derive(Debug, Clone, uniffi::Record)]
pub struct VerifyOutcomeRecord {
    pub verified: bool,
    pub message: String,
}

impl From<&ConnVerifyOutcome> for VerifyOutcomeRecord {
    fn from(v: &ConnVerifyOutcome) -> Self {
        Self {
            verified: v.verified,
            message: v.message.clone(),
        }
    }
}

// ============================================================================
// error taxonomy
// ============================================================================

/// Every failure mode a [`super::CryptoHandle`] call can surface, fail-closed
/// throughout (06 §0.5). Collapsed from `genaryx_connectors::QryxError`'s
/// three variants, plus [`Self::NoEnvironment`] - an ffi-layer-only addition
/// with no connector-level equivalent, exactly like `IdryxError::NoEnvironment`.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CryptoError {
    /// [`super::env::discover`] found no `qryx` binary at all - a normal,
    /// renderable "no crypto plane" outcome (docs/PHASE4.md W1: "An absent
    /// source (no... `qryx` binary) must render as an HONEST first-class
    /// empty state"), not a bug.
    #[error("no crypto plane found (no qryx binary at ~/.taipan/bin/qryx)")]
    NoEnvironment,
    /// The qryx binary could not be spawned - missing, not executable, or an
    /// operator-supplied path via [`super::CryptoHandle::connect`] that does
    /// not exist. Distinct from [`Self::NoEnvironment`]: a binary WAS named,
    /// just could not be run.
    #[error("could not run qryx at {bin}: {reason}")]
    Spawn { bin: String, reason: String },
    /// A scan/verify subcommand exited nonzero - qryx's own stderr, verbatim.
    #[error("qryx exited {code}: {stderr}")]
    Cli { code: i32, stderr: String },
    /// A `--format <json>` stdout failed to parse into the expected shape, or
    /// (see [`EvidenceReportRecord::from_conn`]) `assets` failed to
    /// re-serialize.
    #[error("could not parse qryx output: {reason}")]
    Json { reason: String },
}

impl From<ConnQryxError> for CryptoError {
    fn from(e: ConnQryxError) -> Self {
        match e {
            ConnQryxError::Spawn { bin, source } => CryptoError::Spawn {
                bin,
                reason: source.to_string(),
            },
            ConnQryxError::Cli { code, stderr } => CryptoError::Cli { code, stderr },
            ConnQryxError::Json(source) => CryptoError::Json {
                reason: source.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(algorithm: &str) -> ConnNcscFinding {
        ConnNcscFinding {
            algorithm: algorithm.to_string(),
            asset_type: "public-key".to_string(),
            severity: "high".to_string(),
            occurrences: 2,
            locations: vec!["a.go:10".to_string()],
            externally_facing: true,
            long_lived_data: false,
            planned: false,
        }
    }

    #[test]
    fn counts_from_preserves_every_entry() {
        let mut map = BTreeMap::new();
        map.insert("code".to_string(), 3);
        map.insert("certs".to_string(), 1);
        let entries = counts_from(&map);
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|e| e.key == "code" && e.count == 3));
        assert!(entries.iter().any(|e| e.key == "certs" && e.count == 1));
    }

    #[test]
    fn ncsc_priority_record_keeps_migrated_count_verbatim_even_when_zero() {
        let conn = ConnNcscPriority {
            verdict: "not-started".to_string(),
            criteria: "quantum-vulnerable AND (externally-facing OR long-lived)".to_string(),
            count: 1,
            migrated_count: 0,
            remaining_count: 1,
            note: String::new(),
            findings: vec![finding("RSA-2048")],
        };
        let record = NcscPriorityRecord::from(&conn);
        assert_eq!(record.migrated_count, 0);
        assert_eq!(record.remaining_count, 1);
        assert_eq!(record.findings.len(), 1);
        assert_eq!(record.findings[0].algorithm, "RSA-2048");
    }

    #[test]
    fn evidence_report_record_serializes_assets_to_a_json_string() {
        let conn = ConnEvidenceReport {
            tool: "qryx".to_string(),
            version: "0.4.0".to_string(),
            standard: "CNSA 2.0".to_string(),
            generated_at: "2026-07-17T10:00:00Z".to_string(),
            root: "/repo".to_string(),
            summary: ConnEvidenceSummary {
                compliant: 8,
                non_compliant: 2,
                issues: 2,
                total: 10,
                score_pct: 80,
                by_severity: BTreeMap::new(),
            },
            assets: vec![serde_json::json!({"algorithm": "RSA-2048", "compliant": false})],
            digest: "sha256:abcdef".to_string(),
            signature: Some(ConnSignature {
                alg: "ml-dsa-65".to_string(),
                value: "BASE64SIG".to_string(),
                public_key: "BASE64SPKI".to_string(),
            }),
        };
        let record = EvidenceReportRecord::from_conn(&conn).expect("serializes cleanly");
        assert_eq!(record.digest, "sha256:abcdef");
        assert_eq!(record.summary.score_pct, 80);
        let sig = record.signature.expect("signed");
        assert_eq!(sig.alg, "ml-dsa-65");
        assert!(record.assets_json.contains("RSA-2048"));

        let parsed: serde_json::Value = serde_json::from_str(&record.assets_json)
            .expect("assets_json must itself be valid JSON");
        assert!(parsed.is_array());
    }

    #[test]
    fn evidence_report_record_unsigned_variant_has_no_signature() {
        let conn = ConnEvidenceReport {
            tool: "qryx".to_string(),
            version: "0.4.0".to_string(),
            standard: "CNSA 2.0".to_string(),
            generated_at: "2026-07-17T10:00:00Z".to_string(),
            root: "/repo".to_string(),
            summary: ConnEvidenceSummary {
                compliant: 10,
                non_compliant: 0,
                issues: 0,
                total: 10,
                score_pct: 100,
                by_severity: BTreeMap::new(),
            },
            assets: vec![],
            digest: "sha256:beef".to_string(),
            signature: None,
        };
        let record = EvidenceReportRecord::from_conn(&conn).expect("serializes cleanly");
        assert!(record.signature.is_none());
        assert_eq!(record.assets_json, "[]");
    }
}
