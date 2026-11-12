//! A person's grant reaches the operator as the raw token, once, and nothing the
//! console keeps ever holds it (CLAUDE.md invariant 46).
//!
//! Everything here drives the real `policy_decide_approval` against a stub
//! wardryx, with a real store and a real journal, and then reads back every byte
//! the console wrote. The tokens are invented: their signatures say so, and the
//! console never verifies one, so no secret is involved.

use crate::cloud_stub::spawn_cloud;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use genaryx_api::policy::PolicyState;
use genaryx_api::policy::approver::ApproverKey;
use genaryx_api::policy::commands::{
    DecideOutcome, DecisionDto, PolicyError, policy_decide_approval, policy_list_approvals,
};
use genaryx_api::policy::env::EnvSource;
use genaryx_api::policy::state::{BusHandle, PolicyClient, PolicyInner};
use genaryx_connectors::WardryxClient;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const APPROVAL: &str = "ap_0123456789abcdef";
const DECIDE_ROUTE: &str = "/v1/approvals/ap_0123456789abcdef/decide";
const DIGEST: &str = "1111111111111111111111111111111111111111111111111111111111111111";

fn claims() -> Value {
    json!({
        "agent_id": "agent://example.invalid/agents/held",
        "run_id": "run-held",
        "tools": ["fetch_url"],
        "max_cost_usd": 0.5,
        "exp": 4_102_444_800_i64,
        "nonce": "not-a-real-nonce",
        "v": 2,
        "tcd": DIGEST,
    })
}

fn segment(value: &Value) -> String {
    URL_SAFE_NO_PAD.encode(value.to_string())
}

/// `payload.sig`: a token signed with the shared secret.
fn shared_secret_token() -> String {
    format!("{}.fake-signature-of-a-console-test", segment(&claims()))
}

/// `header.payload.sig`: a v3 compact JWS signed through Keyryx, which is what
/// stack-single's wardryx mints.
fn keyryx_token() -> String {
    let header = json!({"alg": "ES256", "kid": "test-key", "typ": "wardryx-approval+jwt"});
    let mut claims = claims();
    claims["v"] = json!(3);
    claims["aid"] = json!(APPROVAL);
    format!(
        "{}.{}.fake-es256-signature-of-a-console-test",
        segment(&header),
        segment(&claims)
    )
}

fn grant_body(token: &str) -> String {
    json!({"approval_id": APPROVAL, "decision": "grant", "approval_token": token}).to_string()
}

fn scratch_bus(tag: &str) -> (PathBuf, BusHandle) {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-approval-token-itest-{tag}-{}-{n}",
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

async fn decide(state: &PolicyState, decision: DecisionDto) -> Result<DecideOutcome, PolicyError> {
    policy_decide_approval(
        APPROVAL.to_string(),
        decision,
        None,
        state,
        &ApproverKey::NotConfigured,
        &genaryx_api::policy::preview::SeenPreviews::default(),
    )
    .await
}

/// The answer as the browser receives it.
fn wire(outcome: &DecideOutcome) -> Value {
    serde_json::to_value(outcome).expect("the answer serialises")
}

/// Every file under `dir`, as bytes: the SQLite store and whatever it keeps
/// beside itself, the bus file, anything a later change starts writing there.
fn every_file_under(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next).expect("read dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let bytes = std::fs::read(&path).expect("read file");
                files.push((path, bytes));
            }
        }
    }
    files
}

fn holds(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// The whole token and each of its segments, so that a copy of any part of one
/// counts: the signature on its own is what forges nothing, and the payload on
/// its own is what says what a person approved.
fn parts_of(token: &str) -> Vec<String> {
    std::iter::once(token.to_string())
        .chain(token.split('.').map(str::to_string))
        .collect()
}

#[tokio::test]
async fn a_grant_answers_with_the_raw_token_wardryx_minted() {
    for token in [shared_secret_token(), keyryx_token()] {
        let base = spawn_cloud(vec![(DECIDE_ROUTE, "200 OK", grant_body(&token))]);
        let state = policy_at(&base, None).await;
        let outcome = decide(&state, DecisionDto::Grant).await.expect("granted");
        let answer = wire(&outcome);
        assert_eq!(answer["approval_token"], json!(token));
        assert_eq!(answer["http_status"], 200);
    }
}

#[tokio::test]
async fn a_grant_carries_the_claims_it_shows_in_both_token_formats() {
    for token in [shared_secret_token(), keyryx_token()] {
        let base = spawn_cloud(vec![(DECIDE_ROUTE, "200 OK", grant_body(&token))]);
        let state = policy_at(&base, None).await;
        let outcome = decide(&state, DecisionDto::Grant).await.expect("granted");
        let answer = wire(&outcome);
        assert_eq!(
            answer["token"]["agent_id"], "agent://example.invalid/agents/held",
            "{token}"
        );
        assert_eq!(answer["token"]["exp_unix"], 4_102_444_800_i64);
        assert_eq!(answer["token"]["tool_call_digest"], DIGEST);
    }
}

#[tokio::test]
async fn a_token_granted_on_a_hold_with_no_call_says_it_binds_none() {
    let mut unbound = claims();
    let map = unbound.as_object_mut().expect("an object");
    map.remove("tcd");
    map.remove("v");
    let token = format!("{}.fake-signature-of-a-console-test", segment(&unbound));
    let base = spawn_cloud(vec![(DECIDE_ROUTE, "200 OK", grant_body(&token))]);
    let state = policy_at(&base, None).await;
    let outcome = decide(&state, DecisionDto::Grant).await.expect("granted");
    assert!(wire(&outcome)["token"]["tool_call_digest"].is_null());
}

#[tokio::test]
async fn a_grant_whose_claims_cannot_be_read_still_answers_with_the_raw_token() {
    for token in [
        "not-a-token-at-all",
        "not-base64-claims!!.fake-signature-of-a-console-test",
    ] {
        let base = spawn_cloud(vec![(DECIDE_ROUTE, "200 OK", grant_body(token))]);
        let state = policy_at(&base, None).await;
        let outcome = decide(&state, DecisionDto::Grant).await.expect("granted");
        let answer = wire(&outcome);
        assert_eq!(answer["approval_token"], json!(token));
        assert!(answer["token"].is_null(), "{answer}");
    }
}

#[tokio::test]
async fn a_deny_answers_with_no_token() {
    // Wardryx sends none on a denial. If one ever came, it is still no grant.
    let body = json!({
        "approval_id": APPROVAL,
        "decision": "deny",
        "approval_token": shared_secret_token(),
    })
    .to_string();
    let base = spawn_cloud(vec![(DECIDE_ROUTE, "200 OK", body)]);
    let state = policy_at(&base, None).await;
    let outcome = decide(&state, DecisionDto::Deny).await.expect("denied");
    assert!(wire(&outcome)["approval_token"].is_null());
}

#[tokio::test]
async fn an_approval_already_decided_shows_no_token_a_second_time() {
    let body = json!({"error": "approval was already decided"}).to_string();
    let base = spawn_cloud(vec![(DECIDE_ROUTE, "409 Conflict", body)]);
    let state = policy_at(&base, None).await;
    let refused = decide(&state, DecisionDto::Grant)
        .await
        .expect_err("refused");
    assert!(matches!(
        refused,
        PolicyError::Wardryx {
            status: Some(409),
            ..
        }
    ));
}

#[tokio::test]
async fn the_raw_token_reaches_no_journal_row_no_bus_line_and_no_file() {
    for token in [
        shared_secret_token(),
        keyryx_token(),
        "not-a-token-at-all".to_string(),
    ] {
        let base = spawn_cloud(vec![(DECIDE_ROUTE, "200 OK", grant_body(&token))]);
        let (dir, bus) = scratch_bus("files");
        let line_path = bus.console_events_path.clone();
        let store_path = bus.store_db_path.clone();
        let state = policy_at(&base, Some(bus)).await;

        let outcome = decide(&state, DecisionDto::Grant).await.expect("granted");
        let answer = wire(&outcome);
        assert_eq!(answer["bus_recorded"], true, "{answer}");
        assert_eq!(answer["approval_token"], json!(token));

        // Did not fail is not did run: the record of this grant is there, in the
        // two places it belongs, or this scan measured nothing.
        let line = std::fs::read_to_string(&line_path).expect("the bus line");
        assert!(line.contains("console.grant_approval"), "{line}");
        let journal = rusqlite::Connection::open(&store_path).expect("the store");
        let (action, verify): (String, String) = journal
            .query_row(
                "SELECT action, verify_result FROM commands_journal ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("a journal row");
        assert_eq!(action, "console.grant_approval");
        assert!(verify.starts_with("granted"), "{verify}");
        drop(journal);

        let files = every_file_under(&dir);
        assert!(files.len() >= 2, "only {} files to read", files.len());
        for (path, bytes) in &files {
            for part in parts_of(&token) {
                assert!(!holds(bytes, &part), "{} holds {part}", path.display());
            }
        }
    }
}

#[tokio::test]
async fn the_answer_prints_without_the_raw_token() {
    let token = shared_secret_token();
    let base = spawn_cloud(vec![(DECIDE_ROUTE, "200 OK", grant_body(&token))]);
    let state = policy_at(&base, None).await;
    let outcome = decide(&state, DecisionDto::Grant).await;
    assert!(wire(outcome.as_ref().expect("granted"))["approval_token"].is_string());
    let printed = format!("{outcome:?}");
    for part in parts_of(&token) {
        assert!(!printed.contains(&part), "{printed}");
    }
}

#[tokio::test]
async fn the_queue_carries_the_reason_and_the_call_wardryx_sent() {
    let approvals = json!([{
        "approval_id": APPROVAL,
        "agent_id": "agent://example.invalid/agents/held",
        "run_id": "run-held",
        "requested_at": "2026-11-07T10:00:00Z",
        "pending": true,
        "context": {
            "reason": "no policy grants this tool",
            "tool_names": ["fetch_url"],
            "tool_call": {
                "name": "fetch_url",
                "target": "https://example.invalid/page",
                "arguments_truncated": true,
                "digest": DIGEST,
            },
        },
    }]);
    let base = spawn_cloud(vec![("/v1/approvals", "200 OK", approvals.to_string())]);
    let state = policy_at(&base, None).await;
    let rows = policy_list_approvals(&state).await.expect("the queue");
    let row = serde_json::to_value(&rows[0]).expect("a row serialises");
    assert_eq!(row["reason"], "no policy grants this tool");
    assert_eq!(row["tool_call"]["target"], "https://example.invalid/page");
    assert_eq!(row["tool_call"]["arguments_truncated"], true);
}

/// Felyx has no road to a token: the copilot crate names neither the call that
/// mints one, nor the response that carries one, nor the field itself.
#[test]
fn the_copilot_crate_cannot_name_the_token_or_the_call_that_mints_it() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../copilot/src");
    let files = every_file_under(&root);
    assert!(files.len() >= 5, "only {} copilot files read", files.len());
    for (path, bytes) in &files {
        for name in [
            "approval_token",
            "decide_approval",
            "ApprovalDecideResponse",
            "RawApprovalToken",
            "DecideOutcome",
        ] {
            assert!(!holds(bytes, name), "{} names {name}", path.display());
        }
    }
}
