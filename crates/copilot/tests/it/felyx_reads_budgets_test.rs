//! Felyx states a budget only when a tool returned it.
//!
//! The defect, measured 2026-10-05 on the forge lab (genaryx-console v1.1.19,
//! Haiku 4.5 through the stack's own gateway): asked about the home router
//! agents, Felyx said "p1-beryl2 exceeded budget alert (spent $0.004019 vs
//! $0.001 cap)" although no budget exists on any beryl2 run (the $0.001 cap
//! was p1-brume's), and asked which runs have a budget it called mig-flint
//! "no budget" although the Cloud's `GET /v1/budgets` held 4500 uUSD for it
//! (spent 3210). Its tool trace was `list_agents`, `list_runs`, `alerts`,
//! `incidents`: there was no budgets tool, so it read budgets off `alerts`,
//! which lists a run only once it is near or over its limit.
//!
//! The fixture below is that Cloud, the same run ids and figures, read back
//! from the admin API the same minute (the evidence log appends it).
//!
//! HAND-ROLLED stub server, the same approach `money_reaches_the_model_as_usd_test.rs`
//! takes, but routing by path, because the `budgets` tool joins several reads.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use genaryx_connectors::CloudClient;
use genaryx_copilot::{
    ChatRequest, ChatTurn, Clients, Felyx, LlmProvider, ProviderDescriptor, ProviderError, Role,
    ToolCall, ToolRegistry, Usage,
};
use serde_json::{Value, json};

// ---- the forge Cloud, 2026-10-05 -------------------------------------------

const BUDGETS: &str =
    r#"{"genaryx-copilot":100000,"mig-flint":4500,"p1-brume":1000,"mig-brume":1278}"#;

fn run(run_id: &str, agent: &str, spent: i64, killed: bool) -> Value {
    json!({
        "run_id": run_id, "model": "claude-haiku-4-5", "agent_id": agent,
        "spent_microusd": spent, "calls": 1, "cache_hits": 0, "steps": 1,
        "last_seen_millis": 0, "killed": killed,
    })
}

fn runs_body() -> String {
    let r = "agent://taipanbox.dev/routers";
    json!([
        run("p1-beryl2", &format!("{r}/beryl2"), 4019, false),
        run("mig-flint", &format!("{r}/flint"), 3210, false),
        run("p1-flint", &format!("{r}/flint"), 2625, true),
        run("p1-brume", &format!("{r}/brume"), 2008, false),
        run("p1-flint-2", &format!("{r}/flint"), 1288, false),
        run("mig-brume", &format!("{r}/brume"), 1278, false),
        run("mig-beryl2", &format!("{r}/beryl2"), 1278, false),
        run("p2-adk-local", &format!("{r}/beryl1"), 37, false),
        run(
            "genaryx-copilot",
            "agent://local/genaryx/felyx",
            92576,
            false
        ),
    ])
    .to_string()
}

const ALERTS: &str = r#"[
  {"run_id":"p1-brume","spent_microusd":2008,"budget_micros":1000,"fraction":2.008,"killed":false},
  {"run_id":"mig-brume","spent_microusd":1278,"budget_micros":1278,"fraction":1.0,"killed":false}
]"#;

/// One stub Cloud: path -> body, anything else 404. Records every
/// `METHOD path` it was asked for.
struct Stub {
    base: String,
    seen: Arc<Mutex<Vec<String>>>,
}

fn spawn_cloud(routes: Vec<(&'static str, String)>) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
    let base = format!("http://{}", listener.local_addr().unwrap());
    let routes: HashMap<&'static str, String> = routes.into_iter().collect();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_bg = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            serve(stream, &routes, &seen_bg);
        }
    });
    Stub { base, seen }
}

fn serve(stream: TcpStream, routes: &HashMap<&'static str, String>, seen: &Mutex<Vec<String>>) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let path = target.split('?').next().unwrap_or("").to_string();
    let mut content_length = 0usize;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).is_err() {
            return;
        }
        let t = h.trim_end_matches(['\r', '\n']);
        if t.is_empty() {
            break;
        }
        if let Some((n, v)) = t.split_once(':')
            && n.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).ok();
    }
    seen.lock().unwrap().push(format!("{method} {path}"));
    let (status, out) = match routes.get(path.as_str()) {
        Some(b) => ("200 OK", b.clone()),
        None => ("404 Not Found", r#"{"error":"not found"}"#.to_string()),
    };
    let mut w = reader.into_inner();
    let _ = write!(
        w,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
        out.len()
    );
    let _ = w.flush();
}

fn forge_cloud() -> Stub {
    spawn_cloud(vec![
        ("/v1/budgets", BUDGETS.to_string()),
        ("/v1/runs", runs_body()),
        ("/v1/alerts", ALERTS.to_string()),
        ("/v1/unit-budgets", "{}".to_string()),
        ("/v1/units", "[]".to_string()),
    ])
}

fn registry(base: &str) -> ToolRegistry {
    ToolRegistry::new(Clients {
        cloud: Some(CloudClient::new(base, "test-token").expect("client")),
        ..Default::default()
    })
}

async fn budgets_result(base: &str) -> Value {
    registry(base)
        .dispatch("budgets", &json!({}))
        .await
        .unwrap_or_else(|e| panic!("budgets dispatch failed: {e}"))
}

fn row<'a>(result: &'a Value, run_id: &str) -> Option<&'a Value> {
    result["run_budgets"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["run_id"] == run_id))
}

// ---- the tool ---------------------------------------------------------------

#[tokio::test]
async fn a_run_with_a_budget_and_no_alert_is_listed_with_its_budget_and_spend() {
    let cloud = forge_cloud();
    let result = budgets_result(&cloud.base).await;
    let flint = row(&result, "mig-flint")
        .unwrap_or_else(|| panic!("mig-flint has a Cloud budget and must be listed: {result}"));
    assert_eq!(flint["budget_usd"], json!(0.0045));
    assert_eq!(flint["spent_usd"], json!(0.00321));
    assert_eq!(flint["at_or_over"], json!(false));
    assert_eq!(
        flint["agent_id"],
        json!("agent://taipanbox.dev/routers/flint")
    );
    assert_eq!(result["run_budgets_total"], json!(4));
}

#[tokio::test]
async fn a_cap_set_on_one_run_is_never_reported_on_another() {
    let cloud = forge_cloud();
    let result = budgets_result(&cloud.base).await;
    assert!(
        row(&result, "p1-beryl2").is_none(),
        "p1-beryl2 has no budget on the Cloud and must not appear: {result}"
    );
    assert!(row(&result, "mig-beryl2").is_none());
    let none: Vec<&str> = result["runs_without_budget"]
        .as_array()
        .expect("runs_without_budget")
        .iter()
        .filter_map(|r| r["run_id"].as_str())
        .collect();
    assert!(
        none.contains(&"p1-beryl2"),
        "p1-beryl2 is listed as having none: {none:?}"
    );
    assert!(!none.contains(&"mig-flint"), "{none:?}");
    assert_eq!(result["runs_without_budget_total"], json!(5));
    let brume = row(&result, "p1-brume").expect("p1-brume listed");
    assert_eq!(brume["budget_usd"], json!(0.001));
    assert_eq!(brume["spent_usd"], json!(0.002008));
    assert_eq!(brume["at_or_over"], json!(true));
    let mig = row(&result, "mig-brume").expect("mig-brume listed");
    assert_eq!(
        mig["at_or_over"],
        json!(true),
        "spent equal to budget is at it"
    );
}

#[tokio::test]
async fn the_tool_says_an_absent_run_has_no_control_plane_budget_and_what_it_cannot_see() {
    let cloud = forge_cloud();
    let result = budgets_result(&cloud.base).await;
    let scope = result["scope"].as_str().expect("scope sentence");
    assert!(
        scope.contains("not listed here has no budget on the control plane"),
        "{scope}"
    );
    assert!(
        scope.contains("gateway"),
        "names the gateway-local budgets it cannot see: {scope}"
    );
    assert_eq!(result["complete"], json!(true));
}

#[tokio::test]
async fn a_budget_with_no_spend_record_says_spent_is_unknown_not_zero() {
    let cloud = spawn_cloud(vec![
        ("/v1/budgets", r#"{"never-ran":5000}"#.to_string()),
        ("/v1/runs", "[]".to_string()),
        ("/v1/unit-budgets", "{}".to_string()),
        ("/v1/units", "[]".to_string()),
    ]);
    let result = budgets_result(&cloud.base).await;
    let r = row(&result, "never-ran").expect("listed");
    assert_eq!(r["budget_usd"], json!(0.005));
    assert_eq!(
        r["spent_usd"],
        json!(null),
        "no record is not zero spend: {r}"
    );
    assert_eq!(r["at_or_over"], json!(null));
    assert_eq!(r["agent_id"], json!(null));
}

#[tokio::test]
async fn unit_budgets_carry_their_month_to_date_spend() {
    let cloud = spawn_cloud(vec![
        ("/v1/budgets", "{}".to_string()),
        ("/v1/runs", "[]".to_string()),
        ("/v1/unit-budgets", r#"{"routers":20000}"#.to_string()),
        (
            "/v1/units",
            r#"[{"unit":"routers","spent_microusd":90000,"calls":40,"runs":9,
                 "last_seen_millis":0,"tool_calls":0,"month":"2026-10",
                 "month_spent_microusd":12500,"month_calls":12}]"#
                .to_string(),
        ),
    ]);
    let result = budgets_result(&cloud.base).await;
    let u = &result["unit_budgets"][0];
    assert_eq!(u["unit"], json!("routers"));
    assert_eq!(u["budget_usd"], json!(0.02));
    assert_eq!(u["month"], json!("2026-10"));
    assert_eq!(u["month_spent_usd"], json!(0.0125));
    assert_eq!(u["at_or_over"], json!(false));
}

#[tokio::test]
async fn an_older_cloud_without_unit_budgets_says_so_rather_than_showing_none() {
    let cloud = spawn_cloud(vec![
        ("/v1/budgets", BUDGETS.to_string()),
        ("/v1/runs", runs_body()),
    ]);
    let result = budgets_result(&cloud.base).await;
    assert_eq!(
        result["unit_budgets"],
        json!(null),
        "unknown is not empty: {result}"
    );
    assert!(
        result["unit_budgets_unavailable"]
            .as_str()
            .is_some_and(|s| s.contains("/v1/unit-budgets")),
        "{result}"
    );
    assert!(
        row(&result, "mig-flint").is_some(),
        "run budgets still read"
    );
}

#[tokio::test]
async fn the_budgets_tool_only_reads() {
    let cloud = forge_cloud();
    let reg = registry(&cloud.base);
    assert!(reg.tool_names().contains(&"budgets"));
    assert!(!reg.is_propose_tool("budgets"));
    let _ = reg.dispatch("budgets", &json!({})).await.expect("dispatch");
    let seen = cloud.seen.lock().unwrap().clone();
    assert!(!seen.is_empty());
    assert!(
        seen.iter().all(|s| s.starts_with("GET ")),
        "the budgets tool must only issue GETs: {seen:?}"
    );
}

// ---- the loop: no budget fact without a tool behind it ------------------

/// A scripted provider, one turn per call, keeping every request it saw.
struct Script {
    turns: Mutex<Vec<ChatTurn>>,
    seen: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait]
impl LlmProvider for Script {
    async fn chat(&self, req: ChatRequest) -> Result<ChatTurn, ProviderError> {
        self.seen.lock().unwrap().push(req);
        let mut t = self.turns.lock().unwrap();
        if t.is_empty() {
            return Err(ProviderError::Decode("script ran out of turns".into()));
        }
        Ok(t.remove(0))
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

fn calls(names: &[&str]) -> ChatTurn {
    ChatTurn {
        content: None,
        tool_calls: names
            .iter()
            .enumerate()
            .map(|(i, n)| ToolCall {
                id: format!("c{i}-{n}"),
                name: (*n).to_string(),
                arguments: json!({}),
            })
            .collect(),
        usage: Usage::default(),
    }
}

fn says(text: &str) -> ChatTurn {
    ChatTurn {
        content: Some(text.to_string()),
        tool_calls: vec![],
        usage: Usage::default(),
    }
}

fn felyx(base: &str, turns: Vec<ChatTurn>) -> (Felyx, Arc<Mutex<Vec<ChatRequest>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider = Script {
        turns: Mutex::new(turns),
        seen: seen.clone(),
    };
    (Felyx::new(Box::new(provider), registry(base), 8, 512), seen)
}

/// The last user message the loop sent, which is where a revision request
/// lands.
fn last_user_message(req: &ChatRequest) -> String {
    req.messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .map(|m| m.content.clone())
        .unwrap_or_default()
}

const Q1_WRONG: &str = "**beryl2**: p1-beryl2 ($0.004019), mig-beryl2 ($0.001278)\n\
Status: p1-beryl2 exceeded budget alert (spent $0.004019 vs $0.001 cap) and stalled\n\
**brume**: p1-brume exceeded budget alert (spent $0.002008 vs $0.001 cap); mig-brume at budget threshold";

const Q1_RIGHT: &str = "**beryl2**: p1-beryl2 ($0.004019), mig-beryl2 ($0.001278); no budget set on the control plane.\n\
**brume**: p1-brume is over its budget ($0.002008 of $0.001); mig-brume is at its budget ($0.001278).";

#[tokio::test]
async fn a_budget_stated_for_a_run_no_tool_gave_one_is_sent_back_for_revision() {
    let cloud = forge_cloud();
    let (felyx, seen) = felyx(
        &cloud.base,
        vec![
            calls(&["list_runs", "alerts", "budgets"]),
            says(Q1_WRONG),
            says(Q1_RIGHT),
        ],
    );
    let answer = felyx.answer("router fleet, over budget?").await.unwrap();
    assert_eq!(
        answer.text, Q1_RIGHT,
        "the revised answer is the one returned"
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3, "one revision turn, no more");
    let feedback = last_user_message(&seen[2]);
    assert!(feedback.contains("p1-beryl2"), "names the run: {feedback}");
    assert!(
        !feedback.contains("p1-brume"),
        "p1-brume's budget was returned by a tool and must not be flagged: {feedback}"
    );
}

#[tokio::test]
async fn a_budgeted_run_called_unbudgeted_is_sent_back_for_revision() {
    let cloud = forge_cloud();
    let q2_wrong = "Only 2 runs have budgets set: p1-brume (over) and mig-brume (at).\n\
The other router runs (p1-beryl2, mig-flint, p1-flint) do not have budgets set.";
    let q2_right = "Three router runs have a budget: p1-brume, mig-brume and mig-flint.";
    let (felyx, seen) = felyx(
        &cloud.base,
        vec![calls(&["budgets"]), says(q2_wrong), says(q2_right)],
    );
    let answer = felyx.answer("which runs have a budget?").await.unwrap();
    assert_eq!(answer.text, q2_right);
    let seen = seen.lock().unwrap();
    let feedback = last_user_message(&seen[2]);
    assert!(feedback.contains("mig-flint"), "{feedback}");
    assert!(
        !feedback.contains("p1-beryl2"),
        "p1-beryl2 really has none: {feedback}"
    );
}

#[tokio::test]
async fn budget_talk_without_reading_budgets_is_sent_back_to_read_them() {
    let cloud = forge_cloud();
    let (felyx, seen) = felyx(
        &cloud.base,
        vec![
            calls(&["alerts"]),
            says("p1-brume is over its budget ($0.002008 of $0.001)."),
            calls(&["budgets"]),
            says("p1-brume is over its budget ($0.002008 of $0.001)."),
        ],
    );
    let answer = felyx.answer("anything over budget?").await.unwrap();
    assert!(
        answer.tool_trace.iter().any(|t| t.name == "budgets"),
        "{:?}",
        answer.tool_trace
    );
    let seen = seen.lock().unwrap();
    assert!(last_user_message(&seen[2]).contains("`budgets`"));
}

#[tokio::test]
async fn a_claim_that_survives_revision_reaches_the_operator_marked_unsupported() {
    let cloud = forge_cloud();
    let (felyx, _) = felyx(
        &cloud.base,
        vec![calls(&["budgets"]), says(Q1_WRONG), says(Q1_WRONG)],
    );
    let answer = felyx.answer("router fleet").await.unwrap();
    assert!(answer.text.starts_with(Q1_WRONG));
    assert!(
        answer.text.contains("not supported by any tool result")
            && answer.text.contains("p1-beryl2"),
        "the operator must see which statement is unsupported: {}",
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
async fn a_correct_budget_answer_costs_no_extra_call() {
    let cloud = forge_cloud();
    let (felyx, seen) = felyx(
        &cloud.base,
        vec![calls(&["list_runs", "budgets"]), says(Q1_RIGHT)],
    );
    let answer = felyx.answer("router fleet").await.unwrap();
    assert_eq!(answer.text, Q1_RIGHT);
    assert!(answer.unsupported_claims.is_empty());
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn saying_felyx_cannot_change_a_budget_is_not_a_budget_claim() {
    let cloud = forge_cloud();
    let refusal = "I cannot change a budget: a human must approve and sign that.";
    let (felyx, seen) = felyx(&cloud.base, vec![says(refusal)]);
    let answer = felyx.answer("raise p1-brume's budget").await.unwrap();
    assert_eq!(answer.text, refusal);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn the_system_prompt_says_budgets_come_from_the_budgets_tool_not_alerts() {
    let cloud = forge_cloud();
    let (felyx, seen) = felyx(&cloud.base, vec![says("ok")]);
    felyx.answer("hi").await.unwrap();
    let system = seen.lock().unwrap()[0].system.clone();
    assert!(system.contains("`budgets`"), "{system}");
    assert!(system.contains("`alerts`"), "{system}");
}

// ---- the second forge run, 2026-10-05, console v1.1.24 ------------------
//
// With the budgets tool and the run-level check in place, Felyx got every
// run right and still closed with "Flint and brume have budget overages".
// No flint run is over its budget (mig-flint used 71%), and "flint" is an
// agent name, not a run id, so the run-level check never looked at it.

const LIVE_Q1_ROWS: &str = "| **flint** | $0.01325 | mig-flint ($0.0032), p1-flint ($0.0026, **killed**), p1-flint-2 ($0.0013) | p1-flint killed |\n\
| **beryl2** | $0.00658 | p1-beryl2 ($0.0040), mig-beryl2 ($0.0026) | mig-beryl2 stalled; no budget issues |\n\
| **brume** | $0.00329 | p1-brume ($0.0020, **over budget $0.001**), mig-brume ($0.0013, **at budget $0.0013**) | p1-brume exceeded budget 2x; mig-brume at its budget limit |";

const LIVE_Q1_SUMMARY: &str =
    "**Summary:** Flint and brume have budget overages. Flint has a killed run (p1-flint).";

#[tokio::test]
async fn an_agent_said_to_be_over_budget_with_no_run_over_is_sent_back() {
    let cloud = forge_cloud();
    let wrong = format!("{LIVE_Q1_ROWS}\n\n{LIVE_Q1_SUMMARY}");
    let right = format!(
        "{LIVE_Q1_ROWS}\n\n**Summary:** brume has budget overages. Flint has a killed run (p1-flint)."
    );
    let (felyx, seen) = felyx(
        &cloud.base,
        vec![
            calls(&["list_runs", "alerts", "budgets"]),
            says(&wrong),
            says(&right),
        ],
    );
    let answer = felyx.answer("router fleet").await.unwrap();
    assert_eq!(answer.text, right);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3, "exactly one revision");
    let feedback = last_user_message(&seen[2]);
    assert!(feedback.contains("`flint`"), "{feedback}");
    assert!(
        !feedback.contains("`brume`"),
        "brume really has a run over its budget: {feedback}"
    );
}

#[tokio::test]
async fn the_live_forge_rows_alone_raise_nothing() {
    let cloud = forge_cloud();
    let (felyx, seen) = felyx(
        &cloud.base,
        vec![
            calls(&["list_runs", "alerts", "budgets"]),
            says(LIVE_Q1_ROWS),
        ],
    );
    let answer = felyx.answer("router fleet").await.unwrap();
    assert!(
        answer.unsupported_claims.is_empty(),
        "{:?}",
        answer.unsupported_claims
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "a correct answer costs no revision"
    );
}

#[tokio::test]
async fn a_run_said_to_be_over_its_budget_while_under_it_is_sent_back() {
    let cloud = forge_cloud();
    let (felyx, seen) = felyx(
        &cloud.base,
        vec![
            calls(&["budgets"]),
            says("mig-flint is over its budget."),
            says("mig-flint is within its budget (71%)."),
        ],
    );
    let answer = felyx.answer("is mig-flint over?").await.unwrap();
    assert_eq!(answer.text, "mig-flint is within its budget (71%).");
    let feedback = last_user_message(&seen.lock().unwrap()[2]);
    assert!(feedback.contains("`mig-flint`"), "{feedback}");
}

#[tokio::test]
async fn the_revision_request_asks_for_a_fresh_answer_that_does_not_mention_the_check() {
    let cloud = forge_cloud();
    let (felyx, seen) = felyx(
        &cloud.base,
        vec![
            calls(&["budgets"]),
            says("p1-beryl2 is over its $0.001 cap."),
            says("ok"),
        ],
    );
    felyx.answer("router fleet").await.unwrap();
    let feedback = last_user_message(&seen.lock().unwrap()[2]);
    assert!(
        feedback.contains("Do not mention this check"),
        "the operator saw \"You're right. Let me revise\" on forge: {feedback}"
    );
}
