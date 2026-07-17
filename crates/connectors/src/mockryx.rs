//! `MockryxClient`: a `ToolRunner` over the Mockryx fire-drill CLI
//! (docs/PHASE4.md W2) - the drills plane the console's Drills panel runs and
//! renders. Grounded in the mockryx Go source (`~/Development/mockryx`, commit
//! `50579989`, read 2026-07-17).
//!
//! ## What a drill is, and how it is invoked
//!
//! `mockryx run [flags] <scenario-dir>` rehearses adversarial scenarios against
//! a live TokenFuse gateway (it POSTs crafted requests to `<gateway>/v1/messages`
//! and checks each guardrail's response), then reports whether each guardrail
//! HELD or GAPPED. This client shells it with `--format json` and parses the
//! report off stdout. Flags may precede or follow the positional (mockryx's
//! `parseArgsAnyOrder`), but this always puts them first. The scenario dir is
//! any directory of `.yaml`/`.yml`/`.json` scenarios (loaded non-recursively,
//! sorted); there is no built-in scenario enum, so the panel's env layer
//! resolves which directory to run (the mockryx checkout's shipped `scenarios/`
//! is the usual one).
//!
//! ## Exit codes are signal, like Qryx
//!
//! mockryx exits `0` when every rehearsed guardrail held, `1` when the run
//! completed and found at least one defensive GAP (a `findingsError`), and `2`
//! for a real usage error (bad flag, MISSING GATEWAY, unreadable scenario dir).
//! Crucially the JSON report is printed on BOTH exit 0 and 1 - a gap is a
//! finding in the report, not a tool failure - so [`MockryxClient::run`] parses
//! stdout on `0|1` and only treats `2` (and spawn failure) as
//! [`MockryxError`]. An unreachable gateway is indistinguishable at the
//! exit-code level from a real gap (it becomes a transport-error Finding, exit
//! 1); the report's `detail` text is where that shows.
//!
//! ## `--fail-on-skip` is honest about skips
//!
//! A scenario whose guardrail feature was never observed active reports
//! `skipped_not_configured` and parks its mismatches in `skipped_findings`
//! (NOT counted as gaps by default - a genuine skip is not a gap on its own).
//! `--fail-on-skip` promotes those into `findings` (turning exit 0 into 1) but
//! does NOT change the `status` string, so the panel must read "gap" from a
//! non-empty `findings` list or `status == "failed"`, never from `status`
//! alone.
//!
//! ## Fail-closed
//!
//! No panics. A spawn failure is [`MockryxError::Spawn`] (the live test reads it
//! as "mockryx absent, skip"); a `2`/other nonzero exit is
//! [`MockryxError::Cli`] with mockryx's stderr; unparseable stdout is
//! [`MockryxError::Json`]. The caller supplies the resolved binary path,
//! gateway URL, and scenario dir (env discovery is the shell's job, like every
//! other connector).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ---- error -----------------------------------------------------------------

/// Every failure mode a [`MockryxClient`] call can surface. Fail-closed:
/// spawn, real-error-exit, and parse failures are distinct, never a panic.
#[derive(Debug, thiserror::Error)]
pub enum MockryxError {
    /// The mockryx binary could not be spawned (missing, not executable).
    #[error("mockryx spawn {bin}: {source}")]
    Spawn {
        bin: String,
        #[source]
        source: std::io::Error,
    },

    /// mockryx exited `2` (or another non-{0,1} code): a real usage/config
    /// error - a bad flag, a missing gateway, an unreadable scenario dir -
    /// where nothing was actually rehearsed. Carries the code and stderr.
    /// (Exit `1` is NOT here: it means gaps were found, a normal result whose
    /// report is on stdout.)
    #[error("mockryx exited {code}: {stderr}")]
    Cli { code: i32, stderr: String },

    /// A saved or stdout report that failed to deserialize - this client's
    /// DTOs have drifted from mockryx's `report.Report` shape.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// A saved report file could not be read (for [`MockryxClient::load_report`]).
    #[error("read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

// ---- DTOs (exact wire shapes, mockryx internal/report + internal/runner) ----

/// The whole drill report (`report.Report`, `internal/report/report.go:18-23`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct MockryxReport {
    pub run_id: String,
    /// The gateway base URL the drill rehearsed against.
    pub gateway: String,
    /// RFC3339Nano UTC when the report was generated. NOTE the wire field is
    /// `generated_at` (a `time.Time`), not `generated`.
    pub generated_at: String,
    /// One entry per scenario, in load order. Always present (may be empty).
    #[serde(default, deserialize_with = "crate::null_default")]
    pub results: Vec<MockryxResult>,
}

impl MockryxReport {
    /// Whether the drill found any defensive gap: any scenario that outright
    /// `failed`, or any scenario carrying findings (which, after
    /// `--fail-on-skip`, can include promoted skips). A `skipped_not_configured`
    /// scenario with an empty `findings` is NOT a gap on its own.
    pub fn has_gaps(&self) -> bool {
        self.results
            .iter()
            .any(|r| r.status == "failed" || !r.findings.is_empty())
    }
}

/// One scenario's outcome (`runner.Result`, `internal/runner/runner.go:91-109`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct MockryxResult {
    pub scenario: String,
    /// One of `passed` | `failed` | `skipped_not_configured` (`runner.Status`,
    /// exhaustive). Read "gap" from `findings`/`failed`, not this alone (see
    /// the module doc on `--fail-on-skip`).
    pub status: String,
    /// The mismatches that count as gaps (present when `failed`, or when
    /// `--fail-on-skip` promoted skips into it).
    #[serde(default, deserialize_with = "crate::null_default")]
    pub findings: Vec<MockryxFinding>,
    /// Mismatches discarded because the scenario's guardrail was never observed
    /// active (only on `skipped_not_configured`, absent unless present).
    #[serde(default, deserialize_with = "crate::null_default")]
    pub skipped_findings: Vec<MockryxFinding>,
    pub metrics: MockryxMetrics,
}

/// One step mismatch (`runner.Finding`, `internal/runner/runner.go:63-80`).
/// Rich by design: the panel shows exactly what was expected vs what the
/// gateway returned.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct MockryxFinding {
    pub scenario: String,
    pub step: String,
    pub attempt: i64,
    pub expect_status: i64,
    #[serde(default)]
    pub expect_header: Option<BTreeMap<String, String>>,
    pub got_status: i64,
    #[serde(default)]
    pub got_headers: Option<BTreeMap<String, String>>,
    pub detail: String,
    /// Set only for a failed `expect.event` check (a downstream product's
    /// agent-event that should have fired but did not).
    #[serde(default)]
    pub expect_event_source: Option<String>,
    #[serde(default)]
    pub expect_event_type: Option<String>,
}

/// Per-scenario blast-radius metrics (`runner.Metrics`,
/// `internal/runner/runner.go:85-88`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct MockryxMetrics {
    pub calls: i64,
    pub budget_burned_usd: f64,
}

// ---- client ----------------------------------------------------------------

/// A `ToolRunner` over the mockryx CLI. Holds the resolved binary path; `run`
/// is one synchronous `mockryx run …` invocation (a batch job the caller runs
/// off the UI thread, like [`crate::QryxClient`]).
#[derive(Debug, Clone)]
pub struct MockryxClient {
    bin: PathBuf,
}

impl MockryxClient {
    /// Construct a client for a resolved `mockryx` binary path.
    pub fn new(bin: impl Into<PathBuf>) -> Self {
        Self { bin: bin.into() }
    }

    /// `mockryx run --gateway <gateway> --format json [--api-key K]
    /// [--fail-on-skip] [--save P] <scenario_dir>` -> the parsed
    /// [`MockryxReport`]. Exit `0|1` both yield a report (exit 1 = gaps found,
    /// a normal result); exit `2`/other is a real [`MockryxError::Cli`].
    ///
    /// `gateway` is the TokenFuse gateway base URL to rehearse against
    /// (required by mockryx; an empty one makes mockryx exit 2). `save_path`,
    /// when given, also writes the JSON report there for later `report`/Evidence
    /// use; the return value is parsed from stdout regardless.
    pub fn run(
        &self,
        scenario_dir: &Path,
        gateway: &str,
        api_key: Option<&str>,
        fail_on_skip: bool,
        save_path: Option<&Path>,
    ) -> Result<MockryxReport, MockryxError> {
        let dir = scenario_dir.to_string_lossy();
        let save = save_path.map(|p| p.to_string_lossy());
        let mut args: Vec<&str> = vec!["run", "--gateway", gateway, "--format", "json"];
        if let Some(key) = api_key {
            args.push("--api-key");
            args.push(key);
        }
        if fail_on_skip {
            args.push("--fail-on-skip");
        }
        if let Some(ref s) = save {
            args.push("--save");
            args.push(s);
        }
        args.push(&dir);

        let out = std::process::Command::new(&self.bin)
            .args(&args)
            .output()
            .map_err(|source| MockryxError::Spawn {
                bin: self.bin.display().to_string(),
                source,
            })?;

        // Exit 0 (all held) and 1 (gaps found) both print the JSON report on
        // stdout; only 2/other means nothing ran.
        let code = out.status.code().unwrap_or(-1);
        if code != 0 && code != 1 {
            return Err(MockryxError::Cli {
                code,
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
        }
        Ok(serde_json::from_slice(&out.stdout)?)
    }

    /// Re-load a previously `--save`d report (or any mockryx JSON report) from
    /// disk, without re-running the drill. The saved file is the same
    /// [`MockryxReport`] JSON, so this reads and parses it directly rather than
    /// shelling `mockryx report`. Handy for the Drills panel's "last run" view
    /// and for the Evidence Center (W3).
    pub fn load_report(path: &Path) -> Result<MockryxReport, MockryxError> {
        let bytes = std::fs::read(path).map_err(|source| MockryxError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The exact JSON mockryx's report.Save / --format json emit, parsed offline.
    // A live `mockryx run` against a real gateway lives in tests/, skip-gracefully.

    const REPORT: &[u8] = br#"{
      "run_id": "run-abc",
      "gateway": "http://127.0.0.1:4100",
      "generated_at": "2026-07-17T12:34:56.789012Z",
      "results": [
        {
          "scenario": "runaway-budget",
          "status": "passed",
          "metrics": {"calls": 8, "budget_burned_usd": 0.0}
        },
        {
          "scenario": "wardryx-denied-tool",
          "status": "failed",
          "findings": [
            {"scenario":"wardryx-denied-tool","step":"call-denied-tool","attempt":1,
             "expect_status":403,"expect_header":{"x-fuse-wardryx":"deny"},
             "got_status":200,"detail":"guardrail did not deny"}
          ],
          "metrics": {"calls": 3, "budget_burned_usd": 0.012}
        },
        {
          "scenario": "dlp-secret-leak",
          "status": "skipped_not_configured",
          "skipped_findings": [
            {"scenario":"dlp-secret-leak","step":"leak","attempt":1,"expect_status":403,
             "got_status":200,"detail":"dlp feature never observed active"}
          ],
          "metrics": {"calls": 1, "budget_burned_usd": 0.001}
        }
      ]
    }"#;

    #[test]
    fn report_parses_all_three_status_shapes() {
        let rep: MockryxReport = serde_json::from_slice(REPORT).expect("parse report");
        assert_eq!(rep.run_id, "run-abc");
        assert_eq!(rep.results.len(), 3);

        let passed = &rep.results[0];
        assert_eq!(passed.status, "passed");
        assert!(passed.findings.is_empty());
        assert_eq!(passed.metrics.calls, 8);

        let failed = &rep.results[1];
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.findings.len(), 1);
        let f = &failed.findings[0];
        assert_eq!(f.expect_status, 403);
        assert_eq!(f.got_status, 200);
        assert_eq!(
            f.expect_header.as_ref().unwrap().get("x-fuse-wardryx"),
            Some(&"deny".to_string())
        );

        let skipped = &rep.results[2];
        assert_eq!(skipped.status, "skipped_not_configured");
        assert!(skipped.findings.is_empty());
        assert_eq!(skipped.skipped_findings.len(), 1);
    }

    #[test]
    fn has_gaps_counts_failed_and_findings_not_bare_skips() {
        let rep: MockryxReport = serde_json::from_slice(REPORT).expect("parse");
        // One `failed` result -> gaps, even though the skip alone would not count.
        assert!(rep.has_gaps());

        // A report that only passed + bare-skipped is NOT a gap.
        let clean = br#"{"run_id":"r","gateway":"g","generated_at":"t","results":[
          {"scenario":"a","status":"passed","metrics":{"calls":1,"budget_burned_usd":0.0}},
          {"scenario":"b","status":"skipped_not_configured",
           "skipped_findings":[{"scenario":"b","step":"s","attempt":1,"expect_status":403,"got_status":200,"detail":"d"}],
           "metrics":{"calls":1,"budget_burned_usd":0.0}}
        ]}"#;
        let rep2: MockryxReport = serde_json::from_slice(clean).expect("parse");
        assert!(!rep2.has_gaps(), "a bare skip is not a gap");
    }

    #[test]
    fn empty_results_parse() {
        let rep: MockryxReport = serde_json::from_slice(
            br#"{"run_id":"r","gateway":"g","generated_at":"t","results":[]}"#,
        )
        .expect("parse");
        assert!(rep.results.is_empty());
        assert!(!rep.has_gaps());
    }

    #[test]
    fn run_against_a_missing_binary_is_fail_closed_spawn_error() {
        let c = MockryxClient::new("/nonexistent/mockryx-binary-xyz");
        match c.run(
            Path::new("/scenarios"),
            "http://127.0.0.1:4100",
            None,
            false,
            None,
        ) {
            Err(MockryxError::Spawn { .. }) => {}
            other => panic!("expected Spawn error, got {other:?}"),
        }
    }

    #[test]
    fn load_report_missing_file_is_fail_closed() {
        match MockryxClient::load_report(Path::new("/nonexistent/report.json")) {
            Err(MockryxError::Read { .. }) => {}
            other => panic!("expected Read error, got {other:?}"),
        }
    }

    #[test]
    fn go_nil_results_slice_as_null_parses_as_empty() {
        // `results` has no `omitempty`, so a mockryx run with zero scenarios
        // emits `"results": null` (Go nil slice), which must parse to an empty
        // Vec via `null_default`, not fail. (Same Go-nil-slice class the qryx
        // live shape test caught.)
        let rep: MockryxReport = serde_json::from_slice(
            br#"{"run_id":"r","gateway":"g","generated_at":"t","results":null}"#,
        )
        .expect("null results parses as empty");
        assert!(rep.results.is_empty());
        assert!(!rep.has_gaps());
    }
}
