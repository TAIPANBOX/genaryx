use super::*;

/// The scenarios Mockryx v0.3.1 ships in `scenarios/`, by file name.
const SHIPPED: [&str; 13] = [
    "allow-list-renamed-tool.yaml",
    "approval-required.yaml",
    "declared-tool-poisoning.yaml",
    "dlp-secret-leak.yaml",
    "egress-after-untrusted-read.yaml",
    "injected-page.yaml",
    "on-behalf-of-forged-chain.yaml",
    "pii-in-prompt.yaml",
    "reaction-chain-reaches-heraldyx.yaml",
    "runaway-budget.yaml",
    "token-flood.yaml",
    "verdryx-quality-drift.yaml",
    "wardryx-denied-tool.yaml",
];

const AGENT: &str = "agent://acme.example/bot/a";

fn next(x: &mut u64) -> u64 {
    *x = x
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *x >> 33
}

#[test]
fn every_type_in_the_table_is_there_once_and_names_a_scenario_that_ships() {
    for (i, e) in TABLE.iter().enumerate() {
        assert!(
            TABLE[i + 1..].iter().all(|o| o.event_type != e.event_type),
            "{} is in the table twice",
            e.event_type
        );
        assert_ne!(e.class, IncidentClass::Unclassified, "{}", e.event_type);
        if let Some(scenario) = e.scenario {
            assert!(
                SHIPPED.contains(&scenario),
                "{} names {scenario}, which Mockryx does not ship",
                e.event_type
            );
        }
    }
}

#[test]
fn an_event_type_is_classified_by_the_table_and_any_other_is_unclassified() {
    assert_eq!(
        classify("budget_exhausted"),
        (IncidentClass::Spend, Some("runaway-budget.yaml"))
    );
    assert_eq!(
        classify("loop_detected"),
        (IncidentClass::Loop, Some("runaway-budget.yaml"))
    );
    assert_eq!(
        classify("policy_deny"),
        (IncidentClass::Policy, Some("wardryx-denied-tool.yaml"))
    );
    assert_eq!(
        classify("policy_hold"),
        (IncidentClass::Approval, Some("approval-required.yaml"))
    );
    assert_eq!(
        classify("dlp_pii"),
        (IncidentClass::Data, Some("pii-in-prompt.yaml"))
    );
    assert_eq!(
        classify("dlp_block"),
        (IncidentClass::Data, Some("dlp-secret-leak.yaml"))
    );
    // A refused key and a quality drift have no scenario that rehearses them.
    assert_eq!(classify("identity_mismatch"), (IncidentClass::Identity, None));
    assert_eq!(classify("quality_drift"), (IncidentClass::Quality, None));

    // Nothing near a type is that type, and nothing else is guessed.
    for other in [
        "",
        "Budget_Exhausted",
        "budget_exhausted_extra",
        "xbudget_exhausted",
        "budget",
        " budget_exhausted",
        "budget_exhausted\n",
        "something_new",
        "run_killed",
    ] {
        assert_eq!(
            classify(other),
            (IncidentClass::Unclassified, None),
            "{other:?}"
        );
    }
    for seed in 0..200u64 {
        let mut x = seed.wrapping_add(9);
        let len = next(&mut x) as usize % 24;
        let text: String = (0..len)
            .map(|_| ['a', 'b', '_', 'e', 'x', '\0', ' ', 'é'][next(&mut x) as usize % 8])
            .collect();
        let (class, scenario) = classify(&text);
        if class == IncidentClass::Unclassified {
            assert_eq!(scenario, None, "seed {seed}");
        }
    }
}

#[test]
fn each_class_is_offered_the_kinds_that_fit_it() {
    use RemedyKind::{BudgetLimit, PolicyCandidate, StopAgent};
    assert_eq!(offered(IncidentClass::Spend), [BudgetLimit, StopAgent]);
    assert_eq!(offered(IncidentClass::Loop), [BudgetLimit, StopAgent]);
    assert_eq!(offered(IncidentClass::Policy), [PolicyCandidate, StopAgent]);
    assert_eq!(
        offered(IncidentClass::Approval),
        [PolicyCandidate, StopAgent]
    );
    assert_eq!(offered(IncidentClass::Data), [PolicyCandidate, StopAgent]);
    assert_eq!(offered(IncidentClass::Quality), [StopAgent]);
    for class in [IncidentClass::Spend, IncidentClass::Loop] {
        assert!(!offered(class).contains(&PolicyCandidate));
    }
    for class in [IncidentClass::Policy, IncidentClass::Approval, IncidentClass::Data] {
        assert!(!offered(class).contains(&BudgetLimit));
    }
    assert_eq!(why_none(IncidentClass::Spend), None);
}

#[test]
fn an_identity_incident_is_offered_no_remedy_and_says_why() {
    assert!(offered(IncidentClass::Identity).is_empty());
    assert_eq!(
        why_none(IncidentClass::Identity),
        Some(IDENTITY_IS_FILED_UNDER_THE_KEY)
    );
    // Stopping or revoking the claimed agent is exactly what is not offered.
    assert!(!offered(IncidentClass::Identity).contains(&RemedyKind::StopAgent));
    assert!(!offered(IncidentClass::Identity).contains(&RemedyKind::RevokeDelegation));
    assert!(offered(IncidentClass::Unclassified).is_empty());
    assert_eq!(why_none(IncidentClass::Unclassified), Some(NOT_CLASSIFIED));
}

#[test]
fn a_remedy_is_checked_before_anything_is_proved_or_routed() {
    let ok = |r: Remedy| assert_eq!(r.check(), Ok(()), "{r:?}");
    let refused = |r: Remedy| assert!(r.check().is_err(), "{r:?} was accepted");

    ok(Remedy::PolicyCandidate {
        candidate: "- name: p\n".to_string(),
    });
    refused(Remedy::PolicyCandidate {
        candidate: "  \n".to_string(),
    });
    refused(Remedy::PolicyCandidate {
        candidate: "a\0b".to_string(),
    });
    refused(Remedy::PolicyCandidate {
        candidate: "x".repeat(64 * 1024 + 1),
    });

    ok(Remedy::BudgetLimit {
        per_run_usd: Some("2".to_string()),
        per_step_usd: None,
        max_steps: Some(20),
    });
    refused(Remedy::BudgetLimit {
        per_run_usd: None,
        per_step_usd: None,
        max_steps: None,
    });
    refused(Remedy::BudgetLimit {
        per_run_usd: Some("abc".to_string()),
        per_step_usd: None,
        max_steps: None,
    });
    refused(Remedy::BudgetLimit {
        per_run_usd: None,
        per_step_usd: None,
        max_steps: Some(0),
    });

    ok(Remedy::StopAgent {
        subject: AGENT.to_string(),
    });
    for bad in ["", "bot", "user://acme.example/alice", "agent://", "agent://Acme/a", "agent://a.b/"] {
        refused(Remedy::StopAgent {
            subject: bad.to_string(),
        });
    }

    ok(Remedy::RevokeDelegation {
        subject: AGENT.to_string(),
    });
    ok(Remedy::RevokeDelegation {
        subject: "user://acme.example/alice".to_string(),
    });
    for bad in ["", "alice", "user://", "user://a b", "user://a\"b", "user://a\\b", "user://a\nb"] {
        refused(Remedy::RevokeDelegation {
            subject: bad.to_string(),
        });
    }

    // Whatever a subject is made of, it is checked or refused, never a panic.
    let alphabet: Vec<char> = "agent:/ubr.-_ x\0\n\"\\é".chars().collect();
    for seed in 0..200u64 {
        let mut x = seed.wrapping_add(21);
        let len = next(&mut x) as usize % 20;
        let subject: String = (0..len)
            .map(|_| alphabet[next(&mut x) as usize % alphabet.len()])
            .collect();
        let _ = Remedy::StopAgent {
            subject: subject.clone(),
        }
        .check();
        let _ = Remedy::RevokeDelegation { subject }.check();
    }
}

#[test]
fn a_stop_or_a_revocation_routes_to_its_ceremony_command_with_its_subject() {
    let stop = Remedy::StopAgent {
        subject: AGENT.to_string(),
    };
    assert_eq!(
        stop.route(),
        ApplyRoute::Command {
            command: "agent_stop",
            arguments: serde_json::json!({ "subject": AGENT }),
            person_supplies: &["reason"],
        }
    );
    let revoke = Remedy::RevokeDelegation {
        subject: "user://acme.example/alice".to_string(),
    };
    assert_eq!(
        revoke.route(),
        ApplyRoute::Command {
            command: "delegation_revoke",
            arguments: serde_json::json!({ "subject": "user://acme.example/alice" }),
            person_supplies: &["reason"],
        }
    );
    for remedy in [&stop, &revoke] {
        match remedy.route() {
            ApplyRoute::Command { command, .. } => {
                assert!(ROUTED_COMMANDS.contains(&command), "{command}");
            }
            other => panic!("expected a command, got {other:?}"),
        }
    }
}

#[test]
fn a_policy_or_a_budget_has_no_console_command_and_says_where_it_is_done() {
    let policy = Remedy::PolicyCandidate {
        candidate: "- name: p\n".to_string(),
    };
    let budget = Remedy::BudgetLimit {
        per_run_usd: Some("2".to_string()),
        per_step_usd: None,
        max_steps: None,
    };
    for remedy in [policy, budget] {
        match remedy.route() {
            ApplyRoute::Outside { at } => assert!(!at.is_empty()),
            other => panic!("expected no console command, got {other:?}"),
        }
    }
}

#[test]
fn a_remedy_reads_from_the_shape_the_command_takes_and_nothing_more() {
    let stop: Remedy =
        serde_json::from_str(r#"{"kind":"stop_agent","subject":"agent://acme.example/bot/a"}"#)
            .expect("a stop");
    assert_eq!(stop.kind(), RemedyKind::StopAgent);
    let budget: Remedy =
        serde_json::from_str(r#"{"kind":"budget_limit","per_run_usd":"2","max_steps":20}"#)
            .expect("a budget");
    assert_eq!(budget.kind(), RemedyKind::BudgetLimit);

    for refused in [
        // The reason is a person's, written at the ceremony, never part of a remedy.
        r#"{"kind":"stop_agent","subject":"agent://a/b","reason":"because"}"#,
        r#"{"kind":"apply_everything"}"#,
        r#"{"subject":"agent://a/b"}"#,
        r#"{"kind":"stop_agent"}"#,
        r#"{"kind":"budget_limit","per_run_usd":2}"#,
    ] {
        assert!(
            serde_json::from_str::<Remedy>(refused).is_err(),
            "{refused} was read as a remedy"
        );
    }
}
