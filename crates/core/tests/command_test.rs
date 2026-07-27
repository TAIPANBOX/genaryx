//! CommandBroker tests: `command::record` journals a `commands_journal` row
//! and appends a conforming `console_command` line to the console events
//! file, for both a kill and a budget-change outcome (06 §2). The emitted
//! line is checked against the real `Conformer`, not just parsed as JSON, so
//! the "kill -> console_command appears on the bus" loop is closed the same
//! way `demo_test.rs` closes it for the demo generator.

use genaryx_core::store::Store;
use genaryx_core::{CommandRecord, Conformer, SchemaVersion, console_command_line, record};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// A fresh, unlikely-to-collide console-events NDJSON path under the OS temp
/// dir (process id + atomic counter, mirroring `ingest_test.rs`'s helper).
/// Deliberately does NOT pre-create the parent directory: `record`'s own
/// `create_dir_all` is one of the things this suite exercises.
fn unique_console_path(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!(
            "genaryx-command-test-{tag}-{}-{n}",
            std::process::id()
        ))
        .join("console.ndjson")
}

fn kill_record(operator: &str) -> CommandRecord {
    CommandRecord {
        operator: operator.to_string(),
        env: "test".to_string(),
        action: "console.kill_run".to_string(),
        target: "run-42".to_string(),
        params: json!({}),
        decision: "allow".to_string(),
        sig_alg: "es256".to_string(),
        sig_fpr: "secure-enclave".to_string(),
        http_status: 200,
        verify_result: "killed:true".to_string(),
    }
}

fn budget_record(operator: &str) -> CommandRecord {
    CommandRecord {
        operator: operator.to_string(),
        env: "test".to_string(),
        action: "console.set_budget".to_string(),
        target: "run-42".to_string(),
        // A break-glass override must carry a reason (Phase-2 wave 3B: the
        // broker's `require_break_glass_reason` refuses one without it).
        params: json!({"budget_usd": 12.5, "reason": "test operator override"}),
        decision: "break_glass".to_string(),
        sig_alg: "es256".to_string(),
        sig_fpr: "software-signed".to_string(),
        http_status: 200,
        verify_result: "budget_micros:12500000".to_string(),
    }
}

#[test]
fn record_journals_a_row_and_appends_a_conforming_line() {
    let store = Store::open_in_memory().expect("open in-memory store");
    let path = unique_console_path("basic");
    let rec = kill_record("user://acme.example/alice");

    assert_eq!(store.commands_journal_count().expect("count"), 0);

    record(&store, &path, "acme.example", "Console-Host.local", &rec).expect("record");

    assert_eq!(store.commands_journal_count().expect("count"), 1);

    let body = std::fs::read_to_string(&path).expect("read console events file");
    let lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1);

    let conformer = Conformer::new().expect("embedded schemas must compile");
    let report = conformer.check_line(lines[0]);
    assert!(
        report.valid,
        "line must conform: {:?}\n  line: {}",
        report.errors, lines[0]
    );
    assert_eq!(report.schema_version, Some(SchemaVersion::V0_2));

    let value: serde_json::Value = serde_json::from_str(lines[0]).expect("parse emitted line");
    assert_eq!(
        value.get("source").and_then(|v| v.as_str()),
        Some("console")
    );
    assert_eq!(
        value.get("type").and_then(|v| v.as_str()),
        Some("console_command")
    );
    // Host is lowercased safely: mixed-case + a literal dot must still land
    // inside a conforming agent_id.
    assert_eq!(
        value.get("agent_id").and_then(|v| v.as_str()),
        Some("agent://acme.example/console/console-host.local")
    );

    let _ = std::fs::remove_dir_all(path.parent().expect("path has a parent"));
}

#[test]
fn kill_and_budget_records_both_conform_and_carry_operator() {
    let conformer = Conformer::new().expect("embedded schemas must compile");
    let ts = "2026-07-16T12:00:00.000Z";
    let operator = "user://acme.example/alice";

    for rec in [kill_record(operator), budget_record(operator)] {
        let line = console_command_line("acme.example", "console-host", &rec, ts, None)
            .expect("console_command_line");

        let report = conformer.check_line(&line);
        assert!(
            report.valid,
            "{} must conform: {:?}\n  line: {line}",
            rec.action, report.errors
        );
        assert_eq!(report.schema_version, Some(SchemaVersion::V0_2));

        let value: serde_json::Value = serde_json::from_str(&line).expect("parse line");
        let on_behalf_of = value
            .get("on_behalf_of")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("on_behalf_of present for {}", rec.action));
        assert_eq!(
            on_behalf_of,
            &vec![serde_json::Value::String(operator.to_string())]
        );

        // `data` carries exactly the 7 outcome fields; `params` (the budget
        // amount, for the budget record) is journaled but not re-emitted.
        let data = value.get("data").expect("data present");
        assert_eq!(
            data.get("action").and_then(|v| v.as_str()),
            Some(rec.action.as_str())
        );
        assert_eq!(
            data.get("target").and_then(|v| v.as_str()),
            Some(rec.target.as_str())
        );
        assert_eq!(
            data.get("decision").and_then(|v| v.as_str()),
            Some(rec.decision.as_str())
        );
        assert_eq!(
            data.get("sig_alg").and_then(|v| v.as_str()),
            Some(rec.sig_alg.as_str())
        );
        assert_eq!(
            data.get("sig_fpr").and_then(|v| v.as_str()),
            Some(rec.sig_fpr.as_str())
        );
        assert_eq!(
            data.get("http_status").and_then(|v| v.as_u64()),
            Some(u64::from(rec.http_status))
        );
        assert_eq!(
            data.get("verify_result").and_then(|v| v.as_str()),
            Some(rec.verify_result.as_str())
        );
        assert!(
            data.get("params").is_none(),
            "params must not leak into the bus event's data"
        );
    }
}

#[test]
fn appending_twice_yields_two_lines() {
    let store = Store::open_in_memory().expect("open in-memory store");
    let path = unique_console_path("append-twice");
    let operator = "agent://acme.example/console/orchestrator";

    record(
        &store,
        &path,
        "acme.example",
        "host-a",
        &kill_record(operator),
    )
    .expect("first record");
    record(
        &store,
        &path,
        "acme.example",
        "host-a",
        &budget_record(operator),
    )
    .expect("second record");

    assert_eq!(store.commands_journal_count().expect("count"), 2);

    let body = std::fs::read_to_string(&path).expect("read console events file");
    let lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 2, "append must not truncate the previous line");

    let conformer = Conformer::new().expect("embedded schemas must compile");
    for line in &lines {
        let report = conformer.check_line(line);
        assert!(
            report.valid,
            "line must conform: {:?}\n  line: {line}",
            report.errors
        );
    }

    let first: serde_json::Value = serde_json::from_str(lines[0]).expect("parse first line");
    let second: serde_json::Value = serde_json::from_str(lines[1]).expect("parse second line");
    assert_eq!(
        first
            .get("data")
            .and_then(|d| d.get("action"))
            .and_then(|v| v.as_str()),
        Some("console.kill_run")
    );
    assert_eq!(
        second
            .get("data")
            .and_then(|d| d.get("action"))
            .and_then(|v| v.as_str()),
        Some("console.set_budget")
    );

    let _ = std::fs::remove_dir_all(path.parent().expect("path has a parent"));
}

#[test]
fn operator_that_does_not_match_principal_pattern_is_omitted() {
    // No `agent://` or `user://` prefix: must be left out of `on_behalf_of`
    // rather than emitted and failing conformance.
    let rec = kill_record("alice");
    let line = console_command_line(
        "acme.example",
        "host",
        &rec,
        "2026-07-16T12:00:00.000Z",
        None,
    )
    .expect("console_command_line");

    let value: serde_json::Value = serde_json::from_str(&line).expect("parse line");
    assert!(
        value.get("on_behalf_of").is_none(),
        "non-conforming operator must be omitted, not emitted"
    );

    let conformer = Conformer::new().expect("embedded schemas must compile");
    let report = conformer.check_line(&line);
    assert!(
        report.valid,
        "line without on_behalf_of must still conform: {:?}",
        report.errors
    );
}

// --- SPEC 6.5 chain ---------------------------------------------------------

/// A console_command must join the chain the products write into the same
/// file, not sit beside it unlinked. That gap mattered more than any other:
/// kill, budget and access-granting actions are precisely the events an
/// auditor needs to trust, and an unchained line can be removed or altered
/// without breaking the chain around it.
#[test]
fn a_console_command_chains_onto_whatever_was_written_before_it() {
    let dir = std::env::temp_dir().join(format!(
        "genaryx-chain-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let events = dir.join("tokenfuse.ndjson");

    // A product's event, exactly as one would already be sitting in the file.
    let existing = r#"{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-07-16T11:59:00.000Z","source":"wardryx","type":"policy_deny","agent_id":"agent://acme.example/finance/bot"}"#;
    std::fs::write(&events, format!("{existing}\n")).unwrap();

    let store = genaryx_core::store::Store::open(&dir.join("console.sqlite")).unwrap();
    let rec = genaryx_core::command::CommandRecord {
        operator: "user://acme.example/alice".to_string(),
        env: "local".to_string(),
        action: "console.issue_wg_peer".to_string(),
        target: "peerkey".to_string(),
        params: serde_json::json!({}),
        decision: "allow".to_string(),
        sig_alg: "webauthn-es256".to_string(),
        sig_fpr: "cred-id".to_string(),
        http_status: 200,
        verify_result: "issued:10.9.0.2".to_string(),
    };
    genaryx_core::command::record(&store, &events, "acme.example", "host", &rec).unwrap();

    let text = std::fs::read_to_string(&events).unwrap();
    let written: serde_json::Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    let expected = genaryx_core::command::chain_hash_of_line(existing).unwrap();
    assert_eq!(
        written.get("prev_hash").and_then(|v| v.as_str()),
        Some(expected.as_str()),
        "the console line must carry the hash of the event before it"
    );

    // And the hash itself must be shaped as the spec requires, or the Go
    // conformance checker rejects the whole file.
    let hex = expected.strip_prefix("sha256:").expect("sha256: prefix");
    assert_eq!(hex.len(), 64, "a prev_hash is 64 hex characters, never 63");
    assert!(
        hex.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// The hash is taken over the event WITHOUT its own prev_hash (SPEC 6.5), so
/// two lines that differ only by their chain link hash identically. Getting
/// this wrong makes every chain self-inconsistent from the second event on.
#[test]
fn the_chain_hash_ignores_the_events_own_prev_hash() {
    let bare = r#"{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-07-16T12:00:00.000Z","source":"console","type":"console_command","agent_id":"agent://acme.example/console/host"}"#;
    let linked = r#"{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-07-16T12:00:00.000Z","source":"console","type":"console_command","agent_id":"agent://acme.example/console/host","prev_hash":"sha256:0000000000000000000000000000000000000000000000000000000000000000"}"#;
    assert_eq!(
        genaryx_core::command::chain_hash_of_line(bare),
        genaryx_core::command::chain_hash_of_line(linked),
    );
}

/// A head event carries no prev_hash at all - not an empty string, not null.
#[test]
fn the_first_event_in_an_empty_file_carries_no_link() {
    let line = genaryx_core::command::console_command_line(
        "acme.example",
        "host",
        &genaryx_core::command::CommandRecord {
            operator: "user://acme.example/alice".to_string(),
            env: "local".to_string(),
            action: "console.kill_run".to_string(),
            target: "run-1".to_string(),
            params: serde_json::json!({}),
            decision: "allow".to_string(),
            sig_alg: "es256".to_string(),
            sig_fpr: "fpr".to_string(),
            http_status: 200,
            verify_result: "killed:true".to_string(),
        },
        "2026-07-16T12:00:00.000Z",
        None,
    )
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert!(
        v.get("prev_hash").is_none(),
        "a head event must omit the field entirely"
    );
}
