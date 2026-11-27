//! The Bus Explorer's row shape: both conversions into it, and the
//! fail-closed mock timeline `recent_events` serves when the store is
//! unavailable.
//!
//! The mock is labelled as such everywhere it surfaces, but its rows still
//! have to be internally consistent: a `raw` line that disagrees with the
//! structured fields beside it, or a schema version the source never emits,
//! would teach an operator a shape the real bus does not have.

use chrono::{DateTime, Utc};
use genaryx_api::events::{UiEvent, mock_events};
use genaryx_core::event::{AgentEvent, ConsoleEvent, Provenance, SchemaVersion};
use genaryx_core::store::StoredEvent;
use serde_json::{Map, Value, json};

const V01: &str = "taipanbox.dev/agent-event/v0.1";
const V02: &str = "taipanbox.dev/agent-event/v0.2";

/// The number of rows in the mock timeline.
const MOCK_ROWS: usize = 41;

fn stored_row() -> StoredEvent {
    StoredEvent {
        id: 42,
        env: "single".to_string(),
        ts: "2026-10-20T09:00:00.000Z".to_string(),
        source: "wardryx".to_string(),
        type_: "policy_deny".to_string(),
        agent_id: "agent://acme.example/agents/a".to_string(),
        run_id: Some("run-7".to_string()),
        severity: Some("high".to_string()),
        schema: V02.to_string(),
        on_behalf_of: vec!["user://acme.example/operator".to_string()],
        data: Some(json!({"policy": "deny-all"})),
        prev_hash: Some(format!("sha256:{}", "ab".repeat(32))),
        raw: "{\"raw\":true}".to_string(),
        file: Some("/var/lib/events/wardryx.ndjson".to_string()),
        off: Some(4096),
    }
}

fn console_event(event_type: &str) -> ConsoleEvent {
    ConsoleEvent {
        event: AgentEvent {
            schema: V02.to_string(),
            ts: "2026-10-20T09:00:00.000Z".to_string(),
            source: "verdryx".to_string(),
            event_type: event_type.to_string(),
            agent_id: "agent://acme.example/agents/b".to_string(),
            severity: Some("info".to_string()),
            run_id: Some("run-1".to_string()),
            on_behalf_of: vec!["agent://acme.example/agents/root".to_string()],
            data: Some(json!({"eval_suite": "qa"})),
            prev_hash: None,
            extra: Map::new(),
        },
        provenance: Provenance {
            env: "demo".to_string(),
            connector: "filetail:verdryx".to_string(),
            file: Some("/tmp/events/verdryx.ndjson".to_string()),
            offset: Some(128),
            endpoint: None,
            received_ts: "2026-10-20T09:00:01Z".to_string(),
        },
        raw: "{\"line\":1}".to_string(),
        schema_version: SchemaVersion::V0_2,
    }
}

fn run_of(e: &UiEvent) -> u32 {
    e.run_id
        .as_deref()
        .and_then(|r| r.strip_prefix("demo-run-"))
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("a mock run id is demo-run-NNN: {:?}", e.run_id))
}

#[test]
fn a_stored_row_reaches_the_ui_field_for_field() {
    let stored = stored_row();
    let ui = UiEvent::from(stored.clone());
    assert_eq!(ui.id, stored.id);
    assert_eq!(ui.env, stored.env);
    assert_eq!(ui.ts, stored.ts);
    assert_eq!(ui.source, stored.source);
    assert_eq!(ui.event_type, stored.type_);
    assert_eq!(ui.agent_id, stored.agent_id);
    assert_eq!(ui.run_id, stored.run_id);
    assert_eq!(ui.severity, stored.severity);
    assert_eq!(ui.schema, stored.schema);
    assert_eq!(ui.on_behalf_of, stored.on_behalf_of);
    assert_eq!(ui.data, stored.data);
    assert_eq!(ui.prev_hash, stored.prev_hash);
    assert_eq!(ui.raw, stored.raw);
    assert_eq!(ui.file, stored.file);
    assert_eq!(ui.off, stored.off);

    // On the wire the type is `type`, not Rust's escaped field name.
    let wire = serde_json::to_value(&ui).expect("a UiEvent serializes");
    assert_eq!(wire["type"], "policy_deny");
    assert!(wire.get("event_type").is_none(), "{wire}");
}

#[test]
fn a_live_event_carries_its_provenance_and_a_negative_id_that_never_repeats() {
    let first = UiEvent::from(console_event("quality_score"));
    let second = UiEvent::from(console_event("quality_drift"));

    assert!(
        first.id < 0,
        "a live row has no store id yet, so its id must not collide with one: {}",
        first.id
    );
    assert!(
        second.id < first.id,
        "synthetic ids strictly decrease: {} then {}",
        first.id,
        second.id
    );

    assert_eq!(first.env, "demo", "env comes from the provenance");
    assert_eq!(first.file.as_deref(), Some("/tmp/events/verdryx.ndjson"));
    assert_eq!(first.off, Some(128));
    assert_eq!(first.raw, "{\"line\":1}");
    assert_eq!(first.source, "verdryx");
    assert_eq!(first.event_type, "quality_score");
    assert_eq!(second.event_type, "quality_drift");
    assert_eq!(first.agent_id, "agent://acme.example/agents/b");
    assert_eq!(first.run_id.as_deref(), Some("run-1"));
    assert_eq!(first.severity.as_deref(), Some("info"));
    assert_eq!(first.schema, V02);
    assert_eq!(first.on_behalf_of, vec!["agent://acme.example/agents/root"]);
    assert_eq!(first.data, Some(json!({"eval_suite": "qa"})));
    assert_eq!(first.prev_hash, None);
}

#[test]
fn the_mock_timeline_is_newest_first_on_a_45_second_cadence_ending_near_now() {
    let all = mock_events(1_000);
    assert_eq!(all.len(), MOCK_ROWS);

    let ids: Vec<i64> = all.iter().map(|e| e.id).collect();
    let expected: Vec<i64> = (1..=MOCK_ROWS as i64).rev().collect();
    assert_eq!(ids, expected, "newest first by id, like Store::recent_events");

    let times: Vec<DateTime<Utc>> = all
        .iter()
        .map(|e| {
            DateTime::parse_from_rfc3339(&e.ts)
                .unwrap_or_else(|err| panic!("{:?} is not RFC 3339: {err}", e.ts))
                .with_timezone(&Utc)
        })
        .collect();
    for pair in times.windows(2) {
        assert_eq!(
            (pair[0] - pair[1]).num_seconds(),
            45,
            "each row is 45 s older than the one above it"
        );
    }
    let newest_age = (Utc::now() - times[0]).num_milliseconds();
    assert!(
        (45_000..120_000).contains(&newest_age),
        "the newest row is anchored about 45 s before now, got {newest_age} ms"
    );
}

#[test]
fn the_mock_timeline_honours_its_limit() {
    assert!(mock_events(0).is_empty());
    let five: Vec<i64> = mock_events(5).iter().map(|e| e.id).collect();
    assert_eq!(five, vec![41, 40, 39, 38, 37]);
    assert_eq!(mock_events(MOCK_ROWS).len(), MOCK_ROWS);
    assert_eq!(mock_events(MOCK_ROWS - 1).len(), MOCK_ROWS - 1);
}

#[test]
fn every_mock_row_carries_the_schema_version_its_source_emits() {
    let all = mock_events(MOCK_ROWS);
    let (mut v1, mut v2) = (0, 0);
    for e in &all {
        let open_source = matches!(e.source.as_str(), "wardryx" | "verdryx" | "mockryx");
        let want = if open_source || e.event_type == "identity_mismatch" {
            v2 += 1;
            V02
        } else {
            v1 += 1;
            V01
        };
        assert_eq!(e.schema, want, "{} {}", e.source, e.event_type);
    }
    assert!(v1 > 0 && v2 > 0, "both versions are represented");
}

#[test]
fn only_the_delegated_rows_carry_an_on_behalf_of_chain() {
    let all = mock_events(MOCK_ROWS);
    let delegated: Vec<&UiEvent> = all.iter().filter(|e| !e.on_behalf_of.is_empty()).collect();
    assert_eq!(delegated.len(), 8, "three runs carry the demo delegation chain");
    for e in &delegated {
        assert_eq!(e.on_behalf_of.len(), 2, "{e:?}");
        assert!(e.on_behalf_of[0].starts_with("user://taipanbox.dev/"));
        assert_eq!(e.on_behalf_of[1], "agent://taipanbox.dev/demo/orchestrator");
        assert!(
            matches!(run_of(e), 3 | 11 | 20),
            "only runs 3, 11 and 20 are delegated: {e:?}"
        );
    }
}

#[test]
fn every_mock_raw_line_parses_and_agrees_with_the_fields_beside_it() {
    for e in mock_events(MOCK_ROWS) {
        let raw: Value = serde_json::from_str(&e.raw)
            .unwrap_or_else(|err| panic!("raw must be one JSON object: {err}: {}", e.raw));
        assert_eq!(raw["schema"], e.schema.as_str());
        assert_eq!(raw["ts"], e.ts.as_str());
        assert_eq!(raw["source"], e.source.as_str());
        assert_eq!(raw["type"], e.event_type.as_str());
        assert_eq!(raw["agent_id"], e.agent_id.as_str());
        assert_eq!(raw["severity"], e.severity.as_deref().expect("a severity"));
        assert_eq!(raw["run_id"], e.run_id.as_deref().expect("a run id"));
        assert_eq!(&raw["data"], e.data.as_ref().expect("data"));
        if e.on_behalf_of.is_empty() {
            assert!(
                raw.get("on_behalf_of").is_none(),
                "an empty chain is omitted, not written as []: {}",
                e.raw
            );
        } else {
            assert_eq!(raw["on_behalf_of"], json!(e.on_behalf_of));
        }
    }
}

#[test]
fn every_mock_row_has_honest_provenance_and_no_chain() {
    for e in mock_events(MOCK_ROWS) {
        assert_eq!(e.env, "local");
        assert_eq!(e.prev_hash, None, "fabricated rows are never chained");
        assert_eq!(
            e.file.as_deref(),
            Some(format!("~/.taipan/events/{}.ndjson", e.source).as_str())
        );
        assert_eq!(e.off, Some(((e.id - 1) as u64) * 128));
        assert!(e.agent_id.starts_with("agent://taipanbox.dev/demo/"));
        assert!(matches!(
            e.severity.as_deref(),
            Some("info" | "medium" | "high" | "critical")
        ));
    }
}

#[test]
fn every_mock_event_type_gets_its_own_data_shape() {
    const TOPICS: [&str; 6] = [
        "customer_refund_policy",
        "kyc_verification_steps",
        "fraud_hold_criteria",
        "sla_response_times",
        "chargeback_procedure",
        "aml_screening_rules",
    ];
    const EVAL_SUITES: [&str; 5] = [
        "refund-policy-qa",
        "kyc-accuracy-qa",
        "fraud-triage-qa",
        "sla-compliance-qa",
        "aml-screening-qa",
    ];
    const SCENARIOS: [&str; 5] = [
        "prod-deploy-rehearsal",
        "budget-exhaustion-drill",
        "policy-bypass-drill",
        "credential-leak-drill",
        "runaway-agent-drill",
    ];
    let keys: [(&str, &[&str]); 19] = [
        ("identity_mismatch", &["key_id", "agent_id", "reason"]),
        (
            "budget_exhausted",
            &["budget_usd", "spent_usd", "reason", "policy_id"],
        ),
        (
            "breaker_tripped",
            &["budget_usd", "spent_usd", "reason", "policy_id"],
        ),
        (
            "spend_spike",
            &["window_s", "spend_usd", "baseline_usd", "multiplier"],
        ),
        ("sustained_loop", &["calls", "window_s", "pattern"]),
        ("fanout_explosion", &["child_agents", "depth", "budget_usd"]),
        ("policy_allow", &["policy", "reason"]),
        ("policy_deny", &["policy", "reason"]),
        ("approval_requested", &["policy", "reason"]),
        ("approval_granted", &["policy", "granted_by"]),
        ("memory_written", &["memory_id", "topic"]),
        (
            "contradiction_found",
            &["memory_id", "conflicting_memory_id", "topic"],
        ),
        ("quality_score", &["eval_suite", "current_score"]),
        (
            "quality_drift",
            &["eval_suite", "baseline_score", "current_score", "delta"],
        ),
        ("sim_run", &["scenario", "status"]),
        ("sim_finding", &["scenario", "finding"]),
        (
            "blast_radius_measured",
            &["scenario", "blast_radius_score", "affected_resources"],
        ),
        ("crypto_finding", &["algorithm", "risk", "recommended"]),
        ("evidence_signed", &["evidence_id", "algorithm", "subject"]),
    ];

    for e in mock_events(MOCK_ROWS) {
        let data = e.data.as_ref().expect("every mock row carries data");
        let object = data.as_object().expect("data is an object");
        let (_, want) = keys
            .iter()
            .find(|(t, _)| *t == e.event_type)
            .unwrap_or_else(|| panic!("an event type with no data shape: {}", e.event_type));
        let mut got: Vec<&str> = object.keys().map(String::as_str).collect();
        let mut expected: Vec<&str> = want.to_vec();
        got.sort_unstable();
        expected.sort_unstable();
        assert_eq!(got, expected, "{} carries exactly its own keys", e.event_type);

        let run = run_of(&e) as usize;
        match e.event_type.as_str() {
            "memory_written" => {
                assert_eq!(data["memory_id"], format!("mem-{:04}", 3000 + run));
                assert_eq!(data["topic"], TOPICS[run % TOPICS.len()]);
            }
            "contradiction_found" => {
                assert_eq!(data["memory_id"], format!("mem-{:04}", 3000 + run));
                assert_eq!(
                    data["conflicting_memory_id"],
                    format!("mem-{:04}", 2000 + run)
                );
                assert_eq!(data["topic"], TOPICS[run % TOPICS.len()]);
            }
            "quality_score" | "quality_drift" => {
                assert_eq!(data["eval_suite"], EVAL_SUITES[run % EVAL_SUITES.len()]);
            }
            "sim_run" | "sim_finding" | "blast_radius_measured" => {
                assert_eq!(data["scenario"], SCENARIOS[run % SCENARIOS.len()]);
            }
            "evidence_signed" => {
                assert_eq!(data["evidence_id"], format!("ev-{:05}", 55_000 + run));
                assert_eq!(data["subject"], e.agent_id.as_str());
            }
            "identity_mismatch" => {
                // The envelope names the agent the key CLAIMED; the key that
                // called is in `data.key_id`.
                assert_eq!(data["agent_id"], e.agent_id.as_str());
                assert_eq!(data["key_id"], "forge-imposter");
            }
            _ => {}
        }
    }
}
