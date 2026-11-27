//! A decision on a held call with an approver key configured, driven through
//! the real `policy_decide_approval` against a stub wardryx with a real
//! journal (CLAUDE.md invariants 35 and 48): the statement names what the
//! person opened, a decision that cannot carry one is refused before wardryx
//! is asked, and every refusal is journaled. Also the Policy panel's reads.

use crate::cloud_stub::{Seen, spawn_recording_cloud};
use genaryx_api::console_actor::{ConsoleSignature, with_signature};
use genaryx_api::policy::PolicyState;
use genaryx_api::policy::approver::ApproverKey;
use genaryx_api::policy::commands::{
    DecisionDto, PolicyError, PolicyStatusDto, policy_decide_approval, policy_enforcement_status,
    policy_list_approvals, policy_list_policies, policy_status,
};
use genaryx_api::policy::env::EnvSource;
use genaryx_api::policy::preview::SeenPreviews;
use genaryx_api::policy::state::{BusHandle, PolicyClient, PolicyInner};
use genaryx_connectors::WardryxClient;
use genaryx_signing::Es256Signer as _;
use genaryx_signing::SoftwareSigner;
use genaryx_signing::approver::{Shown, verify};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const OPERATOR: &str = "user://wardryx.local/test";
const TCD: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const PVD: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const CREDENTIAL: &str = "Y3JlZC10ZXN0";
const HOLDS: &str = "/v1/approvals";
const DECIDE: &str = "/v1/approvals/ap_1/decide";

fn holds(context: Value) -> String {
    json!([{
        "approval_id": "ap_1",
        "agent_id": "agent://acme.example/agents/a",
        "run_id": "run-1",
        "requested_at": "2026-11-12T00:00:00Z",
        "pending": true,
        "context": context,
    }])
    .to_string()
}

fn sealed_hold() -> String {
    holds(json!({
        "org": "acme.example",
        "tool_call": {"name": "t", "target": "s", "digest": TCD},
        "preview": {"state": "sealed", "pvd": PVD, "ruleset": "r1"},
    }))
}

fn granted() -> String {
    json!({"approval_id": "ap_1", "decision": "grant"}).to_string()
}

fn scratch_bus(tag: &str) -> (PathBuf, BusHandle) {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-policy-decide-itest-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
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
        operator: OPERATOR.to_string(),
        host: "test".to_string(),
        sig_alg: "none",
        sig_fpr: "bearer-admin",
        bus,
    });
    state
}

/// The console's approver key on a fixed test scalar, and its public half.
fn approver() -> (ApproverKey, Vec<u8>) {
    let signer = SoftwareSigner::from_scalar(&[7u8; 32]).expect("a test scalar");
    let public = signer.public_key_x963().expect("public half");
    (ApproverKey::Ready(Box::new(signer)), public)
}

fn passkey() -> Option<ConsoleSignature> {
    Some(ConsoleSignature {
        alg: "webauthn-es256".to_string(),
        fpr: CREDENTIAL.to_string(),
    })
}

/// The `data` of every console line the journal holds.
fn journal(bus: &BusHandle) -> Vec<Value> {
    std::fs::read_to_string(&bus.console_events_path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: Value = serde_json::from_str(l).expect("a JSON line");
            v["data"].clone()
        })
        .collect()
}

fn decides(log: &Arc<Mutex<Vec<Seen>>>) -> Vec<Seen> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|s| s.method == "POST")
        .cloned()
        .collect()
}

fn now() -> i64 {
    genaryx_api::policy::approver::now_unix()
}

#[tokio::test]
async fn a_confirmed_grant_of_an_opened_preview_sends_a_statement_naming_it() {
    let (base, log) = spawn_recording_cloud(vec![
        (HOLDS, "200 OK", sealed_hold()),
        (DECIDE, "200 OK", granted()),
    ]);
    let (dir, bus) = scratch_bus("statement");
    let state = policy_at(&base, Some(bus.clone())).await;
    let (key, public) = approver();
    let seen = SeenPreviews::default();
    seen.record(OPERATOR, "ap_1", PVD);

    let outcome = with_signature(
        passkey(),
        policy_decide_approval(
            "ap_1".to_string(),
            DecisionDto::Grant,
            Some(PVD.to_string()),
            &state,
            &key,
            &seen,
        ),
    )
    .await
    .expect("granted");
    assert_eq!(outcome.http_status, 200);
    assert_eq!(outcome.verify_result, "granted (no token returned)");
    assert!(outcome.bus_recorded, "{:?}", outcome.bus_error);

    let posts = decides(&log);
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].path, DECIDE);
    let body: Value = serde_json::from_str(&posts[0].body).expect("a JSON body");
    assert_eq!(body["decision"], "grant");
    assert_eq!(body["decided_by"], OPERATOR);
    let statement = body["approver_statement"]
        .as_str()
        .expect("a statement is sent");
    let claims = verify(&public, statement, now()).expect("it verifies under the console key");
    assert_eq!(claims.aid, "ap_1");
    assert_eq!(claims.dec, "grant");
    assert_eq!(claims.sub, OPERATOR);
    assert_eq!(claims.cred, CREDENTIAL);
    assert_eq!(
        claims.shown,
        Some(Shown {
            tcd: TCD.to_string(),
            pvd: PVD.to_string()
        })
    );

    let lines = journal(&bus);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["action"], "console.grant_approval");
    assert_eq!(lines[0]["decision"], "allow");
    assert_eq!(lines[0]["sig_alg"], "webauthn-es256");
    assert_eq!(lines[0]["sig_fpr"], CREDENTIAL);
    assert_eq!(lines[0]["http_status"], 200);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_grant_of_a_sealed_hold_nobody_opened_is_refused_and_journaled() {
    let (base, log) = spawn_recording_cloud(vec![
        (HOLDS, "200 OK", sealed_hold()),
        (DECIDE, "200 OK", granted()),
    ]);
    let (dir, bus) = scratch_bus("unopened");
    let state = policy_at(&base, Some(bus.clone())).await;
    let (key, _) = approver();
    // The browser names the digest, but this person never opened it here.
    let result = with_signature(
        passkey(),
        policy_decide_approval(
            "ap_1".to_string(),
            DecisionDto::Grant,
            Some(PVD.to_string()),
            &state,
            &key,
            &SeenPreviews::default(),
        ),
    )
    .await;
    let reason = match result {
        Err(PolicyError::Preview { reason }) => reason,
        other => panic!("expected a Preview refusal, got {other:?}"),
    };
    assert!(reason.contains("no record of the person deciding opening"), "{reason}");
    assert!(decides(&log).is_empty(), "nothing is sent to wardryx");

    let lines = journal(&bus);
    assert_eq!(lines.len(), 1, "the refusal is journaled");
    assert_eq!(lines[0]["action"], "console.grant_approval");
    assert_eq!(lines[0]["decision"], "refused");
    assert_eq!(lines[0]["http_status"], 0);
    assert_eq!(
        lines[0]["verify_result"],
        format!("refused before wardryx: {reason}")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn with_a_key_file_a_decision_no_passkey_confirmed_never_reaches_wardryx() {
    let (base, log) = spawn_recording_cloud(vec![
        (HOLDS, "200 OK", holds(json!({"tool_call": {"name": "t", "digest": TCD}}))),
        (DECIDE, "200 OK", granted()),
    ]);
    let (dir, bus) = scratch_bus("no-passkey");
    let state = policy_at(&base, Some(bus.clone())).await;
    let (key, _) = approver();
    let result = policy_decide_approval(
        "ap_1".to_string(),
        DecisionDto::Deny,
        None,
        &state,
        &key,
        &SeenPreviews::default(),
    )
    .await;
    match result {
        Err(PolicyError::ApproverStatement { reason }) => assert_eq!(
            reason,
            "a key file is configured for approver statements, and no passkey confirmed this decision"
        ),
        other => panic!("expected an ApproverStatement refusal, got {other:?}"),
    }
    assert!(decides(&log).is_empty());
    let lines = journal(&bus);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["action"], "console.deny_approval");
    assert_eq!(lines[0]["decision"], "refused");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn an_unusable_key_file_refuses_every_decision_without_asking_wardryx() {
    let (base, log) = spawn_recording_cloud(vec![(DECIDE, "200 OK", granted())]);
    let state = policy_at(&base, None).await;
    let result = policy_decide_approval(
        "ap_1".to_string(),
        DecisionDto::Grant,
        None,
        &state,
        &ApproverKey::Failed("/etc/genaryx/approver.pem: not a key".to_string()),
        &SeenPreviews::default(),
    )
    .await;
    match result {
        Err(PolicyError::ApproverStatement { reason }) => assert_eq!(
            reason,
            "the approver key file cannot be used: /etc/genaryx/approver.pem: not a key"
        ),
        other => panic!("expected an ApproverStatement refusal, got {other:?}"),
    }
    assert!(log.lock().unwrap().is_empty(), "no request at all");
}

#[tokio::test]
async fn a_decision_on_a_console_that_is_not_ready_says_which_state_it_is_in() {
    let pending = PolicyState::pending();
    let result = policy_decide_approval(
        "ap_1".to_string(),
        DecisionDto::Grant,
        None,
        &pending,
        &ApproverKey::NotConfigured,
        &SeenPreviews::default(),
    )
    .await;
    assert!(matches!(result, Err(PolicyError::Bootstrapping)));

    let none = PolicyState::pending();
    *none.inner.lock().await = PolicyInner::NoEnvironment;
    assert!(matches!(
        policy_list_approvals(&none).await,
        Err(PolicyError::NoEnvironment)
    ));

    let down = PolicyState::pending();
    *down.inner.lock().await = PolicyInner::Unreachable {
        source: EnvSource::EnvFallback,
        wardryx_url: "http://127.0.0.1:1".to_string(),
        reason: "healthz timed out".to_string(),
    };
    match policy_list_policies(&down).await {
        Err(PolicyError::Unreachable { reason }) => assert_eq!(reason, "healthz timed out"),
        other => panic!("expected Unreachable, got {other:?}"),
    }
    assert!(matches!(
        policy_enforcement_status(&pending).await,
        Err(PolicyError::Bootstrapping)
    ));
}

#[tokio::test]
async fn the_status_names_each_state_the_panel_can_be_in() {
    assert!(matches!(
        policy_status(&PolicyState::pending()).await,
        Ok(PolicyStatusDto::Bootstrapping)
    ));
    let none = PolicyState::pending();
    *none.inner.lock().await = PolicyInner::NoEnvironment;
    assert!(matches!(
        policy_status(&none).await,
        Ok(PolicyStatusDto::NoEnvironment)
    ));
    let down = PolicyState::pending();
    *down.inner.lock().await = PolicyInner::Unreachable {
        source: EnvSource::Taipan {
            name: "lab".to_string(),
        },
        wardryx_url: "http://127.0.0.1:1".to_string(),
        reason: "refused".to_string(),
    };
    match policy_status(&down).await {
        Ok(PolicyStatusDto::Unreachable {
            source,
            wardryx_url,
            reason,
        }) => {
            assert_eq!(
                source,
                EnvSource::Taipan {
                    name: "lab".to_string()
                }
            );
            assert_eq!(wardryx_url, "http://127.0.0.1:1");
            assert_eq!(reason, "refused");
        }
        other => panic!("expected Unreachable, got {other:?}"),
    }
    let ready = policy_at("http://127.0.0.1:9", None).await;
    match policy_status(&ready).await {
        Ok(PolicyStatusDto::Ready {
            source,
            wardryx_url,
            org_domain,
        }) => {
            assert_eq!(source, EnvSource::EnvFallback);
            assert_eq!(wardryx_url, "http://127.0.0.1:9");
            assert_eq!(org_domain, "wardryx.local");
        }
        other => panic!("expected Ready, got {other:?}"),
    }
}

#[tokio::test]
async fn the_reads_carry_what_wardryx_answered() {
    let policies = json!([{
        "id": "p-1",
        "name": "no shell",
        "target": "agent://acme.example/*",
        "deny_tool": ["shell"],
        "max_steps": 40,
        "updated_at": "2026-11-20T10:00:00Z"
    }])
    .to_string();
    let status = json!({
        "policy_version": "v7",
        "base_policies": 3,
        "store_policies": 1,
        "effective_policies": 4
    })
    .to_string();
    let (base, _log) = spawn_recording_cloud(vec![
        (HOLDS, "200 OK", sealed_hold()),
        ("/v1/policies", "200 OK", policies),
        ("/v1/status", "200 OK", status),
    ]);
    let state = policy_at(&base, None).await;

    let approvals = policy_list_approvals(&state).await.expect("approvals");
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].approval_id, "ap_1");
    assert_eq!(approvals[0].org.as_deref(), Some("acme.example"));
    assert_eq!(
        approvals[0].preview.as_ref().map(|p| p.state.as_str()),
        Some("sealed")
    );

    let list = policy_list_policies(&state).await.expect("policies");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, "p-1");
    assert_eq!(list[0].name, "no shell");
    assert_eq!(list[0].deny_tool, vec!["shell".to_string()]);
    assert_eq!(list[0].max_steps, 40);
    assert_eq!(list[0].updated_at.as_deref(), Some("2026-11-20T10:00:00Z"));

    let enforced = policy_enforcement_status(&state).await.expect("status");
    assert_eq!(enforced.policy_version, "v7");
    assert_eq!(enforced.effective_policies, 4);
    assert_eq!(enforced.base_policies, 3);
}

#[tokio::test]
async fn a_read_wardryx_refuses_carries_its_status() {
    let (base, _log) = spawn_recording_cloud(vec![(
        "/v1/policies",
        "500 Internal Server Error",
        r#"{"error":"store unavailable"}"#.to_string(),
    )]);
    let state = policy_at(&base, None).await;
    match policy_list_policies(&state).await {
        Err(PolicyError::Wardryx { status, message }) => {
            assert_eq!(status, Some(500));
            assert!(message.contains("store unavailable"), "{message}");
        }
        other => panic!("expected a Wardryx error, got {other:?}"),
    }
}
