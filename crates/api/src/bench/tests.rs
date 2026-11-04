use super::*;

fn score(case: &str, truth: Cause, named: Named, ids: bool, remedy: Option<bool>) -> CaseScore {
    CaseScore {
        case: case.to_string(),
        truth,
        named,
        cause_right: cause_right(named, truth),
        ids_exist: ids,
        remedy_stops: remedy,
    }
}

#[test]
fn a_cause_is_named_when_one_cause_is_named_in_a_sentence_and_no_other() {
    for (text, cause) in [
        (
            "The run kept calling the same tool, a retry loop, until its budget was gone.",
            Cause::RetryLoop,
        ),
        ("It is a LOOP.", Cause::RetryLoop),
        (
            "The policy denied the tool the agent asked for.",
            Cause::DeniedTool,
        ),
        (
            "The gateway denies that tool to this agent.",
            Cause::DeniedTool,
        ),
        (
            "The key is an unknown key: the gateway does not hold it.",
            Cause::UnknownKey,
        ),
        (
            "The key presented is not held by the gateway.",
            Cause::UnknownKey,
        ),
    ] {
        assert_eq!(named_cause(text), Named::Cause(cause), "{text}");
    }
    assert_eq!(named_cause(""), Named::Nothing);
    // The words of a cause are in one sentence, not spread over two, and one word
    // of a group is not the group.
    assert_eq!(
        named_cause("It was denied. A tool was used."),
        Named::Nothing
    );
    assert_eq!(named_cause("A tool was used."), Named::Nothing);
    assert_eq!(named_cause("The key was presented."), Named::Nothing);
    assert_eq!(
        named_cause("The case does not say what caused this."),
        Named::Nothing
    );
    // Two causes in one answer is no cause named.
    assert_eq!(
        named_cause("It was a retry loop. Or the policy denied the tool."),
        Named::Several
    );
    // The words of a cause in another order are still the cause's words, but the
    // words of two different causes never make a third.
    assert_eq!(
        named_cause("An unknown key. A retry loop. A denied tool."),
        Named::Several
    );
}

#[test]
fn a_packet_gives_the_cause_away_only_by_its_words() {
    assert_eq!(
        gives_away("calls 118 of 120 to search, a retry loop", Cause::RetryLoop),
        Some("retry")
    );
    assert_eq!(
        gives_away("RETRY_LOOP", Cause::RetryLoop),
        Some("retry_loop")
    );
    assert_eq!(
        gives_away("a Denied Tool", Cause::DeniedTool),
        Some("denied tool")
    );
    assert_eq!(
        gives_away("reason: unknown_key", Cause::UnknownKey),
        Some("unknown_key")
    );
    assert_eq!(
        gives_away(
            "118 of 120 calls to one tool at 31 a minute; decision deny; key_id forge-imposter",
            Cause::RetryLoop
        ),
        None
    );
    // A word of another cause is not this cause's giveaway.
    assert_eq!(gives_away("unknown_key", Cause::RetryLoop), None);
    for cause in CAUSES {
        assert_eq!(gives_away("", cause), None);
    }
}

#[test]
fn the_remedy_a_cause_calls_for_is_the_one_that_follows_from_it() {
    assert_eq!(Cause::RetryLoop.remedy(), Some(RemedyFor::Budget));
    assert_eq!(Cause::DeniedTool.remedy(), Some(RemedyFor::Policy));
    assert_eq!(Cause::UnknownKey.remedy(), None);
}

#[test]
fn the_first_two_questions_are_judged_on_the_cause_named_and_on_what_was_left_unsupported() {
    assert!(cause_right(
        Named::Cause(Cause::RetryLoop),
        Cause::RetryLoop
    ));
    assert!(!cause_right(
        Named::Cause(Cause::DeniedTool),
        Cause::RetryLoop
    ));
    assert!(!cause_right(Named::Nothing, Cause::RetryLoop));
    assert!(!cause_right(Named::Several, Cause::RetryLoop));

    assert!(ids_exist(&[], "Run `run-1` spent $9.97."));
    assert!(!ids_exist(
        &["run-ghost-9".to_string()],
        "Run `run-1` spent $9.97."
    ));
    assert!(!ids_exist(
        &[],
        &format!("{WITHHELD_PREFIX}: what the draft rested on is not in the case.")
    ));
}

#[test]
fn a_remedy_that_does_not_apply_is_counted_as_neither_a_pass_nor_a_miss() {
    let scores = [
        score(
            "a",
            Cause::RetryLoop,
            Named::Cause(Cause::RetryLoop),
            true,
            Some(true),
        ),
        score(
            "b",
            Cause::DeniedTool,
            Named::Cause(Cause::DeniedTool),
            true,
            Some(false),
        ),
        score(
            "c",
            Cause::UnknownKey,
            Named::Cause(Cause::UnknownKey),
            true,
            None,
        ),
        score("d", Cause::RetryLoop, Named::Nothing, false, None),
    ];
    assert_eq!(
        tally(&scores),
        Totals {
            cases: 4,
            cause_right: 3,
            ids_exist: 3,
            remedy_applicable: 2,
            remedy_stops: 1,
        }
    );
    assert_eq!(
        tally(&[]),
        Totals {
            cases: 0,
            cause_right: 0,
            ids_exist: 0,
            remedy_applicable: 0,
            remedy_stops: 0,
        }
    );
}

#[test]
fn the_report_names_every_case_and_every_question() {
    let scores = [
        score(
            "spend-loop",
            Cause::RetryLoop,
            Named::Cause(Cause::RetryLoop),
            true,
            Some(true),
        ),
        score(
            "unknown-key",
            Cause::UnknownKey,
            Named::Several,
            false,
            None,
        ),
    ];
    let text = render(&scores);
    for needle in [
        "spend-loop",
        "unknown-key",
        "retry_loop",
        "(several)",
        "named the cause right: 1 of 2",
        "every cited id exists: 1 of 2",
        "the remedy stops the replay: 1 of 1",
    ] {
        assert!(text.contains(needle), "{needle} is missing from:\n{text}");
    }
    // The line of a case that missed says so in capitals, where a reader looks.
    let missed = text
        .lines()
        .find(|l| l.starts_with("unknown-key"))
        .expect("a line");
    assert!(missed.contains("NO"), "{missed}");
    assert!(missed.contains("n/a"), "{missed}");
}

#[test]
fn a_200_seed_sweep_of_text_never_panics_and_never_names_two_ways() {
    let pieces = [
        "retry loop",
        "the same tool",
        "denied the tool",
        "unknown key",
        "not held",
        ".",
        ";",
        "\n",
        "",
        "\0",
        "LOOP",
        "é",
    ];
    for seed in 0..200u64 {
        let mut x = seed.wrapping_add(17);
        let mut text = String::new();
        for _ in 0..(seed % 10) {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            text.push_str(pieces[(x >> 33) as usize % pieces.len()]);
            text.push(' ');
        }
        // Whatever the text is, it names one cause, none or several, and a single
        // cause named is a cause whose words are in it.
        if let Named::Cause(cause) = named_cause(&text) {
            assert!(
                cause
                    .groups()
                    .iter()
                    .any(|g| g.iter().all(|w| text.to_lowercase().contains(w))),
                "seed {seed}: {text:?}"
            );
        }
    }
}
