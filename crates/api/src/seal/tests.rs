use super::*;

fn case() -> CaseFile {
    CaseFile {
        incident_id: "bus:7".to_string(),
        subject: "agent://acme.example/bot/a".to_string(),
        claimed_agent: None,
        run_id: Some("r-1".to_string()),
        lines: Vec::new(),
        facts: Vec::new(),
        digest: "sha256:0000".to_string(),
    }
}

fn decision() -> Decision {
    Decision {
        outcome: Outcome::Applied,
        note: "limit set at the gateway".to_string(),
    }
}

fn pack() -> Value {
    build_pack(
        &case(),
        &[ProofRecord {
            kind: "budget",
            request: json!({ "incident_id": "bus:7", "per_run_usd": "2" }),
            result: json!({ "error": { "kind": "not_configured" } }),
        }],
        &decision(),
        "user://acme.example/operator",
        "2026-11-04T12:00:00Z",
    )
}

#[test]
fn the_same_inputs_make_the_same_bytes_and_the_same_digest() {
    let first = seal_value(&pack()).expect("sealed");
    let second = seal_value(&pack()).expect("sealed");
    assert_eq!(first, second);
    assert!(first.digest.starts_with("sha256:"));
    assert_eq!(first.digest.len(), "sha256:".len() + 64);
    // Another time is another pack.
    let later = build_pack(
        &case(),
        &[],
        &decision(),
        "user://acme.example/operator",
        "2026-11-04T12:00:01Z",
    );
    assert_ne!(seal_value(&later).expect("sealed").digest, first.digest);
}

#[test]
fn the_digest_is_the_sha256_of_the_canonical_text_and_nothing_else() {
    assert_eq!(
        digest_of_text(""),
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        digest_of_text("abc"),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let sealed = seal_value(&pack()).expect("sealed");
    assert_eq!(sealed.digest, digest_of_text(&sealed.canonical));
    // The pack holds the case, the proofs, the decision, who and when, and no
    // digest of itself.
    let value: Value = serde_json::from_str(&sealed.canonical).expect("JSON");
    let mut members: Vec<&str> = value
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    members.sort_unstable();
    assert_eq!(
        members,
        [
            "case",
            "decision",
            "format",
            "proofs",
            "sealed_at",
            "sealed_by"
        ]
    );
    assert_eq!(value["format"], PACK_FORMAT);
    assert_eq!(value["decision"]["outcome"], "applied");
    assert_eq!(value["decision"]["said_by"], "user://acme.example/operator");
}

#[test]
fn a_pack_is_the_pack_its_digest_names_only_if_it_is_whole_canonical_and_of_this_format() {
    let sealed = seal_value(&pack()).expect("sealed");
    assert_eq!(verify(&sealed.canonical, &sealed.digest), Ok(()));

    // A letter changed in the note.
    let altered = sealed.canonical.replace("limit set", "limit not set");
    assert!(matches!(
        verify(&altered, &sealed.digest),
        Err(VerifyError::DigestDiffers { .. })
    ));
    // The same document, written for reading: not the bytes that were hashed.
    let value: Value = serde_json::from_str(&sealed.canonical).expect("JSON");
    let pretty = serde_json::to_string_pretty(&value).expect("pretty");
    assert_eq!(
        verify(&pretty, &sealed.digest),
        Err(VerifyError::NotCanonical)
    );
    // Members in the order they were written in, not the canonical order.
    let unsorted = format!(
        "{{\"sealed_by\":{},\"case\":{}}}",
        value["sealed_by"], value["case"]
    );
    assert_eq!(
        verify(&unsorted, &digest_of_text(&unsorted)),
        Err(VerifyError::NotCanonical)
    );
    // Another format, hashed honestly.
    let other = canonical(&json!({ "format": "something.else/1" })).expect("canonical");
    assert_eq!(
        verify(&other, &digest_of_text(&other)),
        Err(VerifyError::WrongFormat)
    );
    assert_eq!(verify("not json", "sha256:00"), Err(VerifyError::NotJson));
    // The right document under another document's digest.
    let elsewhere = digest_of_text("something else");
    assert_eq!(
        verify(&sealed.canonical, &elsewhere),
        Err(VerifyError::DigestDiffers {
            found: sealed.digest.clone()
        })
    );
}

#[test]
fn a_pack_with_a_number_a_double_cannot_hold_or_past_the_bound_is_not_sealed() {
    let not_exact = json!({ "format": PACK_FORMAT, "n": u64::MAX });
    assert_eq!(
        seal_value(&not_exact),
        Err(SealError::NotCanonical(JcsError::IntegerNotExact))
    );
    let big = json!({ "format": PACK_FORMAT, "x": "y".repeat(PACK_MAX_BYTES) });
    assert!(matches!(
        seal_value(&big),
        Err(SealError::TooLarge { bytes }) if bytes > PACK_MAX_BYTES
    ));
    let at_the_bound = json!({ "format": PACK_FORMAT, "x": "y".repeat(PACK_MAX_BYTES - 40) });
    assert!(seal_value(&at_the_bound).is_ok());
}

#[test]
fn a_decision_is_trimmed_bounded_and_free_of_control_characters() {
    let say = |note: &str| {
        Decision {
            outcome: Outcome::Declined,
            note: note.to_string(),
        }
        .checked()
    };
    assert_eq!(say("  kept as it is \n").expect("ok").note, "kept as it is");
    assert_eq!(say("").expect("ok").note, "");
    assert_eq!(say("a\nb").expect("ok").note, "a\nb");
    assert!(say(&"x".repeat(NOTE_MAX_CHARS)).is_ok());
    assert!(say(&"x".repeat(NOTE_MAX_CHARS + 1)).is_err());
    // Counted in characters, not bytes.
    assert!(say(&"\u{e9}".repeat(NOTE_MAX_CHARS)).is_ok());
    for control in ["a\0b", "a\u{7}b", "a\u{1b}[31mb", "a\rb", "a\u{85}b"] {
        assert!(say(control).is_err(), "{control:?}");
    }
}

#[test]
fn only_a_sha256_in_lowercase_hex_is_a_digest() {
    let ok = format!("sha256:{}", "ab".repeat(32));
    assert!(is_digest(&ok));
    assert!(is_digest(&format!(
        "sha256:{}",
        "0123456789abcdef".repeat(4)
    )));
    for not in [
        String::new(),
        "sha256:".to_string(),
        "sha256:abc".to_string(),
        format!("sha256:{}", "ab".repeat(31)),
        format!("sha256:{}", "ab".repeat(33)),
        format!("sha256:{}", "AB".repeat(32)),
        format!("sha256:{}g", "ab".repeat(31) + "a"),
        format!("sha512:{}", "ab".repeat(32)),
        "ab".repeat(32),
        format!("sha256:../{}", "ab".repeat(30)),
        format!("sha256:{}/", "ab".repeat(31) + "a"),
        format!(" sha256:{}", "ab".repeat(32)),
        format!("sha256:{}\n", "ab".repeat(32)),
    ] {
        assert!(!is_digest(&not), "{not:?}");
    }
    let alphabet: Vec<char> = "sha256:abcdef0123./\\ \n\u{e9}".chars().collect();
    for seed in 0..200u64 {
        let mut x = seed.wrapping_add(31);
        let len = (x % 80) as usize;
        let text: String = (0..len)
            .map(|_| {
                x = x
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                alphabet[(x >> 33) as usize % alphabet.len()]
            })
            .collect();
        if is_digest(&text) {
            assert!(
                !text.contains(['/', '.', '\\', ' ', '\n']),
                "seed {seed}: {text:?}"
            );
        }
    }
}

#[test]
fn a_proof_spec_and_a_decision_read_from_the_shape_the_command_takes_and_nothing_more() {
    let policy: ProofSpec =
        serde_json::from_str(r#"{"kind":"policy","candidate":"- name: p\n"}"#).expect("policy");
    assert!(matches!(policy, ProofSpec::Policy { .. }));
    let budget: ProofSpec =
        serde_json::from_str(r#"{"kind":"budget","per_run_usd":"2","max_steps":20}"#)
            .expect("budget");
    assert!(matches!(
        budget,
        ProofSpec::Budget {
            max_steps: Some(20),
            ..
        }
    ));
    let decision: Decision =
        serde_json::from_str(r#"{"outcome":"deferred"}"#).expect("a decision without a note");
    assert_eq!(decision.outcome, Outcome::Deferred);
    assert_eq!(decision.note, "");

    for refused in [
        r#"{"kind":"policy"}"#,
        r#"{"kind":"policy","candidate":"x","extra":1}"#,
        r#"{"kind":"unknown"}"#,
    ] {
        assert!(
            serde_json::from_str::<ProofSpec>(refused).is_err(),
            "{refused}"
        );
    }
    for refused in [
        r#"{"outcome":"done"}"#,
        r#"{"outcome":"applied","note":"x","by":"someone"}"#,
        r#"{"note":"x"}"#,
    ] {
        assert!(
            serde_json::from_str::<Decision>(refused).is_err(),
            "{refused}"
        );
    }
}
