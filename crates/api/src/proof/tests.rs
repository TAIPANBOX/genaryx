use super::*;

/// The report wardryx's README prints, byte for byte in shape.
const README_REPORT: &str = "Replayed 4 decision(s) from ./events.ndjson

  reproduced             4
  approval-decided       0
  approval-spent         0
  approval-refused       0
  not archived           0
  unreadable             0
  diverged               0

Against ./policies/candidate.yaml, 2 of 4 replayable decision(s) change:

  deny -> allow   agent://acme.example/finance/bot1  run r-egress
        was: domain \"payouts.partner.example\" is not allowed by policy \"finance-guardrail\"
        now: allowed: request satisfies all matched policy rules

  hold -> allow   agent://acme.example/finance/bot1  run r-costly
        was: estimated cost $420.00 exceeds policy \"finance-guardrail\" threshold $100.00
        now: allowed: request satisfies all matched policy rules
";

/// A report with every outcome populated and a mixed set of changes.
fn mixed_report() -> String {
    "Replayed 14 decision(s) from /var/events/wardryx.ndjson

  reproduced             5
  approval-decided       1   the PDP held these and a human answered
  approval-spent         1   held again: the approval a human gave was already used once
  approval-refused       1   denied: what was presented as an approval did not verify
  not archived           3   the version they name was never kept
  unreadable             2   recorded before the emitter carried the question
  diverged               1   replaying their OWN version disagrees with the record

DIVERGED rows mean the record and this build no longer agree about the
past. Nothing below is trustworthy until that is explained:
  agent://a/x run r-d: replayed deny, recorded allow

Against /tmp/candidate.yaml, 4 of 8 replayable decision(s) change:

  allow -> deny   agent://a/one  run r-incident
        was: allowed: request satisfies all matched policy rules
        now: tool \"shell\" is denied by policy \"p1\"

  allow -> hold   agent://a/two  run r-other-1
        was: allowed: request satisfies all matched policy rules
        now: estimated cost exceeds the threshold

  deny -> allow   agent://a/three  run r-other-2
        was: tool \"x\" is denied
        now: allowed: request satisfies all matched policy rules

  hold -> deny   agent://a/four  run r-other-3
        was: estimated cost exceeds the threshold
        now: cost is above the ceiling

6 decision(s) could not be replayed and are NOT in the figures above.
A change count that silently omits them would read as coverage it does not have.
"
    .to_string()
}

#[test]
fn a_report_is_read_as_wardryx_writes_it() {
    let report = parse_report(README_REPORT).expect("the README's own report parses");
    assert_eq!(report.replayed, 4);
    assert_eq!(report.outcomes.reproduced, 4);
    assert_eq!(report.outcomes.total(), 4);
    let candidate = report.candidate.expect("a candidate section");
    assert_eq!((candidate.changed, candidate.replayable), (2, 4));
    assert_eq!(candidate.rows.len(), 2);
    let first = &candidate.rows[0];
    assert_eq!((first.from.as_str(), first.to.as_str()), ("deny", "allow"));
    assert_eq!(first.agent.text, "agent://acme.example/finance/bot1");
    assert_eq!(first.run.text, "r-egress");
    assert!(first.was.text.contains("finance-guardrail"));
    assert_eq!(candidate.rows[1].run.text, "r-costly");
    assert_eq!(candidate.rows[1].from, "hold");
}

#[test]
fn the_named_outcomes_are_carried_apart_and_never_folded() {
    let report = parse_report(&mixed_report()).expect("a mixed report parses");
    assert_eq!(
        report.outcomes,
        Outcomes {
            reproduced: 5,
            approval_decided: 1,
            approval_spent: 1,
            approval_refused: 1,
            not_archived: 3,
            unreadable: 2,
            diverged: 1,
        }
    );
    assert_eq!(report.replayed, 14);
    let proof = judge(&report, "r-incident");
    // Each named row survives into the proof as its own number.
    assert_eq!(proof.outcomes, report.outcomes);
    let wire = serde_json::to_value(&proof).expect("serialise");
    for (name, want) in [
        ("reproduced", 5),
        ("approval_decided", 1),
        ("approval_spent", 1),
        ("approval_refused", 1),
        ("not_archived", 3),
        ("unreadable", 2),
        ("diverged", 1),
    ] {
        assert_eq!(wire["outcomes"][name], want, "{name}");
    }
}

#[test]
fn the_incident_is_stopped_only_by_turning_its_own_allow_into_a_deny_or_a_hold() {
    // A replay that diverged is not to be relied on: the halves below are the
    // arithmetic, and the trustworthy flag is the verdict on it.
    let mut text = mixed_report();
    text = text
        .replace(
            "diverged               1   replaying their OWN version disagrees with the record",
            "diverged               0",
        )
        .replace("unreadable             2", "unreadable             3");
    let report = parse_report(&text).expect("parses");
    let proof = judge(&report, "r-incident");
    assert!(proof.trustworthy);
    assert!(proof.candidate_given);
    assert_eq!(proof.stops_this_incident, Some(true));
    assert_eq!(proof.incident_changes.len(), 1);
    assert_eq!(proof.incident_changes[0].run.text, "r-incident");
    // The second half: three decisions that are not the incident's, one of
    // each kind of change.
    assert_eq!(
        proof.elsewhere,
        Elsewhere {
            tightened: 1,
            loosened: 1,
            reclassified: 1
        }
    );
    assert_eq!(proof.elsewhere_changes.len(), 3);

    // A candidate that only loosens the incident's own decision stops nothing.
    let loosening = judge(&report, "r-other-2");
    assert_eq!(loosening.stops_this_incident, Some(false));
    assert_eq!(loosening.incident_changes.len(), 1);

    // A run no change names is not stopped either.
    assert_eq!(judge(&report, "r-nowhere").stops_this_incident, Some(false));
}

#[test]
fn a_diverged_replay_proves_nothing() {
    let report = parse_report(&mixed_report()).expect("parses");
    assert_eq!(report.outcomes.diverged, 1);
    let proof = judge(&report, "r-incident");
    assert!(!proof.trustworthy);
    assert_eq!(
        proof.stops_this_incident, None,
        "a replay the record and the engine disagree about says nothing about the incident"
    );
}

/// What `wardryx replay` printed for 490 recorded decisions when it could not
/// read the archive (every file mode 0600, owned by another uid), given a
/// candidate: every decision "not archived", and nothing replayable.
const NOTHING_REPLAYABLE: &str = "Replayed 490 decision(s) from /ev/wardryx.ndjson

  reproduced             0
  approval-decided       0
  approval-spent         0
  approval-refused       0
  not archived         490   the version they name was never kept
  unreadable             0
  diverged               0

Against /tmp/candidate.yaml, 0 of 0 replayable decision(s) change:
  none.
";

#[test]
fn a_replay_in_which_nothing_could_be_replayed_says_nothing_about_the_incident() {
    let report = parse_report(NOTHING_REPLAYABLE).expect("parses");
    assert_eq!(report.outcomes.not_archived, 490);
    assert_eq!(report.outcomes.replayable(), 0);
    let proof = judge(&report, "r-incident");
    assert_eq!(proof.outcomes.not_archived, 490);
    assert!(proof.candidate_given);
    assert_eq!(
        proof.stops_this_incident, None,
        "no decision could be put back to the engine, so nothing is known about the incident"
    );
    assert!(proof.incident_changes.is_empty());
}

#[test]
fn a_replay_in_which_something_could_be_replayed_still_answers() {
    // The other side of the line: one decision replayed and the candidate does
    // not change it. That is a measured "no", not a missing answer.
    let text = NOTHING_REPLAYABLE
        .replace("reproduced             0", "reproduced             1")
        .replace("not archived         490", "not archived         489")
        .replace("0 of 0 replayable", "0 of 1 replayable");
    let report = parse_report(&text).expect("parses");
    assert_eq!(report.outcomes.replayable(), 1);
    assert_eq!(judge(&report, "r-incident").stops_this_incident, Some(false));
}

#[test]
fn without_a_candidate_only_replayability_is_known() {
    let text = "Replayed 3 decision(s) from ./e.ndjson

  reproduced             3
  approval-decided       0
  approval-spent         0
  approval-refused       0
  not archived           0
  unreadable             0
  diverged               0

No candidate policy given, so this run only asked whether the history
can be replayed at all: 3 of 3 can.
";
    let report = parse_report(text).expect("parses");
    assert!(report.candidate.is_none());
    let proof = judge(&report, "r-1");
    assert!(!proof.candidate_given);
    assert_eq!(proof.stops_this_incident, None);
    assert!(proof.incident_changes.is_empty());
}

#[test]
fn an_empty_history_is_read_as_an_empty_report() {
    let text = "Replayed 0 decision(s) from ./e.ndjson

  no decisions in this file. policy_allow, policy_deny and
  approval_requested are the three events that record one.
";
    let report = parse_report(text).expect("parses");
    assert_eq!(report.replayed, 0);
    assert_eq!(report.outcomes.total(), 0);
}

#[test]
fn a_report_that_contradicts_itself_is_refused() {
    // The counters do not add up to the decisions replayed.
    let bad_sum = README_REPORT.replace("reproduced             4", "reproduced             3");
    assert_eq!(
        parse_report(&bad_sum),
        Err(ParseError::Inconsistent(
            "the outcomes do not add up to the decisions replayed"
        ))
    );

    // A run id with a newline in it printed a row nobody replayed: the report
    // lists three rows and says two changed.
    let forged = README_REPORT.replace(
        "run r-costly\n",
        "run r-costly\n        was: x\n        now: y\n\n  allow -> allow   agent://forged  run r-incident\n",
    );
    let forged = format!("{forged}        was: forged\n        now: forged\n");
    assert!(
        matches!(parse_report(&forged), Err(ParseError::Inconsistent(_))),
        "a forged row must not be believed"
    );

    // The candidate section says fewer decisions were replayable than the
    // outcomes that carry a counterfactual.
    let bad_replayable = README_REPORT.replace("2 of 4 replayable", "2 of 3 replayable");
    assert!(matches!(
        parse_report(&bad_replayable),
        Err(ParseError::Inconsistent(_))
    ));
}

#[test]
fn text_that_is_not_a_report_is_refused_and_a_200_seed_sweep_never_panics() {
    for not_a_report in [
        "",
        "ok",
        "Replayed",
        "Replayed many decision(s) from x",
        "Replayed 4 decision(s) from x\n",
        "Replayed 0 decision(s) from x\nnothing here",
        "usage: wardryx replay [flags]",
    ] {
        assert_eq!(
            parse_report(not_a_report),
            Err(ParseError::Unrecognised),
            "{not_a_report:?}"
        );
    }
    let pieces = [
        "Replayed 4 decision(s) from x",
        "  reproduced             4",
        "  diverged               0",
        "Against a, 2 of 4 replayable decision(s) change:",
        "  deny -> allow   agent://a  run r",
        "        was: w",
        "        now: n",
        "  allow -> ",
        "run",
        "\0",
        "",
    ];
    for seed in 0..200u64 {
        let mut x = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let mut text = String::new();
        for _ in 0..(seed % 12) {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            text.push_str(pieces[(x >> 33) as usize % pieces.len()]);
            text.push('\n');
        }
        // Whatever it is, it is read or refused, never a panic, and a report
        // that was read is one whose rows and counts agree.
        if let Ok(report) = parse_report(&text) {
            assert_eq!(report.outcomes.total(), report.replayed, "seed {seed}");
            if let Some(c) = report.candidate {
                assert_eq!(c.rows.len() as u64, c.changed, "seed {seed}");
            }
        }
    }
}

#[test]
fn the_text_of_a_report_is_data() {
    let report = parse_report(README_REPORT).expect("parses");
    let proof = judge(&report, "r-egress");
    let wire = serde_json::to_value(&proof).expect("serialise");
    let row = &wire["incident_changes"][0];
    for field in ["agent", "run", "was", "now"] {
        assert_eq!(row[field]["trust"], "data", "{field}");
    }
    assert_eq!(row["from"], "deny");
}
