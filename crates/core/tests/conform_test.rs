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

// ---- the contract at 1.0 ---------------------------------------------------

/// agent-passport 1.0 (2026-09-12). SPEC 6.4.1: a consumer MUST accept event
/// v0.1, v0.2 and v1.0. v1.0 is v0.2's shape with the version string changed
/// and one widening (the claimed subject, below), so every line a producer
/// writes today is a v1.0 line once its version string moves. This is that
/// move made on the spec's own example line, and it must RESOLVE to the new
/// version rather than merely pass: a console that accepted the line and filed
/// it under nothing would show it and could not say what it was.
#[test]
fn a_v1_0_event_is_accepted_and_resolved() {
    let c = conformer();
    let line = r#"{"schema":"taipanbox.dev/agent-event/v1.0","ts":"2026-09-12T16:00:00.000Z","source":"tokenfuse","type":"budget_exhausted","severity":"critical","agent_id":"agent://acme-bank.example/support/tier1-bot","run_id":"run-9001","on_behalf_of":["user://acme-bank.example/j.doe"],"data":{"budget_usd":2.0,"spent_usd":2.0,"action":"blocked_402"}}"#;
    let r = c.check_line(line);
    assert!(r.valid, "a v1.0 line must be accepted: {:?}", r.errors);
    assert_eq!(r.schema_version, Some(genaryx_core::SchemaVersion::V1_0));

    let event = c
        .parse_valid(line)
        .expect("a conforming v1.0 line must decode");
    assert_eq!(
        event.schema_version(),
        Some(genaryx_core::SchemaVersion::V1_0)
    );
    assert_eq!(event.schema, genaryx_core::SchemaVersion::SCHEMA_V1_0);
}

/// The one widening, and what this console does with it. Under v1.0 `agent_id`
/// may be `claimed:agent://...` (SPEC 3.3): an identity the producer read from
/// the process's own environment and could not attest. The console has no
/// model for a claim, so SPEC 6.4.1 leaves it one duty: refuse the line and
/// count it, never show it as an agent. The refusal is asserted here under its
/// one fixed reason; the count is `ingest_test.rs`'s
/// `claimed_subjects_are_quarantined_under_one_reason_and_counted`.
///
/// The same subject under v0.2 is refused too, by the pattern, and that is
/// asserted beside it on purpose. SPEC 6.4.1's sentence is that the refusal
/// moved from the version to the subject: a v1.0 consumer refuses less by
/// version and exactly as much by subject. The attested form of the same id
/// under v1.0 is fine, so the prefix is the whole difference.
#[test]
fn a_v1_0_claimed_subject_is_refused_under_one_reason() {
    let c = conformer();
    let claimed = |schema: &str| -> String {
        format!(
            r#"{{"schema":"taipanbox.dev/agent-event/{schema}","ts":"2026-09-12T16:00:01.000Z","source":"idryx","type":"identity_finding","severity":"high","agent_id":"claimed:agent://acme-bank.example/support/tier1-bot","data":{{"detector":"unmanaged_egress"}}}}"#
        )
    };

    let r = c.check_line(&claimed("v1.0"));
    assert!(!r.valid, "a claimed subject must be refused");
    assert_eq!(
        r.schema_version,
        Some(genaryx_core::SchemaVersion::V1_0),
        "the version was recognized; it is the subject that was refused"
    );
    assert_eq!(
        r.errors,
        vec![genaryx_core::conform::CLAIMED_SUBJECT_REFUSED.to_string()],
        "one fixed reason, so the quarantine panel can count under it"
    );

    // `parse_valid` is what the ingest path calls, and it must hand back the
    // same report, because that report's errors become the quarantine reason.
    let refused = c
        .parse_valid(&claimed("v1.0"))
        .expect_err("the ingest path must be refused too");
    assert_eq!(
        refused.errors,
        vec![genaryx_core::conform::CLAIMED_SUBJECT_REFUSED.to_string()]
    );

    // Under v0.2 the pattern refuses it before the decision is reached.
    let r = c.check_line(&claimed("v0.2"));
    assert!(!r.valid, "v0.2's pattern admits no claimed form");
    assert!(
        r.errors
            .iter()
            .all(|e| e != genaryx_core::conform::CLAIMED_SUBJECT_REFUSED),
        "under v0.2 it is the pattern, not the decision: {:?}",
        r.errors
    );

    // And the attested form of the same id under v1.0 is accepted.
    let attested = claimed("v1.0").replace("claimed:agent://", "agent://");
    assert!(
        c.check_line(&attested).valid,
        "the prefix is the whole difference"
    );
}

/// SPEC 6.4: a consumer MAY refuse v0.3, the interim version where the claimed
/// subject first appeared. This one does, by version. Pinned so that adding
/// v1.0 is not read as having admitted v0.3 with it, and so that the refusal
/// names what IS accepted.
#[test]
fn v0_3_stays_refused_by_version() {
    let c = conformer();
    let line = r#"{"schema":"taipanbox.dev/agent-event/v0.3","ts":"2026-09-12T16:00:00.000Z","source":"tokenfuse","type":"budget_exhausted","agent_id":"agent://acme-bank.example/support/tier1-bot"}"#;
    let r = c.check_line(line);
    assert!(!r.valid);
    assert_eq!(r.schema_version, None);
    assert!(
        r.errors[0].contains("agent-event/v1.0") && r.errors[0].contains("agent-event/v0.1"),
        "the refusal names every accepted version: {:?}",
        r.errors
    );
}

/// The vendored v1.0 file is v0.2's shape with two fields changed, and this
/// pins which two, so a re-vendor that brings anything else with it is a red
/// rather than a surprise. Byte-identity with agent-passport's canonical file
/// is held by estate-gates C2, in another repository; this is the shape, not
/// the bytes.
#[test]
fn the_vendored_v1_0_schema_widens_only_the_subject() {
    use serde_json::Value;
    let v0_2: Value =
        serde_json::from_str(include_str!("../src/schemas/agent-event.v0.2.schema.json"))
            .expect("the vendored v0.2 schema parses");
    let v1_0: Value =
        serde_json::from_str(include_str!("../src/schemas/agent-event.v1.0.schema.json"))
            .expect("the vendored v1.0 schema parses");

    assert_eq!(
        v1_0["$id"],
        "https://taipanbox.dev/agent-passport/v1.0/agent-event.schema.json"
    );
    assert_eq!(
        v1_0["properties"]["schema"]["const"],
        genaryx_core::SchemaVersion::SCHEMA_V1_0
    );
    assert_eq!(
        v1_0["properties"]["agent_id"]["pattern"],
        "^(claimed:)?agent://[a-z0-9.-]+/[a-z0-9._/-]+$"
    );
    assert_eq!(v1_0["properties"]["agent_id"]["maxLength"], 263);
    assert_eq!(v0_2["properties"]["agent_id"]["maxLength"], 255);

    let rest = |v: &Value| {
        let mut props = v["properties"]
            .as_object()
            .expect("properties is an object")
            .clone();
        props.remove("agent_id");
        props.remove("schema");
        props
    };
    assert_eq!(
        rest(&v0_2),
        rest(&v1_0),
        "every other property is identical"
    );
    assert_eq!(v0_2["required"], v1_0["required"]);
    for (key, value) in v0_2.as_object().expect("a schema is an object") {
        if key != "$id" && key != "properties" {
            assert_eq!(
                &v1_0[key], value,
                "top-level `{key}` differs between v0.2 and v1.0"
            );
        }
    }
}
