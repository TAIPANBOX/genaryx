//! The bench: incidents with a known cause, a packet that hides it, and three
//! questions asked of what the analyst says. Modelled on `costcrew-bench`.
//!
//! The analyst is the real Felyx loop (`explain_case`), given a scripted model: an
//! oracle that names the true cause from the case, a rival that names another, one
//! that cites a run nobody holds, and one that names nothing. The third question
//! replays the remedy that follows from the cause an answer named through the real
//! parsers and judges of the proofs, over the recorded output of the two engines;
//! the child processes around them are held in `policy_proof_test` and
//! `budget_proof_test`.
//!
//! The same cases can be put to a real model: `live_bench_against_an_endpoint`,
//! ignored by default, reads the endpoint from the environment.

use async_trait::async_trait;
use genaryx_api::bench::{
    CAUSES, Cause, CaseScore, Named, RemedyFor, cause_right, gives_away, ids_exist, named_cause,
    render, tally,
};
use genaryx_api::proof::budget::{BudgetCandidate, judge_budget, parse_backtest};
use genaryx_api::proof::{judge, parse_report};
use genaryx_copilot::{
    ChatRequest, ChatTurn, Clients, Felyx, LlmProvider, ProviderDescriptor, ProviderError,
    ToolRegistry, Usage,
};
use serde_json::{Value, json};
use std::sync::Mutex;

const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// A scripted model, one turn per call.
struct Script {
    turns: Mutex<Vec<ChatTurn>>,
}

#[async_trait]
impl LlmProvider for Script {
    async fn chat(&self, _req: ChatRequest) -> Result<ChatTurn, ProviderError> {
        let mut turns = self.turns.lock().expect("turns");
        if turns.is_empty() {
            return Err(ProviderError::Decode("script ran out of turns".into()));
        }
        Ok(turns.remove(0))
    }
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            provider: "script".into(),
            model: "script".into(),
            endpoint: "mock://local".into(),
            local: true,
        }
    }
}

fn says(text: &str) -> ChatTurn {
    ChatTurn {
        content: Some(text.to_string()),
        tool_calls: vec![],
        usage: Usage::default(),
    }
}

fn felyx_with(turns: Vec<ChatTurn>) -> Felyx {
    let provider = Script {
        turns: Mutex::new(turns),
    };
    Felyx::new(
        Box::new(provider),
        ToolRegistry::new(Clients::default()),
        8,
        512,
    )
}

/// One incident whose cause is known.
struct Case {
    name: &'static str,
    truth: Cause,
    run: &'static str,
    /// What the oracle says: the true cause, from the case alone.
    oracle: &'static str,
    /// What a rival says: another cause, from the same case.
    rival: &'static str,
    /// Which remedy, replayed over this incident's recorded history, stops it.
    stopped_by: Option<RemedyFor>,
    packet: Value,
}

fn line(section: &str, plane: &str, state: &str, facts: u32, reason: Value) -> Value {
    json!({ "section": section, "plane": plane, "state": state, "facts": facts,
            "reason": reason, "upstream": null })
}

fn fact(section: &str, plane: &str, row_id: &str, label: &str, row: Value) -> Value {
    json!({ "section": section, "plane": plane, "row_id": row_id, "digest": DIGEST,
            "label": label, "trust": "data", "row": row })
}

fn cases() -> Vec<Case> {
    let crawler = "agent://acme.example/crawler";
    let shell_user = "agent://acme.example/shell-user";
    let billing = "agent://acme.example/billing";
    vec![
        Case {
            name: "spend-loop",
            truth: Cause::RetryLoop,
            run: "run-spend-1",
            oracle: "Run `run-spend-1` of agent `agent://acme.example/crawler` hit its budget: \
                     the case shows 118 of 120 calls to the same tool, search, at 31 calls a \
                     minute, a retry loop. It spent $9.97 of a $10 budget.",
            rival: "Run `run-spend-1` ended at its budget because a policy denied the tool it \
                    asked for.",
            stopped_by: Some(RemedyFor::Budget),
            packet: json!({
                "incident_id": "bus:11",
                "subject": crawler,
                "claimed_agent": null,
                "run_id": "run-spend-1",
                "lines": [
                    line("event", "bus", "answered", 1, Value::Null),
                    line("spend", "tokenfuse", "answered", 1, Value::Null),
                    line("policy", "wardryx", "empty", 0, Value::Null),
                ],
                "facts": [
                    fact("event", "bus", "11", "the incident's event row", json!({
                        "id": 11, "type": "budget_exhausted", "agent_id": crawler,
                        "run_id": "run-spend-1",
                        "data": { "budget_usd": 10, "spent_usd": 9.97 }
                    })),
                    fact("spend", "tokenfuse", "run-spend-1", "the run's spend and gateway decisions", json!({
                        "run_id": "run-spend-1", "agent_id": crawler,
                        "spent_usd": 9.97, "calls": 120,
                        "tools": { "search": 118, "fetch": 2 },
                        "calls_per_minute": 31
                    })),
                ],
                "digest": DIGEST,
            }),
        },
        Case {
            name: "denied-shell",
            truth: Cause::DeniedTool,
            run: "run-shell-2",
            oracle: "Run `run-shell-2` was refused: the agent asked for the shell tool and the \
                     policy denied it, a denied tool call. The case holds no other trouble.",
            rival: "Run `run-shell-2` kept calling the same tool over and over, a retry loop.",
            stopped_by: Some(RemedyFor::Policy),
            packet: json!({
                "incident_id": "bus:12",
                "subject": shell_user,
                "claimed_agent": null,
                "run_id": "run-shell-2",
                "lines": [
                    line("event", "bus", "answered", 1, Value::Null),
                    line("spend", "tokenfuse", "answered", 1, Value::Null),
                    line("policy", "wardryx", "answered", 1, Value::Null),
                ],
                "facts": [
                    fact("event", "bus", "12", "the incident's event row", json!({
                        "id": 12, "type": "policy_deny", "agent_id": shell_user,
                        "run_id": "run-shell-2",
                        "data": { "tool": "shell", "decision": "deny" }
                    })),
                    fact("spend", "tokenfuse", "run-shell-2", "the run's spend and gateway decisions", json!({
                        "run_id": "run-shell-2", "agent_id": shell_user,
                        "spent_usd": 0.012, "calls": 6,
                        "tools": { "shell": 1, "read": 5 }
                    })),
                    fact("policy", "wardryx", "no-shell", "the policy in force", json!({
                        "target": "agent://acme.example/*", "deny_tool": ["shell"]
                    })),
                ],
                "digest": DIGEST,
            }),
        },
        Case {
            name: "unknown-key",
            truth: Cause::UnknownKey,
            run: "run-key-3",
            oracle: "The incident is filed under the key `key:forge-imposter`, which claimed \
                     agent `agent://acme.example/billing`. The gateway does not hold that key, \
                     an unknown key. The identity plane was not asked.",
            rival: "The key `key:forge-imposter` claimed agent `agent://acme.example/billing` \
                    and the policy denied the tool it asked for.",
            stopped_by: None,
            packet: json!({
                "incident_id": "bus:13",
                "subject": "key:forge-imposter",
                "claimed_agent": billing,
                "run_id": "run-key-3",
                "lines": [
                    line("event", "bus", "answered", 1, Value::Null),
                    line("spend", "tokenfuse", "empty", 0, Value::Null),
                    line("identity", "idryx", "not_asked", 0, json!(
                        "the incident is filed under a key, not an agent: asking about the \
                         agent it claimed would charge that agent with the attempt")),
                ],
                "facts": [
                    fact("event", "bus", "13", "the incident's event row", json!({
                        "id": 13, "type": "identity_mismatch", "agent_id": billing,
                        "run_id": "run-key-3",
                        "data": { "key_id": "forge-imposter", "keys_held": ["billing-prod"] }
                    })),
                ],
                "digest": DIGEST,
            }),
        },
    ]
}

// ---- the recorded replays, read by the real parsers and judges ------------------

fn policy_replay(run: &str, stops: bool) -> String {
    let change = if stops {
        format!(
            "1 of 3 replayable decision(s) change:\n\n  allow -> deny   agent://acme.example/x  run {run}\n        was: allowed: request satisfies all matched policy rules\n        now: tool \"shell\" is denied by policy \"no-shell\"\n"
        )
    } else {
        "0 of 3 replayable decision(s) change:\n  none.\n".to_string()
    };
    format!(
        "Replayed 3 decision(s) from /e/wardryx.ndjson\n\n  reproduced             3\n  approval-decided       0\n  approval-spent         0\n  approval-refused       0\n  not archived           0\n  unreadable             0\n  diverged               0\n\nAgainst /tmp/candidate.yaml, {change}"
    )
}

fn backtest(runs: (u64, u64), calls: (u64, u64), money: (&str, &str, &str)) -> String {
    format!(
        "TokenFuse backtest\n  candidate policy   : budget/run $2.0000\n  runs               : {} ({} affected)\n  calls              : {} ({} would be blocked)\n  spend in trace     : ${}\n  would have saved   : ${} (50.0%)\n  projected spend    : ${}\n",
        runs.0, runs.1, calls.0, calls.1, money.0, money.1, money.2
    )
}

/// Would the remedy, replayed over this incident's recorded history, have stopped
/// the incident? Read by the same parsers and judges the proofs use.
fn replay_stops(case: &Case, remedy: RemedyFor) -> bool {
    let stops = case.stopped_by == Some(remedy);
    match remedy {
        RemedyFor::Policy => {
            let report = parse_report(&policy_replay(case.run, stops)).expect("a policy report");
            judge(&report, case.run).stops_this_incident == Some(true)
        }
        RemedyFor::Budget => {
            let candidate =
                BudgetCandidate::from_parts(Some("2"), None, None).expect("a candidate");
            let (alone, whole) = if stops {
                (
                    backtest((1, 1), (120, 88), ("9.970000", "7.800000", "2.170000")),
                    backtest((40, 3), (900, 130), ("21.500000", "9.100000", "12.400000")),
                )
            } else {
                (
                    backtest((1, 0), (120, 0), ("9.970000", "0.000000", "9.970000")),
                    backtest((40, 3), (900, 130), ("21.500000", "9.100000", "12.400000")),
                )
            };
            let alone = parse_backtest(&alone, &candidate).expect("a backtest report");
            let whole = parse_backtest(&whole, &candidate).expect("a backtest report");
            judge_budget(case.run, &candidate, &alone, &whole, 120, false)
                .expect("a budget proof")
                .stops_this_incident
        }
    }
}

/// Put one case to one analyst and score the three questions.
async fn score(case: &Case, felyx: &Felyx) -> CaseScore {
    let answer = felyx
        .explain_case(&case.packet)
        .await
        .expect("an answer");
    let named = named_cause(&answer.text);
    let remedy_stops = match named {
        Named::Cause(cause) => cause.remedy().map(|remedy| replay_stops(case, remedy)),
        _ => None,
    };
    CaseScore {
        case: case.name.to_string(),
        truth: case.truth,
        named,
        cause_right: cause_right(named, case.truth),
        ids_exist: ids_exist(&answer.unsupported_claims, &answer.text),
        remedy_stops,
    }
}

async fn bench(draft_of: impl Fn(&Case) -> Vec<ChatTurn>) -> Vec<CaseScore> {
    let mut scores = Vec::new();
    for case in cases() {
        let felyx = felyx_with(draft_of(&case));
        scores.push(score(&case, &felyx).await);
    }
    scores
}

// ---- the tests ------------------------------------------------------------------

#[test]
fn the_packets_hide_the_cause_and_every_cause_has_a_case() {
    let all = cases();
    for case in &all {
        let text = case.packet.to_string();
        for cause in CAUSES {
            assert_eq!(
                gives_away(&text, cause),
                None,
                "{} gives {cause:?} away",
                case.name
            );
        }
    }
    for cause in CAUSES {
        assert!(all.iter().any(|c| c.truth == cause), "{cause:?} has no case");
    }
}

#[test]
fn the_recorded_replays_say_what_the_bench_says_they_do() {
    for case in cases() {
        for remedy in [RemedyFor::Budget, RemedyFor::Policy] {
            assert_eq!(
                replay_stops(&case, remedy),
                case.stopped_by == Some(remedy),
                "{} / {remedy:?}",
                case.name
            );
        }
    }
}

#[tokio::test]
async fn an_oracle_that_reads_the_case_scores_every_question_it_is_asked() {
    let scores = bench(|case| vec![says(case.oracle)]).await;
    println!("{}", render(&scores));
    for s in &scores {
        assert_eq!(s.named, Named::Cause(s.truth), "{}", s.case);
        assert!(s.cause_right, "{}", s.case);
        assert!(s.ids_exist, "{}", s.case);
    }
    assert_eq!(scores[0].remedy_stops, Some(true));
    assert_eq!(scores[1].remedy_stops, Some(true));
    assert_eq!(scores[2].remedy_stops, None, "a refused key calls for no remedy");
    let t = tally(&scores);
    assert_eq!(
        (t.cases, t.cause_right, t.ids_exist, t.remedy_applicable, t.remedy_stops),
        (3, 3, 3, 2, 2)
    );
}

#[tokio::test]
async fn a_rival_cause_misses_the_cause_and_the_remedy_but_not_the_ids() {
    let scores = bench(|case| vec![says(case.rival)]).await;
    println!("{}", render(&scores));
    for s in &scores {
        assert!(!s.cause_right, "{}: named {:?}", s.case, s.named);
        assert!(s.ids_exist, "{}: the rival cites only what the case holds", s.case);
        assert_eq!(
            s.remedy_stops,
            Some(false),
            "{}: the remedy that follows from the wrong cause does not stop it",
            s.case
        );
    }
    let t = tally(&scores);
    assert_eq!((t.cause_right, t.ids_exist, t.remedy_stops), (0, 3, 0));
}

#[tokio::test]
async fn an_answer_that_cites_a_run_nobody_holds_misses_the_ids_and_is_withheld() {
    let scores = bench(|case| {
        let draft = format!("{} Run `run-ghost-9` also spent $4.", case.oracle);
        vec![says(&draft), says(&draft)]
    })
    .await;
    println!("{}", render(&scores));
    for s in &scores {
        assert!(!s.ids_exist, "{}", s.case);
        assert!(!s.cause_right, "{}: a withheld chain names no cause", s.case);
        assert_eq!(s.remedy_stops, None, "{}", s.case);
    }
}

#[tokio::test]
async fn an_answer_that_names_nothing_misses_the_cause_and_has_no_remedy_to_replay() {
    let scores = bench(|_| vec![says("The case does not say what caused this incident.")]).await;
    for s in &scores {
        assert_eq!(s.named, Named::Nothing, "{}", s.case);
        assert!(!s.cause_right, "{}", s.case);
        assert!(s.ids_exist, "{}", s.case);
        assert_eq!(s.remedy_stops, None, "{}", s.case);
    }
}

#[tokio::test]
async fn an_answer_that_names_two_causes_names_none() {
    let scores = bench(|case| {
        vec![says(&format!(
            "{} It could equally be a retry loop on one tool.",
            case.rival
        ))]
    })
    .await;
    for s in scores.iter().filter(|s| s.truth != Cause::RetryLoop) {
        assert_eq!(s.named, Named::Several, "{}", s.case);
        assert!(!s.cause_right, "{}", s.case);
    }
}

/// The same cases, put to a real model through an OpenAI-compatible endpoint:
///
/// `GENARYX_BENCH_BASE_URL` (for example `http://192.168.1.10:11434/v1`),
/// `GENARYX_BENCH_MODEL`, and optionally `GENARYX_BENCH_API_KEY_REF` (`env:NAME`).
/// Run with `cargo test -p genaryx-api --test it live_bench -- --ignored --nocapture`.
/// It prints the report and asserts only that the endpoint answered.
#[tokio::test]
#[ignore = "needs a model endpoint named by GENARYX_BENCH_BASE_URL and GENARYX_BENCH_MODEL"]
async fn live_bench_against_an_endpoint() {
    let (Ok(base_url), Ok(model)) = (
        std::env::var("GENARYX_BENCH_BASE_URL"),
        std::env::var("GENARYX_BENCH_MODEL"),
    ) else {
        panic!("set GENARYX_BENCH_BASE_URL and GENARYX_BENCH_MODEL");
    };
    let config = genaryx_copilot::CopilotConfig {
        provider: genaryx_copilot::ProviderKind::OpenAiCompat,
        base_url: Some(base_url),
        model: Some(model),
        api_key_ref: std::env::var("GENARYX_BENCH_API_KEY_REF").ok(),
        ..Default::default()
    };
    let mut scores = Vec::new();
    for case in cases() {
        let provider = genaryx_copilot::build_provider(&config)
            .expect("a provider")
            .expect("a provider is configured");
        let felyx = Felyx::new(provider, ToolRegistry::new(Clients::default()), 8, 1024);
        scores.push(score(&case, &felyx).await);
    }
    println!("{}", render(&scores));
}
