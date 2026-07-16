//! Golden conformance tests. Real campaign NDJSON must all pass; a battery of
//! deliberately-broken envelopes must all fail, including the exact defect the Go
//! validator once caught in the wild: a `prev_hash` with 63 hex chars, not 64 (07 §1).

use genaryx_core::Conformer;

fn conformer() -> Conformer {
    Conformer::new().expect("embedded schemas must compile")
}

/// Every non-empty line in a fixture file must be valid.
fn assert_fixture_all_valid(name: &str, body: &str) {
    let c = conformer();
    let mut n = 0;
    for (i, line) in body.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let report = c.check_line(line);
        assert!(
            report.valid,
            "{name} line {}: expected valid, got errors: {:?}\n  line: {line}",
            i + 1,
            report.errors
        );
        n += 1;
    }
    assert!(n > 0, "{name}: fixture had no events");
}

#[test]
fn canonical_examples_all_valid() {
    // 7 events across all sources, spanning v0.1 and v0.2 (07 §1 examples).
    assert_fixture_all_valid("canonical", include_str!("fixtures/canonical.ndjson"));
}

#[test]
fn real_bank_campaign_all_valid() {
    // Real bank-in-a-box campaign output; conforming agent:// ids.
    assert_fixture_all_valid(
        "campaign-bank",
        include_str!("fixtures/campaign-bank.ndjson"),
    );
}

#[test]
fn real_aws_campaign_agent_ids_are_nonconforming() {
    // Real finding (2026-07-16): the aws-comparable-176 benchmark campaign emitted
    // every event with `agent_id: "aws-comparable-agent"` — no `agent://` prefix —
    // via the fail-open emission path (07 §3). The conformer must catch all 12,
    // which is exactly what the Posture "schema conformance" check surfaces.
    let c = conformer();
    let body = include_str!("fixtures/campaign-aws-176.ndjson");
    let mut checked = 0;
    for (i, line) in body.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let report = c.check_line(line);
        assert!(
            !report.valid,
            "aws line {}: expected NON-conforming agent_id",
            i + 1
        );
        assert!(
            report.errors.iter().any(|e| e.contains("agent://")),
            "aws line {}: expected an agent_id pattern error, got {:?}",
            i + 1,
            report.errors
        );
        checked += 1;
    }
    assert_eq!(
        checked, 12,
        "expected all 12 benchmark events to be checked"
    );
}

#[test]
fn schema_version_is_resolved() {
    let c = conformer();
    let v01 = r#"{"schema":"taipanbox.dev/agent-event/v0.1","ts":"2026-07-09T03:12:44.100Z","source":"tokenfuse","type":"budget_exhausted","agent_id":"agent://acme.example/support/bot"}"#;
    let r = c.check_line(v01);
    assert!(r.valid);
    assert_eq!(r.schema_version, Some(genaryx_core::SchemaVersion::V0_1));
}

// ---- invalid battery -------------------------------------------------------

#[test]
fn prev_hash_63_hex_is_rejected() {
    // The real defect: 63 hex chars instead of 64. Must fail the pattern.
    let c = conformer();
    let bad = r#"{"schema":"taipanbox.dev/agent-event/v0.1","ts":"2026-07-09T03:12:44.100Z","source":"tokenfuse","type":"budget_exhausted","agent_id":"agent://acme.example/support/bot","prev_hash":"sha256:2e81d20e76391693864bc8b7c0963b6aa87ef867c36bc80a0678166dcfb3168"}"#;
    let r = c.check_line(bad);
    assert!(!r.valid, "63-hex prev_hash must be rejected");
    assert!(!r.errors.is_empty());
}

#[test]
fn missing_required_agent_id_is_rejected() {
    let c = conformer();
    let bad = r#"{"schema":"taipanbox.dev/agent-event/v0.1","ts":"2026-07-09T03:12:44.100Z","source":"tokenfuse","type":"budget_exhausted"}"#;
    assert!(!c.check_line(bad).valid);
}

#[test]
fn uppercase_agent_id_violates_pattern() {
    let c = conformer();
    let bad = r#"{"schema":"taipanbox.dev/agent-event/v0.1","ts":"2026-07-09T03:12:44.100Z","source":"tokenfuse","type":"budget_exhausted","agent_id":"agent://Acme.Example/Support/Bot"}"#;
    assert!(!c.check_line(bad).valid);
}

#[test]
fn v0_1_source_enum_is_closed() {
    // `wardryx` is NOT in the v0.1 closed enum; under v0.1 this must fail (07 §1).
    let c = conformer();
    let bad = r#"{"schema":"taipanbox.dev/agent-event/v0.1","ts":"2026-07-09T03:25:47.200Z","source":"wardryx","type":"policy_deny","agent_id":"agent://acme.example/eng/ci-fixer"}"#;
    assert!(
        !c.check_line(bad).valid,
        "v0.1 must reject a source outside its closed enum"
    );

    // The same source IS allowed under v0.2 (open source string).
    let ok = r#"{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-07-09T03:25:47.200Z","source":"wardryx","type":"policy_deny","agent_id":"agent://acme.example/eng/ci-fixer"}"#;
    assert!(
        c.check_line(ok).valid,
        "v0.2 must accept an open source string"
    );
}

#[test]
fn unknown_schema_is_rejected() {
    let c = conformer();
    let bad = r#"{"schema":"taipanbox.dev/agent-event/v9.9","ts":"2026-07-09T03:12:44.100Z","source":"tokenfuse","type":"x","agent_id":"agent://acme.example/a/b"}"#;
    let r = c.check_line(bad);
    assert!(!r.valid);
    assert_eq!(r.schema_version, None);
}

#[test]
fn malformed_json_is_rejected_not_panicked() {
    let c = conformer();
    let r = c.check_line("{not json");
    assert!(!r.valid);
    assert!(r.errors[0].contains("malformed json"));
}

#[test]
fn bad_severity_enum_is_rejected() {
    let c = conformer();
    let bad = r#"{"schema":"taipanbox.dev/agent-event/v0.1","ts":"2026-07-09T03:12:44.100Z","source":"tokenfuse","type":"budget_exhausted","agent_id":"agent://acme.example/a/b","severity":"apocalyptic"}"#;
    assert!(!c.check_line(bad).valid);
}
