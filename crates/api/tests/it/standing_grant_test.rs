//! A standing grant, driven through the real `policy_grant_tool` against a stub
//! wardryx with a real journal: what wardryx is sent, what it is not sent, and
//! what the console's own chain says happened (CLAUDE.md invariant 47).

use crate::cloud_stub::{Seen, spawn_recording_cloud};
use genaryx_api::policy::PolicyState;
use genaryx_api::policy::commands::{PolicyError, policy_grant_tool};
use genaryx_api::policy::env::EnvSource;
use genaryx_api::policy::state::{BusHandle, PolicyClient, PolicyInner};
use genaryx_connectors::WardryxClient;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const AGENT: &str = "agent://acme.example/agents/bot-1";
const POLICY_ROUTE: &str = "/v1/policies/grant-acme.example-agents-bot-1-2ffe7f24";
const PUT_ROUTE: &str = "PUT /v1/policies/grant-acme.example-agents-bot-1-2ffe7f24";

fn approvals(agent: &str, context: Value) -> String {
    json!([{
        "approval_id": "ap_1",
        "agent_id": agent,
        "run_id": "run-1",
        "requested_at": "2026-11-07T10:00:00Z",
        "pending": true,
        "context": context,
    }])
    .to_string()
}

fn floor(source: &str) -> Value {
    json!({
        "hold_kind": "no_grant",
        "identity_source": source,
        "tool_names": ["ask"],
        "tool_call": {"name": "ask", "target": "typryx"},
    })
}

fn accepted() -> String {
    json!({"id": "grant-acme.example-agents-bot-1-2ffe7f24", "target": AGENT}).to_string()
}

fn scratch_bus() -> (PathBuf, BusHandle) {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-standing-grant-itest-{}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let bus = BusHandle::from_dirs(&dir, &dir);
    (dir, bus)
}

async fn policy_at(base: &str, bus: Option<BusHandle>) -> PolicyState {
    let state = PolicyState::pending();
    *state.inner.lock().await = PolicyInner::Ready(PolicyClient {
        client: Arc::new(WardryxClient::new(base, "test-token").expect("wardryx client")),
        source: EnvSource::EnvFallback,
        wardryx_url: base.to_string(),
        org_domain: "wardryx.local".to_string(),
        operator: "user://wardryx.local/test".to_string(),
        host: "test".to_string(),
        sig_alg: "none",
        sig_fpr: "bearer-admin",
        bus,
    });
    state
}

fn sent(log: &Arc<Mutex<Vec<Seen>>>) -> Vec<Seen> {
    log.lock().expect("the stub's log").clone()
}

fn the_put(log: &Arc<Mutex<Vec<Seen>>>) -> Value {
    let seen = sent(log);
    let put = seen
        .iter()
        .find(|s| s.method == "PUT")
        .expect("a PUT reached wardryx");
    assert_eq!(put.path, POLICY_ROUTE);
    serde_json::from_str(&put.body).expect("a JSON body")
}

fn no_put_reached_wardryx(log: &Arc<Mutex<Vec<Seen>>>) {
    assert!(
        !sent(log).iter().any(|s| s.method == "PUT"),
        "a refused grant reached wardryx"
    );
}

/// The console's own journal, as text.
fn journal_of(dir: &Path) -> String {
    let mut all = String::new();
    for entry in std::fs::read_dir(dir).expect("read dir").flatten() {
        if let Ok(text) = std::fs::read_to_string(entry.path()) {
            all.push_str(&text);
        }
    }
    all
}

#[tokio::test]
async fn a_grant_writes_the_policy_agent_allow_writes() {
    let (base, log) = spawn_recording_cloud(vec![
        (
            "/v1/approvals",
            "200 OK",
            approvals(AGENT, floor("agent-token")),
        ),
        (PUT_ROUTE, "200 OK", accepted()),
    ]);
    let state = policy_at(&base, None).await;
    let outcome = policy_grant_tool("ap_1".to_string(), &state)
        .await
        .expect("granted");
    assert_eq!(outcome.agent_id, AGENT);
    assert_eq!(outcome.tools, ["ask"]);

    let body = the_put(&log);
    assert_eq!(body["target"], AGENT);
    assert_eq!(body["grant_tool"], json!(["ask"]));
    assert_eq!(
        body["require_identity_source"],
        json!(["agent-token", "none"])
    );
    assert_eq!(body["deny_if_chain_unproven"], true);
    let name = body["name"].as_str().expect("a name");
    assert!(name.starts_with(&format!("grant {AGENT} ")), "{name}");
    assert!(name.contains("from hold ap_1"), "{name}");
}

#[tokio::test]
async fn a_grant_adds_to_the_policy_the_agent_already_holds() {
    let held = json!({
        "id": "grant-acme.example-agents-bot-1-2ffe7f24",
        "target": AGENT,
        "grant_tool": ["fetch_url"],
        "require_identity_source": ["agent-token", "client-key", "none"],
    })
    .to_string();
    let (base, log) = spawn_recording_cloud(vec![
        (
            "/v1/approvals",
            "200 OK",
            approvals(AGENT, floor("agent-token")),
        ),
        (POLICY_ROUTE, "200 OK", held),
    ]);
    let state = policy_at(&base, None).await;
    policy_grant_tool("ap_1".to_string(), &state)
        .await
        .expect("granted");
    let body = the_put(&log);
    assert_eq!(body["grant_tool"], json!(["ask", "fetch_url"]));
    assert_eq!(
        body["require_identity_source"],
        json!(["agent-token", "client-key", "none"])
    );
}

#[tokio::test]
async fn a_hold_that_came_with_a_static_key_is_not_turned_into_a_refusal() {
    let (base, log) = spawn_recording_cloud(vec![
        (
            "/v1/approvals",
            "200 OK",
            approvals(AGENT, floor("client-key")),
        ),
        (PUT_ROUTE, "200 OK", accepted()),
    ]);
    let state = policy_at(&base, None).await;
    policy_grant_tool("ap_1".to_string(), &state)
        .await
        .expect("granted");
    assert_eq!(
        the_put(&log)["require_identity_source"],
        json!(["agent-token", "client-key", "none"])
    );
}

#[tokio::test]
async fn an_agent_with_no_trust_domain_reaches_no_policy_and_is_journaled() {
    let (base, log) = spawn_recording_cloud(vec![(
        "/v1/approvals",
        "200 OK",
        approvals("agent-42", floor("agent-token")),
    )]);
    let (dir, bus) = scratch_bus();
    let state = policy_at(&base, Some(bus)).await;
    let err = policy_grant_tool("ap_1".to_string(), &state)
        .await
        .expect_err("refused");
    match err {
        PolicyError::StandingGrant { reason } => {
            assert!(reason.contains("names no trust domain"), "{reason}");
        }
        other => panic!("refused the wrong way: {other:?}"),
    }
    no_put_reached_wardryx(&log);
    let journal = journal_of(&dir);
    assert!(journal.contains("console.grant_tool"), "{journal}");
    assert!(journal.contains("refused"), "{journal}");
}

#[tokio::test]
async fn a_hold_of_another_kind_reaches_no_policy() {
    let (base, log) = spawn_recording_cloud(vec![(
        "/v1/approvals",
        "200 OK",
        approvals(AGENT, json!({"tool_names": ["ask"]})),
    )]);
    let state = policy_at(&base, None).await;
    let err = policy_grant_tool("ap_1".to_string(), &state)
        .await
        .expect_err("refused");
    assert!(matches!(err, PolicyError::StandingGrant { .. }), "{err:?}");
    no_put_reached_wardryx(&log);
}

#[tokio::test]
async fn a_hold_wardryx_does_not_have_reaches_no_policy() {
    let (base, log) = spawn_recording_cloud(vec![(
        "/v1/approvals",
        "200 OK",
        approvals(AGENT, floor("agent-token")),
    )]);
    let state = policy_at(&base, None).await;
    let err = policy_grant_tool("ap_nobody".to_string(), &state)
        .await
        .expect_err("refused");
    assert!(matches!(err, PolicyError::StandingGrant { .. }), "{err:?}");
    no_put_reached_wardryx(&log);
}

#[tokio::test]
async fn a_policy_wardryx_will_not_read_writes_nothing_over_it() {
    let (base, log) = spawn_recording_cloud(vec![
        (
            "/v1/approvals",
            "200 OK",
            approvals(AGENT, floor("agent-token")),
        ),
        (POLICY_ROUTE, "500 Internal Server Error", "{}".to_string()),
    ]);
    let state = policy_at(&base, None).await;
    let err = policy_grant_tool("ap_1".to_string(), &state)
        .await
        .expect_err("refused");
    assert!(matches!(err, PolicyError::Wardryx { .. }), "{err:?}");
    no_put_reached_wardryx(&log);
}

#[tokio::test]
async fn a_grant_is_on_the_consoles_own_chain_with_its_agent_and_tool() {
    let (base, _log) = spawn_recording_cloud(vec![
        (
            "/v1/approvals",
            "200 OK",
            approvals(AGENT, floor("agent-token")),
        ),
        (PUT_ROUTE, "200 OK", accepted()),
    ]);
    let (dir, bus) = scratch_bus();
    let state = policy_at(&base, Some(bus)).await;
    let outcome = policy_grant_tool("ap_1".to_string(), &state)
        .await
        .expect("granted");
    assert!(outcome.bus_recorded, "{:?}", outcome.bus_error);
    let journal = journal_of(&dir);
    assert!(journal.contains("console.grant_tool"), "{journal}");
    assert!(journal.contains(AGENT), "{journal}");
    assert!(journal.contains("ap_1"), "{journal}");
}
