//! Demo generator tests: volume/shape assertions against the real-campaign
//! targets (08 §2), and the conformance closed loop, every line of every
//! generated file must pass `genaryx_core::Conformer` (07 §1), proving the
//! synthesized campaign is a valid agent-event stream, not just
//! plausible-looking JSON.

use genaryx_core::{Conformer, demo};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const SERVICES: [&str; 6] = [
    "tokenfuse",
    "wardryx",
    "engram",
    "verdryx",
    "mockryx",
    "qryx",
];

static UNIQUE: AtomicU64 = AtomicU64::new(0);

/// A fresh, unlikely-to-collide directory under the OS temp dir. Only
/// `demo::generate`'s own output must be reproducible, not the directory
/// name, so a process-id + atomic-counter combination is enough (no
/// wall-clock read needed here either).
fn unique_temp_dir(label: &str) -> PathBuf {
    let n = UNIQUE.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!(
        "genaryx-demo-test-{}-{label}-{n}",
        std::process::id()
    ))
}

#[test]
fn generates_over_100_events() {
    let dir = unique_temp_dir("volume");
    let total = demo::generate(&dir).expect("generate must succeed");
    assert!(total > 100, "expected > 100 events, got {total}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn writes_all_six_service_files_and_no_idryx() {
    let dir = unique_temp_dir("files");
    demo::generate(&dir).expect("generate must succeed");

    for service in SERVICES {
        let path = dir.join(format!("{service}.ndjson"));
        let meta =
            fs::metadata(&path).unwrap_or_else(|e| panic!("{} must exist: {e}", path.display()));
        assert!(meta.len() > 0, "{} must be non-empty", path.display());
    }

    let idryx_path = dir.join("idryx.ndjson");
    assert!(
        !idryx_path.exists(),
        "idryx must never emit to the bus (07 §2); found {}",
        idryx_path.display()
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn every_generated_line_conforms() {
    let dir = unique_temp_dir("conform");
    demo::generate(&dir).expect("generate must succeed");
    let conformer = Conformer::new().expect("embedded schemas must compile");

    for service in SERVICES {
        let path = dir.join(format!("{service}.ndjson"));
        let body =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for (i, line) in body.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let report = conformer.check_line(line);
            if !report.valid {
                panic!(
                    "{} line {}: expected valid, got errors: {:?}\n  line: {line}",
                    path.display(),
                    i + 1,
                    report.errors
                );
            }
        }
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn shapes_match_real_campaign_bands() {
    let dir = unique_temp_dir("shapes");
    let total = demo::generate(&dir).expect("generate must succeed");
    assert!(total > 100, "expected > 100 events total, got {total}");

    let mut run_ids: HashSet<String> = HashSet::new();
    let mut agent_ids: HashSet<String> = HashSet::new();
    let mut block_events = 0usize;

    for service in SERVICES {
        let path = dir.join(format!("{service}.ndjson"));
        let body =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for line in body.lines().filter(|l| !l.trim().is_empty()) {
            let value: serde_json::Value =
                serde_json::from_str(line).unwrap_or_else(|e| panic!("parse {line}: {e}"));

            if let Some(run_id) = value.get("run_id").and_then(|v| v.as_str()) {
                run_ids.insert(run_id.to_string());
            }
            if let Some(agent_id) = value.get("agent_id").and_then(|v| v.as_str()) {
                agent_ids.insert(agent_id.to_string());
            }

            let source = value.get("source").and_then(|v| v.as_str());
            let event_type = value.get("type").and_then(|v| v.as_str());
            if source == Some("tokenfuse")
                && matches!(
                    event_type,
                    Some("budget_exhausted") | Some("breaker_tripped")
                )
            {
                block_events += 1;
            }
        }
    }

    assert!(
        run_ids.len() >= 40,
        "expected >= 40 distinct run_id, got {}",
        run_ids.len()
    );
    assert!(
        agent_ids.len() >= 20,
        "expected >= 20 distinct agent_id, got {}",
        agent_ids.len()
    );
    assert!(
        block_events >= 10,
        "expected >= 10 tokenfuse block events, got {block_events}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn generation_is_byte_deterministic() {
    let dir_a = unique_temp_dir("det-a");
    let dir_b = unique_temp_dir("det-b");
    demo::generate(&dir_a).expect("generate a");
    demo::generate(&dir_b).expect("generate b");

    for service in SERVICES {
        let a = fs::read(dir_a.join(format!("{service}.ndjson"))).expect("read a");
        let b = fs::read(dir_b.join(format!("{service}.ndjson"))).expect("read b");
        assert_eq!(a, b, "{service}.ndjson must be byte-identical across calls");
    }

    let _ = fs::remove_dir_all(&dir_a);
    let _ = fs::remove_dir_all(&dir_b);
}
