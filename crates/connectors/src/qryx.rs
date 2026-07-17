//! `QryxClient`: a `ToolRunner` over the Qryx crypto-plane CLI (docs/PHASE4.md
//! W1) - the post-quantum/crypto-agility plane the console's Crypto panel
//! renders from and the Evidence Center (W3) builds signed bundles with.
//! Grounded in the qryx Go source (`~/Development/qryx`, read 2026-07-17).
//!
//! ## A CLI wrapper, because Qryx's machine surface IS `--format`
//!
//! Unlike [`crate::VerdryxClient`] (a SQLite reader, since verdryx has no JSON
//! output), qryx's structured output is its `--format` flag on the read
//! subcommands (`scan`/`tls`/`bin`/`image`/`aws`/`gcp`/`azure`/`agents`,
//! `cmd/qryx/main.go`). The machine-readable formats are `cbom` (CycloneDX
//! 1.6), `cnsa`, `evidence`, `ncsc`, and `migration` - all JSON. This client
//! shells the binary with an explicit `--format` and parses stdout. The panel's
//! headline is `--format ncsc` (the NCSC 2028/2031/2035 PQC migration timeline,
//! `internal/report/ncsc.go`); the inventory is `--format cbom`; the signed
//! attestation is `--format evidence` (`internal/report/evidence.go`).
//!
//! Flags precede the positional path (`qryx <cmd> [flags] <path>`; Go's `flag`
//! stops at the first positional, per qryx's own CLAUDE.md), so every arg list
//! here puts `--format …`/`--sign-key …` before the target.
//!
//! ## Exit codes are signal, not just success/failure
//!
//! qryx exits `0` ok, `1` on a real error, `2` when `--fail-on <sev>` trips, and
//! `3` on a `--policy` violation or a `trend --fail-on-regression`
//! (`cmd/qryx/main.go`; qryx CLAUDE.md). The panel's read methods here pass
//! NEITHER `--fail-on` nor `--policy`, so a successful scan is always `0` and any
//! nonzero is a genuine failure -> [`QryxError::Cli`] carrying qryx's stderr.
//! (A gate-flag-carrying call is a W3/CI concern; when we add it we will read
//! the code explicitly rather than treat 2/3 as errors.) [`QryxClient::verify_evidence`]
//! is the one method that reads the exit code itself: `verify-evidence` exits `0`
//! for VERIFIED and nonzero for "not verified," and BOTH are legitimate answers,
//! never a connector error - only a spawn failure is.
//!
//! ## Fail-closed (06 §0.5)
//!
//! No panics, no `unwrap`/`expect`. A spawn failure becomes
//! [`QryxError::Spawn`]; a nonzero exit on a read method becomes
//! [`QryxError::Cli`]; unparseable stdout becomes [`QryxError::Json`]. The
//! caller supplies the resolved qryx binary path (descriptor/checkout discovery
//! is the env layer's job, exactly like [`crate::IdryxClient::rescan`]); a
//! missing binary therefore surfaces as [`QryxError::Spawn`], which the live
//! test treats as "skip."

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ---- error -----------------------------------------------------------------

/// Every failure mode a [`QryxClient`] call can surface. Fail-closed: spawn,
/// nonzero-exit, and parse failures are distinct variants, never a panic.
#[derive(Debug, thiserror::Error)]
pub enum QryxError {
    /// The qryx binary could not be spawned (missing, not executable). The live
    /// test reads this as "qryx absent, skip."
    #[error("qryx spawn {bin}: {source}")]
    Spawn {
        bin: String,
        #[source]
        source: std::io::Error,
    },

    /// A read subcommand exited nonzero (no gate flags were passed, so this is a
    /// genuine error, not a `--fail-on`/`--policy` signal). Carries the exit
    /// code and qryx's stderr.
    #[error("qryx exited {code}: {stderr}")]
    Cli { code: i32, stderr: String },

    /// A `--format <json>` stdout that failed to deserialize into the expected
    /// shape - this client's DTOs have drifted from qryx's output.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

// ---- DTOs: --format ncsc (internal/report/ncsc.go) --------------------------

/// `--format ncsc`: the NCSC PQC migration-timeline report the Crypto panel's
/// headline renders (`ncscReport`, `internal/report/ncsc.go:222-229`). Three
/// milestones (2028 discovery / 2031 highest-priority / 2035 full) over qryx's
/// shared crypto-asset graph, each with a deterministic verdict.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct NcscReport {
    /// Fixed label, `"NCSC PQC migration timeline (2028/2031/2035)"`
    /// (`ncsc.go:36`).
    pub standard: String,
    #[serde(rename = "generatedAt")]
    pub generated_at: String,
    pub root: String,
    #[serde(rename = "discovery2028")]
    pub discovery_2028: NcscDiscovery,
    #[serde(rename = "highestPriority2031")]
    pub highest_priority_2031: NcscPriority,
    #[serde(rename = "fullMigration2035")]
    pub full_migration_2035: NcscFullMigration,
}

/// The 2028 "complete discovery" milestone (`ncscDiscovery`, `ncsc.go:193-201`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NcscDiscovery {
    /// One of `on-track` | `at-risk` | `not-started` (`ncscVerdict`,
    /// `ncsc.go:162-164`).
    pub verdict: String,
    /// Inventoried asset count per source bucket (e.g. `code`, `certs`, `tls`).
    #[serde(default)]
    pub coverage_by_source: BTreeMap<String, i64>,
    pub total_inventoried: i64,
    pub quantum_vulnerable_count: i64,
    pub migration_plan_exists: bool,
    #[serde(default)]
    pub migration_plan_note: String,
    #[serde(default)]
    pub quantum_vulnerable_findings: Vec<NcscFinding>,
}

/// The 2031 "highest-priority systems" milestone (`ncscHighestPriority`,
/// `ncsc.go:204-212`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NcscPriority {
    pub verdict: String,
    /// The human-readable subset criteria (quantum-vulnerable AND
    /// externally-facing-or-long-lived).
    #[serde(default)]
    pub criteria: String,
    pub count: i64,
    /// Always `0`: qryx does not track remediation state across runs; progress
    /// is the `--baseline` drift / evidence-trail's job, not this report's
    /// (`ncsc.go:231-236`). The panel must label it accordingly, not show
    /// "0 migrated" as real progress.
    pub migrated_count: i64,
    pub remaining_count: i64,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub findings: Vec<NcscFinding>,
}

/// The 2035 "all systems" milestone (`ncscFullMigration`, `ncsc.go:215-219`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct NcscFullMigration {
    pub verdict: String,
    pub count: i64,
    #[serde(default)]
    pub findings: Vec<NcscFinding>,
}

/// One quantum-vulnerable asset in a milestone finding list (`ncscFindingJSON`,
/// `ncsc.go:168-178`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NcscFinding {
    pub algorithm: String,
    /// Asset type (e.g. `public-key`, `certificate`); `type` on the wire.
    #[serde(rename = "type")]
    pub asset_type: String,
    pub severity: String,
    pub occurrences: i64,
    #[serde(default)]
    pub locations: Vec<String>,
    pub externally_facing: bool,
    pub long_lived_data: bool,
    pub planned: bool,
}

// ---- DTOs: --format evidence (internal/report/evidence.go) ------------------

/// `--format evidence`: a CNSA 2.0 compliance attestation with a SHA-256 content
/// digest (computed with the `digest` field blanked, so it self-verifies without
/// keys) and an optional detached signature (`evidenceReport`,
/// `internal/report/evidence.go:20-29`). The Evidence Center (W3) emits and
/// re-verifies these.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct EvidenceReport {
    pub tool: String,
    pub version: String,
    pub standard: String,
    #[serde(rename = "generatedAt")]
    pub generated_at: String,
    pub root: String,
    pub summary: EvidenceSummary,
    /// Per-asset CNSA rows (`cnsaAssetJSON`). Kept as raw JSON: it is a large,
    /// display-only shape the panel renders as a table, not something this
    /// connector reasons over.
    #[serde(default)]
    pub assets: Vec<serde_json::Value>,
    /// `"sha256:<hex>"` over the canonical report with `digest` blanked
    /// (`evidence.go:28`). The security-critical field, so it is typed, not
    /// left in the raw blob.
    pub digest: String,
    /// Present only when built with `--sign-key`; a detached signature over the
    /// digest embedding the signer's SPKI public key (`evidence.go:29`).
    #[serde(default)]
    pub signature: Option<Signature>,
}

/// The compliance rollup in an [`EvidenceReport`] (`evidenceSummary`,
/// `evidence.go:32-38`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceSummary {
    pub compliant: i64,
    pub non_compliant: i64,
    pub issues: i64,
    pub total: i64,
    /// Compliance score as an integer percent (`scorePct`).
    pub score_pct: i64,
    #[serde(default)]
    pub by_severity: BTreeMap<String, i64>,
}

/// A detached evidence signature (`attest.Signature`,
/// `internal/attest/attest.go:45-49`). `alg` is one of `ed25519`,
/// `ecdsa-p256`, or `ml-dsa-44|65|87` (FIPS 204); `public_key` is the base64
/// SPKI so the bundle self-verifies.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Signature {
    pub alg: String,
    pub value: String,
    #[serde(rename = "publicKey")]
    pub public_key: String,
}

/// The result of [`QryxClient::verify_evidence`]. `verified` reflects qryx's
/// exit status (0 = VERIFIED); `message` is qryx's own stdout/stderr line so the
/// panel can show the algorithm + key fingerprint it reported (or the failure
/// reason). A `false` here is a real "not verified" answer, not an error.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct VerifyOutcome {
    pub verified: bool,
    pub message: String,
}

// ---- client ----------------------------------------------------------------

/// A `ToolRunner` over the qryx CLI. Holds the resolved binary path; every
/// method is one synchronous `qryx <cmd> …` invocation (a batch job the caller
/// runs off the UI thread, mirroring [`crate::IdryxClient::rescan`]).
#[derive(Debug, Clone)]
pub struct QryxClient {
    bin: PathBuf,
}

impl QryxClient {
    /// Construct a client for a resolved `qryx` binary path.
    pub fn new(bin: impl Into<PathBuf>) -> Self {
        Self { bin: bin.into() }
    }

    /// Spawn qryx with `args`, returning its raw [`std::process::Output`]. Only a
    /// spawn failure is an error here; interpreting the exit code is each
    /// method's job (read methods treat nonzero as [`QryxError::Cli`];
    /// [`Self::verify_evidence`] reads it as a verdict).
    fn run_raw(&self, args: &[&str]) -> Result<std::process::Output, QryxError> {
        std::process::Command::new(&self.bin)
            .args(args)
            .output()
            .map_err(|source| QryxError::Spawn {
                bin: self.bin.display().to_string(),
                source,
            })
    }

    /// Run a read subcommand and parse its `--format <json>` stdout as `T`. A
    /// nonzero exit (no gate flags are passed) is a genuine error.
    fn run_json<T: serde::de::DeserializeOwned>(&self, args: &[&str]) -> Result<T, QryxError> {
        let out = self.run_raw(args)?;
        if !out.status.success() {
            return Err(QryxError::Cli {
                code: out.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
        }
        Ok(serde_json::from_slice(&out.stdout)?)
    }

    /// `qryx scan --format ncsc <path>` -> the [`NcscReport`] PQC timeline.
    pub fn scan_ncsc(&self, path: &Path) -> Result<NcscReport, QryxError> {
        let p = path.to_string_lossy();
        self.run_json(&["scan", "--format", "ncsc", &p])
    }

    /// `qryx scan --format cbom <path>` -> the CycloneDX 1.6 CBOM as raw JSON.
    /// Kept untyped: CycloneDX is a large, stable, external schema the panel
    /// renders as an inventory, not a shape this connector reasons over.
    pub fn scan_cbom(&self, path: &Path) -> Result<serde_json::Value, QryxError> {
        let p = path.to_string_lossy();
        self.run_json(&["scan", "--format", "cbom", &p])
    }

    /// `qryx scan --format evidence [--sign-key <pem>] <path>` -> the
    /// [`EvidenceReport`] CNSA attestation. Pass `sign_key` to embed a detached
    /// signature (ed25519 / ecdsa-p256 / ml-dsa-*); omit it for an unsigned,
    /// still-self-verifying (digest-only) bundle.
    pub fn scan_evidence(
        &self,
        path: &Path,
        sign_key: Option<&Path>,
    ) -> Result<EvidenceReport, QryxError> {
        let p = path.to_string_lossy();
        let key;
        let args: Vec<&str> = match sign_key {
            Some(k) => {
                key = k.to_string_lossy();
                vec!["scan", "--format", "evidence", "--sign-key", &key, &p]
            }
            None => vec!["scan", "--format", "evidence", &p],
        };
        self.run_json(&args)
    }

    /// `qryx agents --format evidence <path>` -> a CNSA attestation scoped to the
    /// agent-governance stack's own trust surface (Agent Passport attestation
    /// crypto + agent-event hash-chain integrity, `internal/agentstack`).
    /// `agents` has no dedicated format; it reuses `--format`, so `evidence`
    /// gives the Crypto panel a structured per-asset view here.
    pub fn agents_evidence(&self, path: &Path) -> Result<EvidenceReport, QryxError> {
        let p = path.to_string_lossy();
        self.run_json(&["agents", "--format", "evidence", &p])
    }

    /// `qryx verify-evidence <file>` -> recompute the digest AND check the
    /// signature. Reads the exit code itself: `0` = VERIFIED, nonzero = not
    /// verified; both are legitimate [`VerifyOutcome`]s, and only a spawn
    /// failure is a [`QryxError`]. The `message` carries qryx's own reported
    /// line (algorithm + key fingerprint on success, reason on failure).
    pub fn verify_evidence(&self, file: &Path) -> Result<VerifyOutcome, QryxError> {
        let f = file.to_string_lossy();
        let out = self.run_raw(&["verify-evidence", &f])?;
        let verified = out.status.success();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        // qryx prints the verdict on stdout ("evidence: VERIFIED (…)"); on
        // failure the reason may be on stderr. Prefer whichever is non-empty.
        let message = {
            let s = stdout.trim();
            if s.is_empty() {
                stderr.trim().to_string()
            } else {
                s.to_string()
            }
        };
        Ok(VerifyOutcome { verified, message })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The exact shapes qryx's ncsc.go / evidence.go emit, parsed offline (no
    // live qryx). A live scan + verify against a real built qryx binary lives in
    // tests/qryx_test.rs, skip-gracefully when qryx is absent.

    #[test]
    fn ncsc_report_parses_all_three_milestones() {
        let json = br#"{
          "standard":"NCSC PQC migration timeline (2028/2031/2035)",
          "generatedAt":"2026-07-17T10:00:00Z",
          "root":"/repo",
          "discovery2028":{
            "verdict":"at-risk",
            "coverageBySource":{"code":3,"certs":1},
            "totalInventoried":4,
            "quantumVulnerableCount":2,
            "migrationPlanExists":false,
            "migrationPlanNote":"no plan artifact",
            "quantumVulnerableFindings":[
              {"algorithm":"RSA-2048","type":"public-key","severity":"high","occurrences":2,
               "locations":["a.go:10","b.go:20"],"externallyFacing":true,"longLivedData":false,"planned":false}
            ]
          },
          "highestPriority2031":{
            "verdict":"not-started","criteria":"quantum-vulnerable AND (externally-facing OR long-lived)",
            "count":1,"migratedCount":0,"remainingCount":1,"note":"...","findings":[]
          },
          "fullMigration2035":{"verdict":"not-started","count":2,"findings":[]}
        }"#;
        let rep: NcscReport = serde_json::from_slice(json).expect("parse ncsc");
        assert_eq!(rep.discovery_2028.verdict, "at-risk");
        assert_eq!(rep.discovery_2028.total_inventoried, 4);
        assert_eq!(rep.discovery_2028.quantum_vulnerable_count, 2);
        assert!(!rep.discovery_2028.migration_plan_exists);
        assert_eq!(rep.discovery_2028.coverage_by_source.get("code"), Some(&3));
        let f = &rep.discovery_2028.quantum_vulnerable_findings[0];
        assert_eq!(f.algorithm, "RSA-2048");
        assert_eq!(f.asset_type, "public-key");
        assert!(f.externally_facing && !f.long_lived_data);
        assert_eq!(f.locations.len(), 2);
        // 2031 migratedCount is honestly 0 (no cross-run remediation state).
        assert_eq!(rep.highest_priority_2031.migrated_count, 0);
        assert_eq!(rep.highest_priority_2031.remaining_count, 1);
        assert_eq!(rep.full_migration_2035.count, 2);
    }

    #[test]
    fn evidence_report_typed_digest_and_optional_signature() {
        // Signed variant: digest + signature present, assets kept as raw JSON.
        let signed = br#"{
          "tool":"qryx","version":"0.4.0","standard":"CNSA 2.0",
          "generatedAt":"2026-07-17T10:00:00Z","root":"/repo",
          "summary":{"compliant":8,"nonCompliant":2,"issues":2,"total":10,"scorePct":80,
                     "bySeverity":{"high":1,"medium":1}},
          "assets":[{"algorithm":"RSA-2048","compliant":false}],
          "digest":"sha256:abcdef",
          "signature":{"alg":"ml-dsa-65","value":"BASE64SIG","publicKey":"BASE64SPKI"}
        }"#;
        let ev: EvidenceReport = serde_json::from_slice(signed).expect("parse evidence");
        assert_eq!(ev.summary.score_pct, 80);
        assert_eq!(ev.summary.non_compliant, 2);
        assert_eq!(ev.summary.by_severity.get("high"), Some(&1));
        assert_eq!(ev.digest, "sha256:abcdef");
        let sig = ev.signature.as_ref().expect("signed");
        assert_eq!(sig.alg, "ml-dsa-65");
        assert_eq!(sig.public_key, "BASE64SPKI");
        assert_eq!(ev.assets.len(), 1);

        // Unsigned variant: signature absent -> None, not an error.
        let unsigned = br#"{
          "tool":"qryx","version":"0.4.0","standard":"CNSA 2.0",
          "generatedAt":"2026-07-17T10:00:00Z","root":"/repo",
          "summary":{"compliant":10,"nonCompliant":0,"issues":0,"total":10,"scorePct":100,"bySeverity":{}},
          "assets":[],"digest":"sha256:beef"
        }"#;
        let ev2: EvidenceReport = serde_json::from_slice(unsigned).expect("parse unsigned");
        assert!(ev2.signature.is_none());
        assert_eq!(ev2.summary.score_pct, 100);
        assert!(ev2.assets.is_empty());
    }

    #[test]
    fn cli_error_is_fail_closed_when_binary_missing() {
        // A binary that cannot spawn -> Spawn error (this is exactly what the
        // live test reads as "qryx absent, skip").
        let c = QryxClient::new("/nonexistent/qryx-binary-xyz");
        match c.scan_ncsc(Path::new("/repo")) {
            Err(QryxError::Spawn { .. }) => {}
            other => panic!("expected Spawn error, got {other:?}"),
        }
    }
}
