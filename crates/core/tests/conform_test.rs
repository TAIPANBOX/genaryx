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

/// SPEC 5.1 caps the delegation chain at 32 entries and both canonical schemas
/// carry that as `maxItems`. Both vendored copies here had lost it, so this
/// console accepted a chain of any depth while believing it validated one, and
/// nothing said so: the copies are compiled in with `include_str!`, so the only
/// thing between them and the canonical file is a byte comparison living in
/// another repository.
///
/// Both directions are asserted on purpose. A bound that refuses 33 and also
/// refuses 32 is a different bug wearing the same green tick.
#[test]
fn a_delegation_chain_past_the_spec_depth_is_rejected() {
    let c = conformer();
    let chain = |n: usize| -> String {
        (0..n)
            .map(|i| format!("\"agent://acme.example/a/{i}\""))
            .collect::<Vec<_>>()
            .join(",")
    };
    let event = |schema: &str, entries: &str| -> String {
        format!(
            r#"{{"schema":"taipanbox.dev/agent-event/{schema}","ts":"2026-07-09T03:12:44.100Z","source":"tokenfuse","type":"budget_exhausted","agent_id":"agent://acme.example/a/b","on_behalf_of":[{entries}]}}"#
        )
    };

    for schema in ["v0.1", "v0.2"] {
        assert!(
            c.check_line(&event(schema, &chain(32))).valid,
            "{schema}: a chain of exactly 32 is legal under SPEC 5.1 and must stay legal"
        );
        assert!(
            !c.check_line(&event(schema, &chain(33))).valid,
            "{schema}: a chain of 33 exceeds SPEC 5.1 and must be refused"
        );
    }
}

/// SPEC 5.2 added `delegation_proof` to the v0.2 envelope: the four fields that
/// record an RFC 8693 token proved the `on_behalf_of` chain, without carrying
/// the token itself. The vendored copy is what decides whether a line on this
/// bus conforms, and until it was re-vendored the envelope's
/// `additionalProperties: true` waved the whole object through unread. A proof
/// with no key thumbprint, an `exp` that was a string, or a stray field beside
/// the four all validated cleanly, so the console could have shown a delegation
/// as proved over an object that proves nothing.
///
/// Both directions on purpose, as with the depth cap above. A schema that
/// refuses every proof is a different bug wearing the same green tick, so the
/// well-formed one is asserted first and each way of being malformed after it.
#[test]
fn a_v0_2_delegation_proof_is_checked_rather_than_waved_through() {
    let c = conformer();
    let event = |proof: &str| -> String {
        format!(
            r#"{{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-07-09T03:12:44.100Z","source":"wardryx","type":"policy_allow","agent_id":"agent://acme.example/a/b","on_behalf_of":["user://acme.example/alice"],"delegation_proof":{proof}}}"#
        )
    };
    let good = r#"{"jti":"tok-1","jkt":"NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs","iss":"https://idryx.acme.example","exp":1786000000}"#;

    assert!(
        c.check_line(&event(good)).valid,
        "a well-formed SPEC 5.2 proof must stay valid"
    );

    // An event with no proof at all is still legal: SPEC 5.2 is optional, and
    // absent means NOT proven rather than proven elsewhere.
    let bare = r#"{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-07-09T03:12:44.100Z","source":"wardryx","type":"policy_allow","agent_id":"agent://acme.example/a/b"}"#;
    assert!(c.check_line(bare).valid, "the field is optional");

    for (why, proof) in [
        (
            "no jkt: nothing says who was holding the token",
            r#"{"jti":"tok-1","iss":"https://idryx.acme.example","exp":1786000000}"#,
        ),
        (
            "no jti: no auditor can find it in the issuer's record",
            r#"{"jkt":"NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs","iss":"https://idryx.acme.example","exp":1786000000}"#,
        ),
        (
            "no exp: SPEC 2 says the chain carries no freshness, so the proof must",
            r#"{"jti":"tok-1","jkt":"NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs","iss":"https://idryx.acme.example"}"#,
        ),
        (
            "exp as a string, which sorts and compares as text, not as a time",
            r#"{"jti":"tok-1","jkt":"NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs","iss":"https://idryx.acme.example","exp":"1786000000"}"#,
        ),
        (
            "an empty jkt, which is a thumbprint of nothing",
            r#"{"jti":"tok-1","jkt":"","iss":"https://idryx.acme.example","exp":1786000000}"#,
        ),
        (
            "the token itself smuggled in beside the four fields",
            r#"{"jti":"tok-1","jkt":"NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs","iss":"https://idryx.acme.example","exp":1786000000,"token":"eyJhbGciOiJFUzI1NiJ9.e30.sig"}"#,
        ),
    ] {
        let r = c.check_line(&event(proof));
        assert!(!r.valid, "must be refused ({why}): {proof}");
    }
}

/// What this console does with a proof once it conforms, stated as a test
/// rather than as prose in a PR. `AgentEvent` has no `delegation_proof` field:
/// the envelope struct is deliberately tolerant, so an unknown top-level key
/// lands in `extra` and comes back out byte-for-byte on the way to a panel.
/// So the console ACCEPTS and PRESERVES a delegation proof; nothing here mints
/// one, and re-vendoring a schema does not give it the ability to.
///
/// This one was green BEFORE the re-vendor as well, and that is the point: the
/// tolerance is pre-existing, not new. It is pinned here so a later
/// `deny_unknown_fields` cannot quietly drop a proof on the floor.
#[test]
fn a_delegation_proof_survives_the_envelope_struct_verbatim() {
    let c = conformer();
    let line = r#"{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-07-09T03:12:44.100Z","source":"wardryx","type":"policy_allow","agent_id":"agent://acme.example/a/b","on_behalf_of":["user://acme.example/alice"],"delegation_proof":{"jti":"tok-1","jkt":"NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs","iss":"https://idryx.acme.example","exp":1786000000}}"#;
    let event = c.parse_valid(line).expect("a conforming line must decode");

    let proof = event
        .extra
        .get("delegation_proof")
        .expect("an unknown top-level key is preserved in `extra`");
    assert_eq!(proof["jti"], "tok-1");
    assert_eq!(proof["exp"], 1_786_000_000_i64);

    let round_tripped = serde_json::to_value(&event).expect("re-serializes");
    assert_eq!(
        round_tripped["delegation_proof"], *proof,
        "the proof must reach a panel exactly as it arrived"
    );
}
