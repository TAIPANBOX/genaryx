//! The envelope's own small contract: every schema string the console
//! accepts maps to its version and back, an unknown one maps to nothing, the
//! severity ladder reads exactly its five words, and a `ConsoleEvent`
//! round-trips through JSON with its version as the canonical schema string.

use genaryx_core::{AgentEvent, ConsoleEvent, Provenance, SchemaVersion, Severity};

#[test]
fn every_accepted_schema_string_maps_to_its_version_and_back() {
    for (s, v) in [
        ("taipanbox.dev/agent-event/v0.1", SchemaVersion::V0_1),
        ("taipanbox.dev/agent-event/v0.2", SchemaVersion::V0_2),
        ("taipanbox.dev/agent-event/v1.0", SchemaVersion::V1_0),
    ] {
        assert_eq!(SchemaVersion::from_schema_str(s), Some(v));
        assert_eq!(v.as_str(), s);
    }
    for unknown in [
        "taipanbox.dev/agent-event/v0.3",
        "taipanbox.dev/agent-event/v1.0 ",
        "TAIPANBOX.DEV/agent-event/v0.1",
        "",
    ] {
        assert_eq!(SchemaVersion::from_schema_str(unknown), None, "{unknown:?}");
    }
}

#[test]
fn severity_reads_exactly_its_five_words_in_ladder_order() {
    let ladder = ["info", "low", "medium", "high", "critical"];
    let parsed: Vec<Severity> = ladder
        .iter()
        .map(|s| Severity::parse(s).expect("a ladder word"))
        .collect();
    assert_eq!(
        parsed,
        [
            Severity::Info,
            Severity::Low,
            Severity::Medium,
            Severity::High,
            Severity::Critical
        ]
    );
    assert!(parsed.windows(2).all(|w| w[0] < w[1]), "ordered by severity");
    for odd in ["High", "warning", "", " low"] {
        assert_eq!(Severity::parse(odd), None, "{odd:?}");
    }
}

fn event(schema: &str, severity: Option<&str>) -> AgentEvent {
    serde_json::from_value(serde_json::json!({
        "schema": schema,
        "ts": "2026-11-01T00:00:00Z",
        "source": "tokenfuse",
        "type": "budget_exhausted",
        "agent_id": "agent://acme.example/bot",
        "severity": severity,
        "vendor_field": {"kept": true},
    }))
    .expect("a tolerant envelope")
}

#[test]
fn an_envelope_keeps_unknown_keys_and_reads_severity_and_version_tolerantly() {
    let e = event("taipanbox.dev/agent-event/v0.2", Some("high"));
    assert_eq!(e.severity_typed(), Some(Severity::High));
    assert_eq!(e.schema_version(), Some(SchemaVersion::V0_2));
    assert_eq!(e.extra["vendor_field"]["kept"], serde_json::json!(true));

    let odd = event("taipanbox.dev/agent-event/v9", Some("apocalyptic"));
    assert_eq!(odd.severity_typed(), None, "an unknown word is not a severity");
    assert_eq!(odd.schema_version(), None);

    let quiet = event("taipanbox.dev/agent-event/v0.1", None);
    assert_eq!(quiet.severity_typed(), None);
    let line = serde_json::to_string(&quiet).expect("serialize");
    assert!(
        !line.contains("severity") && !line.contains("run_id") && !line.contains("on_behalf_of"),
        "absent optional keys are not written back: {line}"
    );
    assert!(line.contains("\"type\":\"budget_exhausted\""), "{line}");
}

#[test]
fn a_console_event_round_trips_with_its_version_as_the_schema_string() {
    let ce = ConsoleEvent {
        event: event("taipanbox.dev/agent-event/v1.0", Some("low")),
        provenance: Provenance {
            env: "local".into(),
            connector: "filetail:tokenfuse".into(),
            file: Some("/bus/tokenfuse.ndjson".into()),
            offset: Some(42),
            endpoint: None,
            received_ts: "2026-11-01T00:00:01Z".into(),
        },
        raw: "{}".into(),
        schema_version: SchemaVersion::V1_0,
    };
    let v = serde_json::to_value(&ce).expect("serialize");
    assert_eq!(v["schema_version"], "taipanbox.dev/agent-event/v1.0");
    assert!(v["provenance"].get("endpoint").is_none());
    let back: ConsoleEvent = serde_json::from_value(v.clone()).expect("round trip");
    assert_eq!(back.schema_version, SchemaVersion::V1_0);
    assert_eq!(back.provenance.offset, Some(42));

    let mut wrong = v;
    wrong["schema_version"] = serde_json::json!("taipanbox.dev/agent-event/v0.3");
    let err = serde_json::from_value::<ConsoleEvent>(wrong)
        .err()
        .expect("an unknown version must not deserialize");
    assert!(
        err.to_string().contains("unknown schema version"),
        "{err}"
    );
}
