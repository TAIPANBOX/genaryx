//! Live wire-shape tests: run the REAL Qryx and Verdryx tools/stores and prove
//! this crate's DTOs deserialize their actual output byte-for-byte. These exist
//! because the Engram `recall`/`why` shape bugs (fixed in `mcp_stdio.rs` +
//! `engram.rs`) were caught only by live-testing against a real server - no
//! offline fixture had exercised FastMCP's real serialization. These guards do
//! the same for the other two tool-backed connectors, so a future drift between
//! a hand-written DTO and the real wire shape fails loudly here.
//!
//! Both skip gracefully (an `eprintln!` + early return) when the tool/store is
//! not available, exactly like the idryx/cloud/wardryx live tests - they never
//! hard-fail a machine that lacks a sibling checkout.
//!
//! To run them against real artifacts:
//!   QRYX_BIN=/path/to/qryx QRYX_SCAN_TARGET=/some/crypto/dir \
//!   VERDRYX_DB=/path/to/verdryx.db \
//!   cargo test -p genaryx-connectors --test live_shapes_test -- --nocapture

use std::path::PathBuf;

use genaryx_connectors::{MockryxClient, QryxClient, VerdryxClient};

// ---- Qryx --------------------------------------------------------------------

/// Resolve a qryx binary: `$QRYX_BIN`, then the well-known taipan location, then
/// a sibling checkout's `bin/qryx`, then `PATH`. Returns `None` if none exists.
fn resolve_qryx() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("QRYX_BIN") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    for cand in [
        format!("{home}/.taipan/bin/qryx"),
        format!("{home}/Development/qryx/bin/qryx"),
    ] {
        let p = PathBuf::from(cand);
        if p.is_file() {
            return Some(p);
        }
    }
    which("qryx")
}

fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(bin))
        .find(|p| p.is_file())
}

/// A scan target with real crypto: `$QRYX_SCAN_TARGET`, else this crate's own
/// source (a small, always-present directory - even a crypto-free scan yields a
/// valid empty report, which still proves the top-level shape deserializes).
fn qryx_scan_target() -> PathBuf {
    if let Ok(t) = std::env::var("QRYX_SCAN_TARGET") {
        let p = PathBuf::from(t);
        if p.exists() {
            return p;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

#[test]
fn qryx_live_shapes_ncsc_cbom_evidence() {
    let Some(bin) = resolve_qryx() else {
        eprintln!("qryx_live_shapes: SKIP - no qryx binary (set QRYX_BIN)");
        return;
    };
    let client = QryxClient::new(&bin);
    let target = qryx_scan_target();

    // NCSC: the PQC timeline. A too-old qryx that predates `--format ncsc`
    // reports it as an error - treat that as SKIP (stale binary), not a failure.
    match client.scan_ncsc(&target) {
        Ok(rep) => {
            for verdict in [
                &rep.discovery_2028.verdict,
                &rep.highest_priority_2031.verdict,
                &rep.full_migration_2035.verdict,
            ] {
                assert!(
                    matches!(verdict.as_str(), "on-track" | "at-risk" | "not-started"),
                    "unexpected NCSC verdict from real qryx: {verdict:?} - DTO drift"
                );
            }
            // migratedCount is always 0 (no cross-run remediation state).
            assert_eq!(rep.highest_priority_2031.migrated_count, 0);
            eprintln!(
                "qryx_live_shapes: ncsc OK ({} quantum-vulnerable findings deserialized)",
                rep.discovery_2028.quantum_vulnerable_findings.len()
            );
        }
        Err(e) if e.to_string().contains("unknown format") => {
            eprintln!("qryx_live_shapes: SKIP ncsc - qryx too old for --format ncsc: {e}");
            return;
        }
        Err(e) => panic!("scan_ncsc against real qryx failed to deserialize: {e}"),
    }

    // CBOM: untyped serde_json::Value, so this proves the run + JSON parse, and
    // that it is the CycloneDX object shape (has bomFormat/components or at
    // least is an object).
    let cbom = client
        .scan_cbom(&target)
        .expect("scan_cbom real deserialize");
    assert!(cbom.is_object(), "CBOM should be a CycloneDX object");

    // Evidence: the CNSA attestation. Unsigned here (no --sign-key), so the
    // signature must be absent -> None, and a digest must be present.
    let ev = client
        .scan_evidence(&target, None)
        .expect("scan_evidence real deserialize");
    assert!(!ev.digest.is_empty(), "evidence digest present");
    assert!(ev.signature.is_none(), "unsigned evidence has no signature");
    assert_eq!(ev.tool, "qryx");
    eprintln!(
        "qryx_live_shapes: evidence OK (scorePct {}, {} assets)",
        ev.summary.score_pct,
        ev.assets.len()
    );
}

// ---- Mockryx (shape-only: no gateway needed) --------------------------------

#[test]
fn mockryx_live_load_report_shape() {
    // Exercises the report DTO against a report mockryx itself wrote, when one
    // is available at $MOCKRYX_REPORT. (A full `run` needs a live gateway, out
    // of scope for a shape test; `load_report` reads the same report.Report
    // JSON, so it validates the exact same deserialization path.)
    let Ok(path) = std::env::var("MOCKRYX_REPORT") else {
        eprintln!("mockryx_live_load_report_shape: SKIP - set MOCKRYX_REPORT to a saved report");
        return;
    };
    let path = PathBuf::from(path);
    if !path.is_file() {
        eprintln!("mockryx_live_load_report_shape: SKIP - MOCKRYX_REPORT not a file");
        return;
    }
    let rep = MockryxClient::load_report(&path).expect("load_report real deserialize");
    for r in &rep.results {
        assert!(
            matches!(
                r.status.as_str(),
                "passed" | "failed" | "skipped_not_configured"
            ),
            "unexpected mockryx status from real report: {:?} - DTO drift",
            r.status
        );
    }
    eprintln!(
        "mockryx_live_load_report_shape: OK ({} scenarios, has_gaps={})",
        rep.results.len(),
        rep.has_gaps()
    );
}

// ---- Verdryx -----------------------------------------------------------------

/// Resolve a real verdryx store: `$VERDRYX_DB`, then the well-known location.
fn resolve_verdryx_db() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("VERDRYX_DB") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let p = PathBuf::from(format!("{home}/.taipan/verdryx.db"));
    p.is_file().then_some(p)
}

#[test]
fn verdryx_live_shapes_runs_scores_baselines() {
    let Some(db) = resolve_verdryx_db() else {
        eprintln!("verdryx_live_shapes: SKIP - no verdryx.db (set VERDRYX_DB)");
        return;
    };
    let client = VerdryxClient::open(&db).expect("open a real verdryx.db read-only");

    // Every eval_runs row deserializes (id/model/started_at/finished_at,
    // including a NULL finished_at for an in-flight run).
    let runs = client
        .list_eval_runs()
        .expect("list_eval_runs real deserialize");
    eprintln!("verdryx_live_shapes: {} eval runs", runs.len());

    // For each run, the aggregate must be internally consistent with its own
    // score rows - proving scores + run_summary both deserialize and agree.
    for run in &runs {
        let scores = client
            .scores_for_run(&run.id)
            .expect("scores_for_run real deserialize");
        let summary = client
            .run_summary(&run.id)
            .expect("run_summary real deserialize")
            .expect("summary exists for a listed run");
        assert_eq!(
            summary.case_count as usize,
            scores.len(),
            "run_summary case_count must match the score rows"
        );
        // mean_score is None iff there are no scores, never a fabricated 0.
        assert_eq!(
            summary.mean_score.is_none(),
            scores.is_empty(),
            "mean_score None iff no scores"
        );
        if let Some(mean) = summary.mean_score {
            assert!((0.0..=1.0).contains(&mean), "a quality mean is in [0,1]");
        }
    }

    // Every baseline references a run id that exists (FK integrity round-trips).
    let baselines = client
        .list_baselines()
        .expect("list_baselines real deserialize");
    let run_ids: std::collections::BTreeSet<&str> = runs.iter().map(|r| r.id.as_str()).collect();
    for bl in &baselines {
        assert!(
            run_ids.contains(bl.eval_run_id.as_str()),
            "baseline {} references a missing run {}",
            bl.id,
            bl.eval_run_id
        );
    }
    eprintln!(
        "verdryx_live_shapes: OK ({} runs, {} baselines all deserialized + consistent)",
        runs.len(),
        baselines.len()
    );
}
