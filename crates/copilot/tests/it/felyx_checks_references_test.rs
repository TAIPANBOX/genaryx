//! Felyx states an id or an amount only when a tool returned it in the same
//! answer, and an explanation or a card never rests on one that no tool did.
//!
//! The fixture is one incident on the forge fleet's shape: a run over its
//! budget, the Cloud's rows for it, a policy and an approval. The model is a
//! script, so what is under test is the loop around it: what the script writes
//! is what a model might write, and these assert what reaches the operator.

use std::sync::{Arc, Mutex};

use genaryx_connectors::{CloudClient, IdryxClient, WardryxClient};
use genaryx_copilot::{
    ActionKind, Answer, ChatRequest, ChatTurn, Clients, Felyx, Role, ToolCall, ToolRegistry, Usage,
};
use serde_json::{Value, json};

use crate::felyx_reads_budgets_test::{Script, Stub, calls, last_user_message, says, spawn_cloud};

const AGENT: &str = "agent://taipanbox.dev/routers/brume";
const INCIDENT: &str = "budget_exhausted:p1-brume";

pub(crate) fn incident_row(id: &str, run: &str, summary: Option<&str>) -> Value {
    json!({
        "id": id, "org": "default", "run_id": run, "agent_id": AGENT,
        "kind": "budget_exhausted", "severity": "high",
        "first_seen_millis": 1, "last_seen_millis": 2, "occurrences": 3,
        "acknowledged": false, "last_notified_millis": 0,
        "source": "tokenfuse", "summary": summary,
    })
}

pub(crate) fn run_row(run: &str, spent: i64) -> Value {
    json!({
        "run_id": run, "model": "claude-haiku-4-5", "agent_id": AGENT,
        "spent_microusd": spent, "calls": 1, "cache_hits": 0, "steps": 1,
        "last_seen_millis": 0, "killed": false,
    })
}

fn plane() -> Stub {
    spawn_cloud(vec![
        (
            "/v1/incidents",
            json!([incident_row(INCIDENT, "p1-brume", None)]).to_string(),
        ),
        (
            "/v1/runs",
            json!([run_row("p1-brume", 2008), run_row("mig-flint", 3210)]).to_string(),
        ),
        (
            "/v1/policies",
            r#"[{"id":"deny-shell","target":"agent://taipanbox.dev/*","deny_tool":["shell"]}]"#
                .to_string(),
        ),
        (
            "/v1/approvals",
            json!([{"approval_id": "ap-77", "agent_id": AGENT, "run_id": "p1-brume",
                    "requested_at": "2026-11-04T09:00:00Z", "pending": true}])
            .to_string(),
        ),
        (
            "/api/alerts",
            json!([{"detector": "attestation_missing", "identity": AGENT, "severity": "high",
                    "time": "2026-11-04T09:00:00Z", "summary": "attestation=none"}])
            .to_string(),
        ),
    ])
}

pub(crate) fn felyx_over(
    stub: &Stub,
    turns: Vec<ChatTurn>,
) -> (Felyx, Arc<Mutex<Vec<ChatRequest>>>) {
    let registry = ToolRegistry::new(Clients {
        cloud: Some(CloudClient::new(&stub.base, "test-token").expect("cloud client")),
        wardryx: Some(WardryxClient::new(&stub.base, "test-token").expect("wardryx client")),
        idryx: Some(IdryxClient::new(&stub.base).expect("idryx client")),
        ..Default::default()
    });
    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider = Script {
        turns: Mutex::new(turns),
        seen: seen.clone(),
    };
    (Felyx::new(Box::new(provider), registry, 8, 512), seen)
}

pub(crate) fn propose(name: &str, arguments: Value) -> ChatTurn {
    ChatTurn {
        content: None,
        tool_calls: vec![ToolCall {
            id: format!("p-{name}"),
            name: name.to_string(),
            arguments,
        }],
        usage: Usage::default(),
    }
}

async fn ask(turns: Vec<ChatTurn>, question: &str) -> (Answer, usize, String) {
    let stub = plane();
    let (felyx, seen) = felyx_over(&stub, turns);
    let answer = felyx.answer(question).await.unwrap();
    let seen = seen.lock().unwrap();
    let last = seen.last().map(last_user_message).unwrap_or_default();
    (answer, seen.len(), last)
}

// ---- ids --------------------------------------------------------------------

#[tokio::test]
async fn an_id_a_tool_returned_costs_no_extra_call() {
    let text = "Run `p1-brume` hit incident `budget_exhausted:p1-brume` at $0.002008.";
    let (answer, turns, _) = ask(
        vec![calls(&["incidents", "list_runs"]), says(text)],
        "what happened?",
    )
    .await;
    assert_eq!(answer.text, text);
    assert!(answer.unsupported_claims.is_empty());
    assert_eq!(turns, 2);
}

#[tokio::test]
async fn an_invented_run_id_is_sent_back_once() {
    let wrong = "The spike came from run `p1-phantom`.";
    let right = "Run `p1-brume` is the run that spiked.";
    let (answer, turns, feedback) = ask(
        vec![calls(&["list_runs"]), says(wrong), says(right)],
        "what happened?",
    )
    .await;
    assert_eq!(answer.text, right);
    assert_eq!(turns, 3, "exactly one revision");
    assert!(feedback.contains("`p1-phantom`"), "{feedback}");
    assert!(feedback.contains("copied exactly"), "{feedback}");
    assert!(answer.unsupported_claims.is_empty());
}

#[tokio::test]
async fn an_invented_run_id_that_stays_is_taken_out_and_named_in_a_note() {
    let draft = "Run `p1-brume` is over. The spike came from run `p1-phantom`.";
    let (answer, turns, _) = ask(
        vec![calls(&["list_runs"]), says(draft), says(draft)],
        "what happened?",
    )
    .await;
    assert_eq!(turns, 3, "the second draft is final");
    assert!(answer.text.starts_with("Run `p1-brume` is over."));
    assert!(
        !answer.text.contains("The spike came from"),
        "the statement is gone: {}",
        answer.text
    );
    assert!(
        answer.text.contains("statements were removed"),
        "{}",
        answer.text
    );
    assert!(answer.text.contains("`p1-phantom`"), "{}", answer.text);
    assert_eq!(
        answer.unsupported_claims.len(),
        1,
        "{:?}",
        answer.unsupported_claims
    );
    assert!(answer.unsupported_claims[0].contains("no tool returned"));
}

#[tokio::test]
async fn an_answer_with_nothing_left_is_the_note_alone() {
    let draft = "The spike came from run `p1-phantom`.";
    let (answer, _, _) = ask(
        vec![calls(&["list_runs"]), says(draft), says(draft)],
        "what happened?",
    )
    .await;
    assert!(answer.text.starts_with("Check: "), "{:?}", answer.text);
}

#[tokio::test]
async fn an_invented_policy_agent_or_approval_id_is_sent_back_and_a_real_one_is_not() {
    let wrong =
        "Policy `deny-phantom` holds approval `ap-78` for agent://taipanbox.dev/routers/ghost.";
    let right =
        "Policy `deny-shell` holds approval `ap-77` for agent://taipanbox.dev/routers/brume.";
    let (answer, turns, feedback) = ask(
        vec![
            calls(&["policies", "approvals_inbox", "identity_alerts"]),
            says(wrong),
            says(right),
        ],
        "who holds it?",
    )
    .await;
    assert_eq!(answer.text, right);
    assert_eq!(turns, 3);
    for token in ["`deny-phantom`", "`ap-78`", "routers/ghost"] {
        assert!(feedback.contains(token), "{token}: {feedback}");
    }
    assert!(!feedback.contains("`deny-shell`"), "{feedback}");
}

#[tokio::test]
async fn saying_no_tool_returned_an_id_is_the_honest_answer_and_passes() {
    let text = "No tool returned run `p1-phantom`.";
    let (answer, turns, _) = ask(
        vec![calls(&["list_runs"]), says(text)],
        "what about p1-phantom?",
    )
    .await;
    assert_eq!(answer.text, text);
    assert_eq!(turns, 2);
}

// ---- amounts ----------------------------------------------------------------

#[tokio::test]
async fn an_amount_a_tool_returned_passes_rounded_and_an_invented_one_is_taken_out() {
    let rounded = "Run `p1-brume` spent $0.0020.";
    let (answer, turns, _) = ask(vec![calls(&["list_runs"]), says(rounded)], "spend?").await;
    assert_eq!(answer.text, rounded);
    assert_eq!(turns, 2);

    let draft = "Run `p1-brume` spent $0.0020. It would have reached $0.0300 by noon.";
    let (answer, _, feedback) = ask(
        vec![calls(&["list_runs"]), says(draft), says(draft)],
        "spend?",
    )
    .await;
    assert!(feedback.contains("$0.0300"), "{feedback}");
    assert!(!answer.text.contains("by noon"), "{}", answer.text);
    assert!(answer.text.starts_with("Run `p1-brume` spent $0.0020."));
    assert_eq!(answer.unsupported_claims.len(), 1);
}

// ---- explain_incident -------------------------------------------------------

#[tokio::test]
async fn an_explanation_of_an_incident_no_tool_returned_gives_no_chain() {
    let chain =
        "Run `p1-brume` looped -> the spend spiked -> `budget_exhausted:p1-phantom` opened.";
    let stub = plane();
    let (felyx, seen) = felyx_over(&stub, vec![calls(&["incidents"]), says(chain), says(chain)]);
    let answer = felyx
        .explain_incident("budget_exhausted:p1-phantom")
        .await
        .unwrap();
    assert!(!answer.text.contains("looped"), "{}", answer.text);
    assert!(
        answer.text.starts_with("No root-cause chain is given"),
        "{}",
        answer.text
    );
    assert!(
        answer
            .unsupported_claims
            .iter()
            .any(|c| c.contains("explains incident `budget_exhausted:p1-phantom`")),
        "{:?}",
        answer.unsupported_claims
    );
    let feedback = last_user_message(seen.lock().unwrap().last().unwrap());
    assert!(feedback.contains("explains incident"), "{feedback}");
}

#[tokio::test]
async fn an_explanation_that_never_read_its_incident_gives_no_chain() {
    let chain = "Run `p1-brume` looped -> the spend spiked.";
    let stub = plane();
    let (felyx, _) = felyx_over(&stub, vec![calls(&["list_runs"]), says(chain), says(chain)]);
    let answer = felyx.explain_incident(INCIDENT).await.unwrap();
    assert!(
        answer.text.starts_with("No root-cause chain is given"),
        "{}",
        answer.text
    );
}

#[tokio::test]
async fn an_explanation_citing_an_invented_run_gives_no_chain_at_all() {
    let chain = "Run `p1-brume` spiked.\nRun `p1-phantom` started the loop.\nThe spend rose.";
    let stub = plane();
    let (felyx, _) = felyx_over(
        &stub,
        vec![calls(&["incidents", "list_runs"]), says(chain), says(chain)],
    );
    let answer = felyx.explain_incident(INCIDENT).await.unwrap();
    assert!(
        !answer.text.contains("spiked"),
        "a chain with a link taken out reads as a shorter chain: {}",
        answer.text
    );
    assert!(
        answer.text.starts_with("No root-cause chain is given"),
        "{}",
        answer.text
    );
    assert_eq!(
        answer.unsupported_claims.len(),
        1,
        "{:?}",
        answer.unsupported_claims
    );
}

#[tokio::test]
async fn an_explanation_resting_on_what_the_tools_returned_keeps_its_chain() {
    let chain = "Run `p1-brume` spiked -> incident `budget_exhausted:p1-brume` opened. \
                 One action: review run `p1-brume`.";
    let stub = plane();
    let (felyx, seen) = felyx_over(&stub, vec![calls(&["incidents", "list_runs"]), says(chain)]);
    let answer = felyx.explain_incident(INCIDENT).await.unwrap();
    assert_eq!(answer.text, chain);
    assert!(answer.unsupported_claims.is_empty());
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn cards_on_an_explanation_of_no_incident_are_withheld() {
    let chain = "Run `p1-brume` looped.";
    let stub = plane();
    let (felyx, _) = felyx_over(
        &stub,
        vec![
            calls(&["list_runs"]),
            propose(
                "propose_kill",
                json!({"run_id": "p1-brume", "reason": "it looped"}),
            ),
            says(chain),
            says(chain),
        ],
    );
    let answer = felyx
        .explain_incident("budget_exhausted:p1-phantom")
        .await
        .unwrap();
    assert!(answer.proposals.is_empty(), "{:?}", answer.proposals);
    assert_eq!(answer.withheld_proposals.len(), 1);
    assert!(answer.withheld_proposals[0].reason.contains("incident"));
}

// ---- cards ------------------------------------------------------------------

#[tokio::test]
async fn a_card_on_a_run_a_tool_returned_reaches_the_shell() {
    let (answer, _, _) = ask(
        vec![
            calls(&["list_runs"]),
            propose(
                "propose_kill",
                json!({"run_id": "p1-brume", "reason": "over its cap",
                       "evidence_refs": ["run:p1-brume"]}),
            ),
            says("I proposed stopping it, for your approval."),
        ],
        "what should happen to it?",
    )
    .await;
    assert_eq!(answer.proposals.len(), 1, "{:?}", answer.withheld_proposals);
    assert_eq!(answer.proposals[0].kind, ActionKind::Kill);
    assert_eq!(answer.proposals[0].evidence_refs, vec!["run:p1-brume"]);
    assert!(answer.withheld_proposals.is_empty());
}

#[tokio::test]
async fn a_card_on_a_run_no_tool_returned_is_withheld_and_the_model_is_told() {
    let stub = plane();
    let (felyx, seen) = felyx_over(
        &stub,
        vec![
            calls(&["list_runs"]),
            propose(
                "propose_kill",
                json!({"run_id": "p1-phantom", "reason": "looks like a runaway"}),
            ),
            says("The card could not be raised."),
        ],
    );
    let answer = felyx.answer("what should happen?").await.unwrap();
    assert!(answer.proposals.is_empty(), "{:?}", answer.proposals);
    assert_eq!(answer.withheld_proposals.len(), 1);
    assert_eq!(answer.withheld_proposals[0].action.target, "p1-phantom");
    assert!(
        answer.withheld_proposals[0]
            .reason
            .contains("no tool returned")
    );
    let propose_row = answer.tool_trace.iter().find(|t| t.name == "propose_kill");
    assert!(!propose_row.expect("in the trace").ok);
    let seen = seen.lock().unwrap();
    let told = seen[2]
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Tool)
        .map(|m| m.content.clone())
        .unwrap_or_default();
    assert!(told.contains("\"withheld\":true"), "{told}");
}

#[tokio::test]
async fn evidence_no_tool_returned_leaves_the_card_and_is_named() {
    let (answer, _, _) = ask(
        vec![
            calls(&["list_runs"]),
            propose(
                "propose_kill",
                json!({"run_id": "p1-brume", "reason": "over its cap",
                       "evidence_refs": ["run:p1-brume", "incident:budget_exhausted:p1-phantom"]}),
            ),
            says("I proposed it."),
        ],
        "and then?",
    )
    .await;
    assert_eq!(answer.proposals[0].evidence_refs, vec!["run:p1-brume"]);
    assert_eq!(
        answer.unsupported_claims.len(),
        1,
        "{:?}",
        answer.unsupported_claims
    );
    assert!(answer.unsupported_claims[0].contains("cites evidence"));
}
