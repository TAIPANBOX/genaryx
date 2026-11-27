//! The Money panel's commands end to end, on the shared stub Cloud: the
//! status of every state the panel can be in, each read and its DTO, and the
//! three signed mutations with the journal line each one writes, whatever the
//! Cloud answered.
//!
//! The mutation contract is the part where a wrong answer is silent: the
//! Cloud's verdict decides `Ok`/`Err`, the journal is written either way, and
//! a refusal the Cloud never saw (a blank break-glass reason) reaches neither
//! the Cloud nor the journal.

use crate::cloud_stub::{money_at, spawn_cloud, spawn_recording_cloud};
use genaryx_api::money::commands::{
    MoneyError, MoneyStatusDto, money_ack_incident, money_incidents, money_kill_run,
    money_overview, money_owners, money_savings, money_set_budget, money_spend_window,
    money_status,
};
use genaryx_api::money::env::EnvSource;
use genaryx_api::money::state::{BusHandle, MoneyClient, MoneyInner, MoneyState};
use genaryx_connectors::CloudClient;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

fn scratch(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-money-commands-itest-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// A Ready panel at `base` with a paired software device and the given bus.
async fn signed_money_at(base: &str, bus: Option<BusHandle>) -> MoneyState {
    let mut client = CloudClient::new(base, "test-token").expect("client");
    let signer = genaryx_signing::SoftwareSigner::generate().expect("software signer");
    client.attach_device("dev-test", "device-token", Box::new(signer));
    let state = MoneyState::pending();
    *state.inner.lock().await = MoneyInner::Ready(MoneyClient {
        client: Arc::new(client),
        source: EnvSource::EnvFallback,
        cloud_url: base.to_string(),
        org_domain: "acme.example".to_string(),
        operator: "user://acme.example/operator".to_string(),
        host: "test-host".to_string(),
        sig_fpr: "software-signed",
        bus,
    });
    state
}

async fn state_of(inner: MoneyInner) -> MoneyState {
    let state = MoneyState::pending();
    *state.inner.lock().await = inner;
    state
}

/// The last `console_command` line the console appended for this bus.
fn journaled(bus_dir: &std::path::Path) -> Value {
    let body = std::fs::read_to_string(bus_dir.join("console.ndjson"))
        .expect("the console's own events file");
    let last = body.lines().last().expect("at least one line");
    serde_json::from_str(last).expect("the line parses")
}

fn journal_rows(bus_dir: &std::path::Path) -> u64 {
    genaryx_core::store::Store::open(&bus_dir.join("console.sqlite"))
        .expect("store")
        .commands_journal_count()
        .expect("count")
}

#[tokio::test]
async fn the_status_names_every_state_the_panel_can_be_in() {
    let pending = MoneyState::pending();
    assert!(matches!(
        money_status(&pending).await,
        Ok(MoneyStatusDto::Bootstrapping)
    ));

    let none = state_of(MoneyInner::NoEnvironment).await;
    assert!(matches!(
        money_status(&none).await,
        Ok(MoneyStatusDto::NoEnvironment)
    ));

    let failed = state_of(MoneyInner::PairingFailed {
        source: EnvSource::Taipan {
            name: "single".to_string(),
        },
        cloud_url: "http://cloud.internal:8080".to_string(),
        reason: "device pairing failed: 401".to_string(),
    })
    .await;
    match money_status(&failed).await {
        Ok(MoneyStatusDto::PairingFailed {
            source,
            cloud_url,
            reason,
        }) => {
            assert_eq!(
                source,
                EnvSource::Taipan {
                    name: "single".to_string()
                }
            );
            assert_eq!(cloud_url, "http://cloud.internal:8080");
            assert_eq!(reason, "device pairing failed: 401");
        }
        other => panic!("expected PairingFailed, got {other:?}"),
    }

    let ready = signed_money_at("http://127.0.0.1:9", None).await;
    match money_status(&ready).await {
        Ok(MoneyStatusDto::Ready {
            source,
            cloud_url,
            org_domain,
        }) => {
            assert_eq!(source, EnvSource::EnvFallback);
            assert_eq!(cloud_url, "http://127.0.0.1:9");
            assert_eq!(org_domain, "acme.example");
        }
        other => panic!("expected Ready, got {other:?}"),
    }
}

/// Every read refuses with the panel's own state while it is not ready, and
/// never with a Cloud-shaped error for a Cloud it never called.
#[tokio::test]
async fn every_read_answers_the_panels_state_while_it_is_not_ready() {
    let pending = MoneyState::pending();
    assert!(matches!(
        money_overview(&pending).await,
        Err(MoneyError::Bootstrapping)
    ));
    assert!(matches!(
        money_incidents(&pending).await,
        Err(MoneyError::Bootstrapping)
    ));

    let none = state_of(MoneyInner::NoEnvironment).await;
    assert!(matches!(
        money_savings(&none).await,
        Err(MoneyError::NoEnvironment)
    ));
    assert!(matches!(
        money_owners(&none).await,
        Err(MoneyError::NoEnvironment)
    ));

    let failed = state_of(MoneyInner::PairingFailed {
        source: EnvSource::EnvFallback,
        cloud_url: "http://127.0.0.1:9".to_string(),
        reason: "timed out".to_string(),
    })
    .await;
    match money_spend_window(&failed, 7).await {
        Err(MoneyError::PairingFailed { reason }) => assert_eq!(reason, "timed out"),
        other => panic!("expected PairingFailed, got {other:?}"),
    }
    match money_ack_incident("inc-1".to_string(), &failed).await {
        Err(MoneyError::PairingFailed { reason }) => assert_eq!(reason, "timed out"),
        other => panic!("expected PairingFailed, got {other:?}"),
    }
}

/// A blank reason is refused before the panel is even consulted, so a
/// pending panel still answers the reason refusal, and the Cloud is never
/// called.
#[tokio::test]
async fn a_blank_break_glass_reason_is_refused_before_anything_is_called() {
    let (base, seen) = spawn_recording_cloud(vec![]);
    let state = signed_money_at(&base, None).await;
    for reason in ["", "   ", "\n\t"] {
        assert!(matches!(
            money_kill_run("run-1".to_string(), reason.to_string(), &state).await,
            Err(MoneyError::BreakGlassMissingReason)
        ));
        assert!(matches!(
            money_set_budget("run-1".to_string(), 5.0, reason.to_string(), &state).await,
            Err(MoneyError::BreakGlassMissingReason)
        ));
    }
    assert!(
        seen.lock().expect("log").is_empty(),
        "nothing reaches the Cloud without a reason"
    );
    let pending = MoneyState::pending();
    assert!(matches!(
        money_kill_run("run-1".to_string(), String::new(), &pending).await,
        Err(MoneyError::BreakGlassMissingReason)
    ));
}

#[tokio::test]
async fn the_overview_counts_active_runs_and_open_incidents_from_the_clouds_reads() {
    let summary = json!({"runs": 9, "calls": 120, "spent_microusd": 12_500_000}).to_string();
    let runs = json!([
        {"run_id": "r1", "model": "m", "agent_id": "agent://acme.example/a", "spent_microusd": 1,
         "calls": 1, "cache_hits": 0, "steps": 1, "last_seen_millis": 0, "killed": false},
        {"run_id": "r2", "model": "m", "agent_id": "agent://acme.example/a", "spent_microusd": 1,
         "calls": 1, "cache_hits": 0, "steps": 1, "last_seen_millis": 0, "killed": true},
        {"run_id": "r3", "model": "m", "agent_id": "agent://acme.example/b", "spent_microusd": 1,
         "calls": 1, "cache_hits": 0, "steps": 1, "last_seen_millis": 0, "killed": false},
    ])
    .to_string();
    let incidents = json!([
        incident("inc-1", "high", false),
        incident("inc-2", "critical", true),
        incident("inc-3", "low", false),
    ])
    .to_string();
    let savings = json!({
        "blocked_spend_microusd": 1_000_000, "cache_saved_microusd": 250_000,
        "router_saved_microusd": 0, "budget_breaks": 2, "total_saved_microusd": 1_250_000,
    })
    .to_string();
    let base = spawn_cloud(vec![
        ("/v1/summary", "200 OK", summary),
        ("/v1/runs", "200 OK", runs),
        ("/v1/incidents", "200 OK", incidents),
        ("/v1/savings", "200 OK", savings),
    ]);
    let o = money_overview(&money_at(&base).await)
        .await
        .expect("all four reads answer");
    assert_eq!(o.total_runs, 9, "the summary's all-time count");
    assert_eq!(o.total_calls, 120);
    assert_eq!(o.total_spent_usd, 12.5);
    assert_eq!(o.killed_runs, 1);
    assert_eq!(o.active_runs, 2, "runs listed less the killed one");
    assert_eq!(o.open_incidents, 2, "the acknowledged incident is not open");
    assert_eq!(o.total_incidents, 3);
    assert_eq!(o.total_saved_usd, 1.25);
    assert_eq!(o.pii_calls, None, "not reported is not zero");
}

fn incident(id: &str, severity: &str, acknowledged: bool) -> Value {
    json!({
        "id": id, "org": "acme", "run_id": "r1", "agent_id": null, "kind": "spend_spike",
        "severity": severity, "first_seen_millis": 1_753_000_000_000_i64,
        "last_seen_millis": 1_753_000_060_000_i64, "occurrences": 4,
        "acknowledged": acknowledged, "last_notified_millis": 0,
    })
}

/// One failed read fails the overview with that read's own status, never a
/// half-filled set of tiles.
#[tokio::test]
async fn one_failed_read_fails_the_overview_with_the_clouds_status() {
    let base = spawn_cloud(vec![
        (
            "/v1/summary",
            "200 OK",
            json!({"runs": 0, "calls": 0, "spent_microusd": 0}).to_string(),
        ),
        ("/v1/runs", "200 OK", "[]".to_string()),
        ("/v1/incidents", "200 OK", "[]".to_string()),
        (
            "/v1/savings",
            "500 Internal Server Error",
            r#"{"error":"db locked"}"#.to_string(),
        ),
    ]);
    match money_overview(&money_at(&base).await).await {
        Err(MoneyError::Cloud { status, message }) => {
            assert_eq!(status, Some(500));
            assert!(message.contains("db locked"), "{message}");
        }
        other => panic!("expected a Cloud error, got {other:?}"),
    }
}

#[tokio::test]
async fn incidents_savings_and_owners_reach_the_panel_in_its_own_units() {
    let base = spawn_cloud(vec![
        (
            "/v1/incidents",
            "200 OK",
            json!([incident("inc-9", "critical", false)]).to_string(),
        ),
        (
            "/v1/savings",
            "200 OK",
            json!({
                "blocked_spend_microusd": 2_000_000, "cache_saved_microusd": 500_000,
                "router_saved_microusd": 250_000, "budget_breaks": 3,
                "total_saved_microusd": 2_750_000,
            })
            .to_string(),
        ),
        (
            "/v1/owners",
            "200 OK",
            json!([{
                "owner": "user://acme.example/operator", "spent_microusd": 4_200_000,
                "calls": 17, "runs": 3, "agents": 2,
                "last_seen_millis": 1_753_000_000_000_i64, "tool_calls": 5,
            }])
            .to_string(),
        ),
    ]);
    let state = money_at(&base).await;

    let incidents = money_incidents(&state).await.expect("incidents");
    assert_eq!(incidents.len(), 1);
    let i = &incidents[0];
    assert_eq!(i.id, "inc-9");
    assert_eq!(i.severity, "critical");
    assert_eq!(i.kind, "spend_spike");
    assert_eq!(i.run_id.as_deref(), Some("r1"));
    assert_eq!(i.agent_id, None);
    assert_eq!(i.occurrences, 4);
    assert!(!i.acknowledged);
    assert_eq!(i.first_seen, "2025-07-20T08:26:40.000Z");
    assert_eq!(i.last_seen, "2025-07-20T08:27:40.000Z");

    let s = money_savings(&state).await.expect("savings");
    assert_eq!(s.blocked_spend_usd, 2.0);
    assert_eq!(s.cache_saved_usd, 0.5);
    assert_eq!(s.router_saved_usd, 0.25);
    assert_eq!(s.budget_breaks, 3);
    assert_eq!(s.total_saved_usd, 2.75);

    let owners = money_owners(&state).await.expect("owners");
    assert_eq!(owners.len(), 1);
    let o = &owners[0];
    assert_eq!(o.owner, "user://acme.example/operator");
    assert_eq!(o.spent_usd, 4.2);
    assert_eq!((o.calls, o.runs, o.agents, o.tool_calls), (17, 3, 2, 5));
    assert_eq!(o.last_seen, "2025-07-20T08:26:40.000Z");
}

/// A Cloud without the period route cannot answer the question, which is not
/// the answer zero.
#[tokio::test]
async fn a_cloud_without_the_period_route_answers_none_not_zero() {
    let old = spawn_cloud(vec![]);
    assert!(matches!(
        money_spend_window(&money_at(&old).await, 7).await,
        Ok(None)
    ));

    let new = spawn_cloud(vec![(
        "/v1/spend",
        "200 OK",
        json!({
            "days_requested": 7, "days_covered": 3, "from_day": null,
            "spent_microusd": 1500, "calls": 4, "blocked": 1, "agents": [],
        })
        .to_string(),
    )]);
    let w = money_spend_window(&money_at(&new).await, 7)
        .await
        .expect("a read")
        .expect("this Cloud folds by period");
    assert_eq!((w.days_requested, w.days_covered), (7, 3));
    assert_eq!(w.spent_microusd, 1500);
}

/// A kill the Cloud confirms is `Ok`, signed, and journaled in the console's
/// own file with the reason kept out of the event and the verdict in it.
#[tokio::test]
async fn a_confirmed_kill_is_ok_and_journaled_as_break_glass() {
    let bus_dir = scratch("kill-ok");
    let (base, seen) = spawn_recording_cloud(vec![(
        "/v1/runs/run-7/kill",
        "200 OK",
        r#"{"killed":"run-7"}"#.to_string(),
    )]);
    let state = signed_money_at(&base, Some(BusHandle::from_dirs(&bus_dir, &bus_dir))).await;
    let out = money_kill_run(
        "run-7".to_string(),
        "runaway spend".to_string(),
        &state,
    )
    .await
    .expect("the Cloud confirmed the kill");
    assert_eq!(out.summary, "run run-7 killed");
    assert_eq!(out.http_status, 200);
    assert_eq!(out.verify_result, "killed:true");
    assert_eq!(out.sig_alg, "es256");
    assert_eq!(out.sig_fpr, "software-signed");
    assert!(out.bus_recorded, "{:?}", out.bus_error);
    assert_eq!(out.bus_error, None);

    let req = seen.lock().expect("log")[0].clone();
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/v1/runs/run-7/kill");
    assert!(req.header("x-fuse-sig").is_some(), "a kill is signed");
    assert_eq!(req.header("x-fuse-device"), Some("dev-test"));

    let line = journaled(&bus_dir);
    assert_eq!(line["type"], "console_command");
    assert_eq!(line["data"]["action"], "console.kill_run");
    assert_eq!(line["data"]["target"], "run-7");
    assert_eq!(line["data"]["decision"], "break_glass");
    assert_eq!(line["data"]["http_status"], 200);
    assert_eq!(line["data"]["verify_result"], "killed:true");
    assert!(
        !line.to_string().contains("runaway spend"),
        "the reason is journaled in params, never in the bus event"
    );
    assert_eq!(journal_rows(&bus_dir), 1);
    let _ = std::fs::remove_dir_all(&bus_dir);
}

/// A kill answered with a different run id is not a confirmed kill of this
/// one, and the journal says so.
#[tokio::test]
async fn a_kill_answered_for_another_run_is_not_journaled_as_this_one_killed() {
    let bus_dir = scratch("kill-other");
    let base = spawn_cloud(vec![(
        "/v1/runs/run-7/kill",
        "200 OK",
        r#"{"killed":"run-8"}"#.to_string(),
    )]);
    let state = signed_money_at(&base, Some(BusHandle::from_dirs(&bus_dir, &bus_dir))).await;
    let out = money_kill_run("run-7".to_string(), "why".to_string(), &state)
        .await
        .expect("a 200");
    assert_eq!(out.verify_result, "killed:false");
    assert_eq!(journaled(&bus_dir)["data"]["verify_result"], "killed:false");
    let _ = std::fs::remove_dir_all(&bus_dir);
}

/// A Cloud that rejects the device signature is an error to the operator and
/// still a journal line carrying the Cloud's real 403.
#[tokio::test]
async fn a_rejected_signature_is_an_error_and_still_journaled_with_the_real_status() {
    let bus_dir = scratch("kill-403");
    let base = spawn_cloud(vec![(
        "/v1/runs/run-7/kill",
        "403 Forbidden",
        r#"{"error":"signature_invalid"}"#.to_string(),
    )]);
    let state = signed_money_at(&base, Some(BusHandle::from_dirs(&bus_dir, &bus_dir))).await;
    match money_kill_run("run-7".to_string(), "why".to_string(), &state).await {
        Err(MoneyError::Cloud { status, message }) => {
            assert_eq!(status, Some(403));
            assert!(message.contains("signature_invalid"), "{message}");
        }
        other => panic!("expected a Cloud 403, got {other:?}"),
    }
    let line = journaled(&bus_dir);
    assert_eq!(line["data"]["http_status"], 403);
    assert!(
        line["data"]["verify_result"]
            .as_str()
            .is_some_and(|v| v.starts_with("error:")),
        "{line}"
    );
    assert_eq!(journal_rows(&bus_dir), 1);
    let _ = std::fs::remove_dir_all(&bus_dir);
}

#[tokio::test]
async fn a_budget_change_reports_what_the_cloud_stored() {
    let bus_dir = scratch("budget");
    let (base, seen) = spawn_recording_cloud(vec![(
        "/v1/runs/run-7/budget",
        "200 OK",
        r#"{"run":"run-7","budget_micros":5000000}"#.to_string(),
    )]);
    let state = signed_money_at(&base, Some(BusHandle::from_dirs(&bus_dir, &bus_dir))).await;
    let out = money_set_budget(
        "run-7".to_string(),
        5.0,
        "cap the runaway".to_string(),
        &state,
    )
    .await
    .expect("stored");
    assert_eq!(out.summary, "run run-7 budget set to $5.0000");
    assert_eq!(out.verify_result, "budget_micros:5000000");
    let body: Value =
        serde_json::from_str(&seen.lock().expect("log")[0].body).expect("a JSON body");
    assert_eq!(body, json!({"budget_usd": 5.0}));
    let line = journaled(&bus_dir);
    assert_eq!(line["data"]["action"], "console.set_budget");
    assert_eq!(line["data"]["decision"], "break_glass");
    let _ = std::fs::remove_dir_all(&bus_dir);
}

/// A plan the org does not have is its own error, so the panel can say so
/// rather than show a failure.
#[tokio::test]
async fn a_plan_refusal_is_plan_required_never_a_generic_failure() {
    let base = spawn_cloud(vec![(
        "/v1/runs/run-7/budget",
        "402 Payment Required",
        json!({"error": {"type": "plan_required", "feature": "budgets", "org": "acme",
                         "upgrade_url": "https://example.invalid/plans"}})
        .to_string(),
    )]);
    let state = signed_money_at(&base, None).await;
    match money_set_budget("run-7".to_string(), 1.0, "why".to_string(), &state).await {
        Err(MoneyError::PlanRequired { feature, org, .. }) => {
            assert_eq!((feature.as_str(), org.as_str()), ("budgets", "acme"));
        }
        other => panic!("expected PlanRequired, got {other:?}"),
    }
}

/// An acknowledgement is not an override: no reason, decision `allow`.
#[tokio::test]
async fn an_acknowledgement_is_journaled_as_allow() {
    let bus_dir = scratch("ack");
    let base = spawn_cloud(vec![(
        "/v1/incidents/inc-1/ack",
        "200 OK",
        r#"{"acknowledged":"inc-1"}"#.to_string(),
    )]);
    let state = signed_money_at(&base, Some(BusHandle::from_dirs(&bus_dir, &bus_dir))).await;
    let out = money_ack_incident("inc-1".to_string(), &state)
        .await
        .expect("acknowledged");
    assert_eq!(out.summary, "incident inc-1 acknowledged");
    assert_eq!(out.verify_result, "acknowledged:true");
    let line = journaled(&bus_dir);
    assert_eq!(line["data"]["action"], "console.ack_incident");
    assert_eq!(line["data"]["decision"], "allow");
    let _ = std::fs::remove_dir_all(&bus_dir);
}

/// With no live bus the Cloud's verdict still reaches the operator, and the
/// missing journal line is reported, not hidden.
#[tokio::test]
async fn with_no_bus_a_confirmed_mutation_says_it_was_not_journaled() {
    let base = spawn_cloud(vec![(
        "/v1/incidents/inc-1/ack",
        "200 OK",
        r#"{"acknowledged":"inc-1"}"#.to_string(),
    )]);
    let state = signed_money_at(&base, None).await;
    let out = money_ack_incident("inc-1".to_string(), &state)
        .await
        .expect("acknowledged");
    assert!(!out.bus_recorded);
    assert!(
        out.bus_error
            .as_deref()
            .is_some_and(|e| e.contains("no live event bus")),
        "{:?}",
        out.bus_error
    );
}

/// A store that cannot be opened costs the journal line, which is reported,
/// and never the Cloud's verdict.
#[tokio::test]
async fn an_unopenable_store_is_reported_as_not_journaled() {
    let root = scratch("unopenable");
    let file = root.join("not-a-directory");
    std::fs::write(&file, "x").expect("write");
    let base = spawn_cloud(vec![(
        "/v1/incidents/inc-1/ack",
        "200 OK",
        r#"{"acknowledged":"inc-1"}"#.to_string(),
    )]);
    let state = signed_money_at(&base, Some(BusHandle::from_dirs(&file, &root))).await;
    let out = money_ack_incident("inc-1".to_string(), &state)
        .await
        .expect("the Cloud's verdict still stands");
    assert!(!out.bus_recorded);
    assert!(out.bus_error.is_some());
    let _ = std::fs::remove_dir_all(&root);
}

/// The failures that never reached a Cloud carry no status, and say which
/// step stopped them.
#[tokio::test]
async fn failures_before_any_answer_carry_no_status_and_name_their_step() {
    // No paired device: refused before anything is signed.
    let unsigned = money_at("http://127.0.0.1:9").await;
    match money_kill_run("run-1".to_string(), "why".to_string(), &unsigned).await {
        Err(MoneyError::Cloud { status, message }) => {
            assert_eq!(status, None);
            assert!(message.contains("no paired device signer"), "{message}");
        }
        other => panic!("expected a no-signer error, got {other:?}"),
    }

    // An id that cannot be one path segment never addresses another resource.
    let signed = signed_money_at("http://127.0.0.1:9", None).await;
    match money_kill_run("..".to_string(), "why".to_string(), &signed).await {
        Err(MoneyError::Cloud { status, message }) => {
            assert_eq!(status, None);
            assert!(message.contains("cannot be used in a request path"), "{message}");
        }
        other => panic!("expected a path refusal, got {other:?}"),
    }

    // Nothing listening.
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = l.local_addr().expect("addr");
        drop(l);
        format!("http://{addr}")
    };
    match money_savings(&money_at(&dead).await).await {
        Err(MoneyError::Cloud { status, message }) => {
            assert_eq!(status, None);
            assert!(message.starts_with("could not reach the Cloud"), "{message}");
        }
        other => panic!("expected a transport error, got {other:?}"),
    }

    // A 200 whose body is not the shape asked for.
    let odd = spawn_cloud(vec![(
        "/v1/savings",
        "200 OK",
        "<html>not the Cloud</html>".to_string(),
    )]);
    match money_savings(&money_at(&odd).await).await {
        Err(MoneyError::Cloud { status, message }) => {
            assert_eq!(status, None);
            assert!(
                message.starts_with("unexpected response shape from the Cloud"),
                "{message}"
            );
        }
        other => panic!("expected a shape error, got {other:?}"),
    }
}
