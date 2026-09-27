//! Integration proof for the 2026-09-27 defect fix: every money-carrying
//! copilot tool must hand the model a decimal `_usd` amount, never a bare
//! micro-USD integer, when reached through the REAL `Tool::run` dispatch
//! (`ToolRegistry::dispatch`), not just the pure `dollarize` composition
//! `crates/copilot/src/tools/{cloud,optimize}.rs`'s own unit tests exercise.
//!
//! That distinction is the point: a mutant that dropped `dollarize` from the
//! `read_tool!` macro (`crates/copilot/src/tools/cloud.rs`) passed every one
//! of this crate's existing in-module tests, because those call `to_result`
//! and `dollarize` directly rather than going through the tool the model
//! actually calls. Only a test that drives the same `ToolRegistry::dispatch`
//! path the agent loop uses (`crates/copilot/src/agent.rs`) can catch a
//! macro losing its own conversion step.
//!
//! HAND-ROLLED stub server, same shape `crates/copilot/tests/
//! agent_id_header_test.rs` and `crates/api/tests/delegation_revoke_test.rs`
//! already use: this proves GENARYX's own tool dispatch, never a real
//! Cloud's correctness.

use genaryx_connectors::CloudClient;
use genaryx_copilot::{Clients, ToolRegistry};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

/// Read one HTTP/1.1 request off `stream` and answer it with a fixed 200 +
/// `body`, regardless of path - each stub in this file only ever serves the
/// one route its test calls.
fn serve_one(stream: TcpStream, body: &'static str) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));

    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .expect("read request line");

    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("read header line");
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut discard = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut discard).expect("read body");
    }

    let body_bytes = body.as_bytes();
    let mut out = reader.into_inner();
    write!(
        out,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body_bytes.len()
    )
    .expect("write response head");
    out.write_all(body_bytes).expect("write response body");
    out.flush().ok();
}

/// Spawn a stub bound to `127.0.0.1:0` that answers exactly one connection
/// with a fixed 200/`body`, and hand back its base URL.
fn spawn_stub(body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub listener");
    let addr = listener.local_addr().expect("stub local addr");
    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            serve_one(stream, body);
        }
    });
    format!("http://{addr}")
}

fn registry_against(base_url: &str) -> ToolRegistry {
    let cloud = CloudClient::new(base_url, "test-token").expect("build CloudClient");
    ToolRegistry::new(Clients {
        cloud: Some(cloud),
        ..Default::default()
    })
}

async fn dispatch(base_url: &str, tool: &str) -> Value {
    registry_against(base_url)
        .dispatch(tool, &json!({}))
        .await
        .unwrap_or_else(|e| panic!("{tool} dispatch failed: {e}"))
}

#[tokio::test]
async fn money_summary_reaches_the_model_as_a_decimal_usd_field() {
    let base = spawn_stub(r#"{"runs":16,"calls":91,"spent_microusd":167964}"#);
    let result = dispatch(&base, "money_summary").await;
    assert_eq!(result["spent_usd"], json!(0.167_964));
    assert!(
        result.get("spent_microusd").is_none(),
        "the bare micro field must not reach the model: {result}"
    );
}

#[tokio::test]
async fn alerts_reaches_the_model_with_both_spent_and_the_ambiguously_named_budget_as_usd() {
    // The exact 2026-09-27 evidence-log numbers: a run whose real budget was
    // five cents and real spend about six and a third cents, which Felyx
    // reported as "exceeded its $50 budget ... $63.64 spent" when this
    // field reached it as a bare integer.
    let base = spawn_stub(
        r#"[{"run_id":"genaryx-copilot","spent_microusd":63640,"budget_micros":50000,
             "fraction":1.2728,"killed":false}]"#,
    );
    let result = dispatch(&base, "alerts").await;
    assert_eq!(result[0]["spent_usd"], json!(0.063_64));
    assert_eq!(result[0]["budget_usd"], json!(0.05));
    assert!(result[0].get("spent_microusd").is_none());
    assert!(result[0].get("budget_micros").is_none());
}

#[tokio::test]
async fn list_runs_reaches_the_model_with_every_row_and_its_total_as_usd() {
    let base = spawn_stub(
        r#"[{"run_id":"r1","model":"","agent_id":"","spent_microusd":9000,
             "calls":1,"cache_hits":0,"steps":1,"last_seen_millis":0,"killed":false}]"#,
    );
    let result = dispatch(&base, "list_runs").await;
    assert_eq!(result["total_spent_usd"], json!(0.009));
    assert_eq!(result["runs"][0]["spent_usd"], json!(0.009));
}

#[tokio::test]
async fn savings_reaches_the_model_with_every_amount_as_usd() {
    let base = spawn_stub(
        r#"{"blocked_spend_microusd":1000,"cache_saved_microusd":2000,
             "router_saved_microusd":500,"budget_breaks":1,
             "total_saved_microusd":3500}"#,
    );
    let result = dispatch(&base, "savings").await;
    assert_eq!(result["total_saved_usd"], json!(0.0035));
    assert_eq!(result["blocked_spend_usd"], json!(0.001));
}

#[tokio::test]
async fn list_agents_reaches_the_model_with_spend_as_usd() {
    let base = spawn_stub(
        r#"[{"agent_id":"planner","spent_microusd":5000,"calls":2,"runs":1,
             "last_seen_millis":0}]"#,
    );
    let result = dispatch(&base, "list_agents").await;
    assert_eq!(result[0]["spent_usd"], json!(0.005));
}
