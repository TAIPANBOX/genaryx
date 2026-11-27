//! The Copilot view's commands, with no model reached: the residency banner
//! for every state, every question answered as an operator-readable refusal
//! while Felyx cannot answer, a case read refused in words before any model
//! call, and the audit link for an approved proposal journaled only against a
//! paired Money device.
//!
//! The one enabled service here points at a loopback port nothing listens on,
//! so the provider fails at connect and no model is called.

use genaryx_api::bus::{AppState, BusMode};
use genaryx_api::case::commands::CasePlanes;
use genaryx_api::copilot::commands::{
    copilot_ask, copilot_explain, copilot_explain_case, copilot_log_proposal_approved,
    copilot_status,
};
use genaryx_api::copilot::state::{CopilotInner, CopilotState};
use genaryx_api::identity::IdentityState;
use genaryx_api::memory::MemoryState;
use genaryx_api::money::env::EnvSource;
use genaryx_api::money::state::{BusHandle, MoneyClient, MoneyInner, MoneyState};
use genaryx_api::policy::PolicyState;
use genaryx_connectors::CloudClient;
use genaryx_copilot::{Clients, CopilotConfig, CopilotService, ProviderKind};
use genaryx_core::ingest::IngestService;
use genaryx_core::store::Store;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

fn scratch(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-copilot-itest-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

async fn copilot_of(inner: CopilotInner) -> CopilotState {
    let state = CopilotState::pending();
    *state.inner.lock().await = inner;
    state
}

fn disabled_service() -> CopilotService {
    CopilotService::from_config_and_clients(&CopilotConfig::default(), Clients::default())
        .expect("the default config builds a disabled service")
}

/// A loopback address nothing listens on.
fn dead_url() -> String {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = l.local_addr().expect("addr");
    drop(l);
    format!("http://{addr}/v1")
}

fn enabled_service(base_url: &str) -> CopilotService {
    let config = CopilotConfig {
        provider: ProviderKind::Ollama,
        base_url: Some(base_url.to_string()),
        model: Some("test-model".to_string()),
        agent_id: Some("agent://acme.example/genaryx/felyx".to_string()),
        ..CopilotConfig::default()
    };
    CopilotService::from_config_and_clients(&config, Clients::default())
        .expect("a loopback Ollama endpoint passes the residency gate")
}

async fn money_ready(bus: Option<BusHandle>) -> MoneyState {
    let state = MoneyState::pending();
    *state.inner.lock().await = MoneyInner::Ready(MoneyClient {
        client: Arc::new(CloudClient::new("http://127.0.0.1:9", "").expect("client")),
        source: EnvSource::EnvFallback,
        cloud_url: "http://127.0.0.1:9".to_string(),
        org_domain: "acme.example".to_string(),
        operator: "user://acme.example/operator".to_string(),
        host: "test-host".to_string(),
        sig_fpr: "software-signed",
        bus,
    });
    state
}

/// Every plane a case reads, each still in its pending state.
struct PendingPlanes {
    money: MoneyState,
    policy: PolicyState,
    identity: IdentityState,
    memory: MemoryState,
}

impl PendingPlanes {
    fn new() -> Self {
        Self {
            money: MoneyState::pending(),
            policy: PolicyState::pending(),
            identity: IdentityState::pending(),
            memory: MemoryState::pending(),
        }
    }

    fn with<'a>(&'a self, bus: &'a AppState) -> CasePlanes<'a> {
        CasePlanes {
            bus,
            money: &self.money,
            policy: &self.policy,
            identity: &self.identity,
            memory: &self.memory,
        }
    }
}

fn no_store() -> AppState {
    AppState {
        events_dir: None,
        source_events_dir: None,
        mode: BusMode::Unavailable {
            reason: "test".to_string(),
        },
    }
}

#[tokio::test]
async fn the_banner_is_disabled_with_a_reason_until_a_provider_is_ready() {
    let s = copilot_status(&CopilotState::pending())
        .await
        .expect("never fails");
    assert!(!s.enabled);
    assert_eq!(
        s.disabled_reason.as_deref(),
        Some("Copilot is still starting up.")
    );
    assert_eq!(s.provider, None);

    let s = copilot_status(&copilot_of(CopilotInner::Failed("bad model setting".to_string())).await)
        .await
        .expect("never fails");
    assert!(!s.enabled);
    assert_eq!(s.disabled_reason.as_deref(), Some("bad model setting"));

    let s = copilot_status(&copilot_of(CopilotInner::Ready(Arc::new(disabled_service()))).await)
        .await
        .expect("never fails");
    assert!(!s.enabled);
    assert!(
        s.disabled_reason
            .as_deref()
            .is_some_and(|r| r.contains("No copilot provider is configured")),
        "{:?}",
        s.disabled_reason
    );
    assert_eq!((s.provider, s.model, s.endpoint, s.local), (None, None, None, None));
}

/// An enabled provider shows its residency: which provider, which model,
/// where, and that the endpoint is local.
#[tokio::test]
async fn an_enabled_provider_shows_where_inference_runs() {
    let url = dead_url();
    let s = copilot_status(&copilot_of(CopilotInner::Ready(Arc::new(enabled_service(&url)))).await)
        .await
        .expect("never fails");
    assert!(s.enabled);
    assert_eq!(s.provider.as_deref(), Some("ollama"));
    assert_eq!(s.model.as_deref(), Some("test-model"));
    assert_eq!(s.endpoint.as_deref(), Some(url.as_str()));
    assert_eq!(s.local, Some(true));
    assert_eq!(s.disabled_reason, None);
}

#[tokio::test]
async fn a_question_felyx_cannot_answer_is_an_operator_readable_refusal() {
    let Err(e) = copilot_ask(&CopilotState::pending(), "why?".to_string()).await else {
        panic!("a starting copilot answered");
    };
    assert_eq!(e, "Copilot is still starting up; try again in a moment.");

    let failed = copilot_of(CopilotInner::Failed("bad model setting".to_string())).await;
    let Err(e) = copilot_explain(&failed, "inc-1".to_string()).await else {
        panic!("a failed copilot answered");
    };
    assert_eq!(e, "bad model setting");

    let disabled = copilot_of(CopilotInner::Ready(Arc::new(disabled_service()))).await;
    let Err(e) = copilot_ask(&disabled, "why?".to_string()).await else {
        panic!("a disabled copilot answered");
    };
    assert!(e.contains("no copilot provider is configured"), "{e}");
    let Err(e) = copilot_explain(&disabled, "inc-1".to_string()).await else {
        panic!("a disabled copilot explained");
    };
    assert!(e.contains("no copilot provider is configured"), "{e}");

    // An enabled provider that cannot be reached is a provider error, not
    // "not configured".
    let unreachable = copilot_of(CopilotInner::Ready(Arc::new(enabled_service(&dead_url())))).await;
    let Err(e) = copilot_ask(&unreachable, "why?".to_string()).await else {
        panic!("a dead endpoint answered");
    };
    assert!(!e.is_empty());
    assert!(!e.contains("no copilot provider is configured"), "{e}");
}

#[tokio::test]
async fn a_case_read_is_refused_in_words_before_any_model_call() {
    let p = PendingPlanes::new();
    let bus = no_store();

    let Err(e) =
        copilot_explain_case(&CopilotState::pending(), p.with(&bus), "bus:1".to_string()).await
    else {
        panic!("a starting copilot read a case");
    };
    assert_eq!(e, "Copilot is still starting up; try again in a moment.");

    let disabled = copilot_of(CopilotInner::Ready(Arc::new(disabled_service()))).await;
    let Err(e) = copilot_explain_case(&disabled, p.with(&bus), "bus:1".to_string()).await else {
        panic!("a case with no store was read");
    };
    assert_eq!(
        e,
        "No case: the console has no readable event store on this box."
    );
    let Err(e) = copilot_explain_case(&disabled, p.with(&bus), "money:5".to_string()).await else {
        panic!("a money incident was read as a case");
    };
    assert_eq!(e, "No case: a money incident has no event row to start from.");
}

/// A case that does assemble still reaches no model when no provider is
/// configured, and says so.
#[tokio::test]
async fn an_assembled_case_on_a_disabled_copilot_is_no_provider() {
    let root = scratch("case");
    {
        let store = Store::open(&root.join("console.sqlite")).expect("store");
        let mut ingest = IngestService::new(store, "single").expect("ingest");
        let path = root.join("wardryx.ndjson");
        let line = json!({
            "schema": "taipanbox.dev/agent-event/v0.2", "ts": "2026-10-20T09:00:00.000Z",
            "source": "wardryx", "type": "policy_deny",
            "agent_id": "agent://acme.example/agents/a", "run_id": "run-1",
            "data": {"policy": "deny-all"},
        });
        std::fs::write(&path, format!("{line}\n")).expect("write");
        ingest
            .add_file_source("filetail:wardryx", &path)
            .expect("tail");
        assert_eq!(ingest.poll_once().expect("poll").inserted, 1);
    }
    let bus = AppState {
        events_dir: Some(root.clone()),
        source_events_dir: None,
        mode: BusMode::Unavailable {
            reason: "test".to_string(),
        },
    };
    let p = PendingPlanes::new();
    let disabled = copilot_of(CopilotInner::Ready(Arc::new(disabled_service()))).await;
    let Err(e) = copilot_explain_case(&disabled, p.with(&bus), "bus:1".to_string()).await else {
        panic!("a disabled copilot read the case");
    };
    assert!(e.contains("no copilot provider is configured"), "{e}");
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn an_approved_proposal_is_journaled_only_against_a_paired_money_device() {
    let pending = copilot_log_proposal_approved(
        "kill".to_string(),
        "run-1".to_string(),
        json!({}),
        &MoneyState::pending(),
    )
    .await
    .expect("never fails");
    assert!(!pending.journaled);
    assert_eq!(
        pending.journal_error.as_deref(),
        Some("no paired Money device to journal against")
    );

    let no_bus = copilot_log_proposal_approved(
        "kill".to_string(),
        "run-1".to_string(),
        json!({}),
        &money_ready(None).await,
    )
    .await
    .expect("never fails");
    assert!(!no_bus.journaled);
    assert!(
        no_bus
            .journal_error
            .as_deref()
            .is_some_and(|e| e.contains("no live event bus")),
        "{:?}",
        no_bus.journal_error
    );

    let dir = scratch("approved");
    let out = copilot_log_proposal_approved(
        "budget".to_string(),
        "run-42".to_string(),
        json!({"usd_cap": 5.0}),
        &money_ready(Some(BusHandle::from_dirs(&dir, &dir))).await,
    )
    .await
    .expect("never fails");
    assert!(out.journaled, "{:?}", out.journal_error);
    let body = std::fs::read_to_string(dir.join("console.ndjson")).expect("journaled");
    let line: Value = serde_json::from_str(body.lines().last().expect("a line")).expect("parses");
    assert_eq!(line["data"]["action"], "console.copilot_proposal_approved");
    assert_eq!(line["data"]["target"], "run-42");
    assert_eq!(line["data"]["decision"], "allow");
    assert_eq!(
        line["data"]["verify_result"],
        "copilot proposal approved: kind=budget target=run-42"
    );
    let _ = std::fs::remove_dir_all(&dir);

    let root = scratch("approved-unopenable");
    let file = root.join("not-a-directory");
    std::fs::write(&file, "x").expect("write");
    let out = copilot_log_proposal_approved(
        "kill".to_string(),
        "run-1".to_string(),
        json!({}),
        &money_ready(Some(BusHandle::from_dirs(&file, &root))).await,
    )
    .await
    .expect("never fails");
    assert!(!out.journaled);
    assert!(out.journal_error.is_some());
    let _ = std::fs::remove_dir_all(&root);
}
