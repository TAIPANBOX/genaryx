//! A remedy checked against an incident in a real store: the class the event
//! type gives, the kinds offered, whether the remedy fits, the proof that is made
//! or the statement that there is none, the scenario named and whether this box
//! has it, and the route a person would apply it by. The proofs themselves are
//! held in `policy_proof_test` and `budget_proof_test`; what is held here is
//! that the right one is asked for and that nothing is applied.

use genaryx_api::bus::{AppState, BusMode};
use genaryx_api::case::Data;
use genaryx_api::proof::budget_commands::BudgetConfig;
use genaryx_api::proof::commands::{ProofConfig, ProofError};
use genaryx_api::proof::slots::Slots;
use genaryx_api::remedy::commands::{
    ProofOutcome, RemedyCheck, RemedyError, RemedyRequest, remedy_check_in,
};
use genaryx_api::remedy::{
    ApplyRoute, IDENTITY_IS_FILED_UNDER_THE_KEY, IncidentClass, NO_HISTORY_REPLAYS_AN_ACT, Remedy,
    RemedyKind,
};
use genaryx_core::store::Store;
use genaryx_core::{AgentEvent, ConsoleEvent, Provenance, SchemaVersion};
use serde_json::json;
use std::path::{Path, PathBuf};

const SCHEMA: &str = "taipanbox.dev/agent-event/v0.2";
const AGENT: &str = "agent://a/one";

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-remedy-test-{tag}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create_dir_all");
    dir
}

fn event(n: u64, event_type: &str) -> ConsoleEvent {
    ConsoleEvent {
        event: AgentEvent {
            schema: SCHEMA.to_string(),
            ts: "2026-10-08T10:00:00Z".to_string(),
            source: "tokenfuse".to_string(),
            event_type: event_type.to_string(),
            agent_id: AGENT.to_string(),
            severity: Some("high".to_string()),
            run_id: Some("r-incident".to_string()),
            on_behalf_of: Vec::new(),
            data: Some(json!({})),
            prev_hash: None,
            extra: serde_json::Map::new(),
        },
        provenance: Provenance {
            env: "test".to_string(),
            connector: "fixture".to_string(),
            file: Some("tokenfuse.ndjson".to_string()),
            offset: Some(n),
            endpoint: None,
            received_ts: "2026-11-04T00:00:00Z".to_string(),
        },
        raw: json!({ "n": n }).to_string(),
        schema_version: SchemaVersion::from_schema_str(SCHEMA).expect("a known schema"),
    }
}

/// A bus with one event of `event_type`; returns the bus and the event's id.
fn bus(dir: &Path, event_type: &str) -> (AppState, i64) {
    let store = Store::open(&dir.join("console.sqlite")).expect("Store::open");
    store
        .insert_batch(&[event(1, event_type)])
        .expect("insert_batch");
    let id = store.recent_events(10).expect("recent_events")[0].id;
    drop(store);
    let bus = AppState {
        events_dir: Some(dir.to_path_buf()),
        source_events_dir: Some(dir.to_path_buf()),
        mode: BusMode::Live {
            env: "test".to_string(),
            dir: dir.display().to_string(),
        },
    };
    (bus, id)
}

fn unconfigured() -> (ProofConfig, BudgetConfig) {
    (
        ProofConfig {
            wardryx_bin: PathBuf::from("wardryx"),
            archive: None,
            events: None,
        },
        BudgetConfig {
            tokenfuse_bin: PathBuf::from("tokenfuse"),
            trace_dir: None,
        },
    )
}

async fn check(
    bus: &AppState,
    id: i64,
    remedy: Remedy,
    scenario_dir: Option<PathBuf>,
) -> Result<RemedyCheck, RemedyError> {
    let (policy, budget) = unconfigured();
    remedy_check_in(
        &Slots::new(2),
        policy,
        budget,
        RemedyRequest {
            incident_id: format!("bus:{id}"),
            remedy,
        },
        bus,
        scenario_dir,
    )
    .await
}

fn stop() -> Remedy {
    Remedy::StopAgent {
        subject: AGENT.to_string(),
    }
}

#[tokio::test]
async fn a_spend_incident_is_classified_offered_a_budget_and_a_drill() {
    let dir = temp_dir("spend");
    let (bus, id) = bus(&dir, "budget_exhausted");
    let scenarios = temp_dir("spend-scenarios");
    std::fs::write(scenarios.join("runaway-budget.yaml"), "name: runaway-budget\n").unwrap();

    let found = check(&bus, id, stop(), Some(scenarios.clone()))
        .await
        .expect("a check");
    assert_eq!(found.class, IncidentClass::Spend);
    assert_eq!(found.event_type, Data::bounded("budget_exhausted"));
    assert_eq!(found.offered, [RemedyKind::BudgetLimit, RemedyKind::StopAgent]);
    assert_eq!(found.why_none, None);
    assert!(found.fits_class);
    let drill = found.drill.expect("a scenario for spend");
    assert_eq!(drill.scenario, "runaway-budget.yaml");
    assert_eq!(drill.in_scenario_dir, Some(true));

    // The same scenario, named, in a directory that does not hold it, and with no
    // directory at all: said as what each is, never as present.
    let empty = temp_dir("spend-empty");
    let absent = check(&bus, id, stop(), Some(empty)).await.expect("a check");
    assert_eq!(absent.drill.expect("named").in_scenario_dir, Some(false));
    let unknown = check(&bus, id, stop(), None).await.expect("a check");
    assert_eq!(unknown.drill.expect("named").in_scenario_dir, None);

    // A policy candidate is not what a spend incident is offered.
    let policy = Remedy::PolicyCandidate {
        candidate: "- name: p\n".to_string(),
    };
    let misfit = check(&bus, id, policy, None).await.expect("a check");
    assert!(!misfit.fits_class);
}

#[tokio::test]
async fn a_stop_is_unproven_and_says_so_and_routes_to_the_ceremony() {
    let dir = temp_dir("stop");
    let (bus, id) = bus(&dir, "policy_deny");
    let found = check(&bus, id, stop(), None).await.expect("a check");
    assert_eq!(
        found.proof,
        ProofOutcome::Unproven {
            reason: NO_HISTORY_REPLAYS_AN_ACT
        }
    );
    assert_eq!(
        found.apply,
        ApplyRoute::Command {
            command: "agent_stop",
            arguments: json!({ "subject": AGENT }),
            person_supplies: &["reason"],
        }
    );
    assert!(found.fits_class);
    // The check read the store and nothing else: no slot is held after it.
    let slots = Slots::new(0);
    let (policy, budget) = unconfigured();
    let with_no_slot = remedy_check_in(
        &slots,
        policy,
        budget,
        RemedyRequest {
            incident_id: format!("bus:{id}"),
            remedy: stop(),
        },
        &bus,
        None,
    )
    .await;
    assert!(with_no_slot.is_ok(), "an unproven remedy needs no slot");
}

#[tokio::test]
async fn a_policy_candidate_goes_to_the_policy_proof_and_a_budget_to_the_budget_proof() {
    let dir = temp_dir("routing");
    let (bus, id) = bus(&dir, "policy_deny");

    let policy = Remedy::PolicyCandidate {
        candidate: "- name: p\n".to_string(),
    };
    match check(&bus, id, policy, None).await.expect("a check").proof {
        ProofOutcome::Failed {
            error: ProofError::NotConfigured { missing },
        } => assert!(missing.contains("WARDRYX_POLICY_ARCHIVE"), "{missing}"),
        other => panic!("expected the policy proof to be asked for, got {other:?}"),
    }

    let budget = Remedy::BudgetLimit {
        per_run_usd: Some("2".to_string()),
        per_step_usd: None,
        max_steps: None,
    };
    match check(&bus, id, budget, None).await.expect("a check").proof {
        ProofOutcome::Failed {
            error: ProofError::NotConfigured { missing },
        } => assert!(missing.contains("TOKENFUSE_DATA_DIR"), "{missing}"),
        other => panic!("expected the budget proof to be asked for, got {other:?}"),
    }
}

#[tokio::test]
async fn an_identity_refusal_is_offered_nothing_and_a_stop_of_its_claimed_agent_does_not_fit() {
    let dir = temp_dir("identity");
    let (bus, id) = bus(&dir, "identity_mismatch");
    let found = check(&bus, id, stop(), None).await.expect("a check");
    assert_eq!(found.class, IncidentClass::Identity);
    assert!(found.offered.is_empty());
    assert_eq!(found.why_none, Some(IDENTITY_IS_FILED_UNDER_THE_KEY));
    assert!(!found.fits_class);
    // No scenario rehearses a refused key.
    assert!(found.drill.is_none());
    // It is still a check, not a refusal to speak: the remedy is routed as a
    // person asked for it and the proof says it has none.
    assert!(matches!(found.proof, ProofOutcome::Unproven { .. }));

    let revoke = Remedy::RevokeDelegation {
        subject: AGENT.to_string(),
    };
    assert!(!check(&bus, id, revoke, None).await.expect("a check").fits_class);
}

#[tokio::test]
async fn an_event_type_the_table_does_not_hold_is_offered_nothing_and_never_guessed() {
    let dir = temp_dir("unclassified");
    let (bus, id) = bus(&dir, "a_type_nobody_listed");
    let found = check(&bus, id, stop(), None).await.expect("a check");
    assert_eq!(found.class, IncidentClass::Unclassified);
    assert!(found.offered.is_empty());
    assert!(found.why_none.is_some());
    assert!(!found.fits_class);
    assert!(found.drill.is_none());
}

#[tokio::test]
async fn a_remedy_that_is_not_well_formed_is_refused_before_the_store_is_read() {
    // No store at all: a malformed remedy is refused as such, not as a missing
    // incident, so the refusal came first.
    let bus = AppState {
        events_dir: None,
        source_events_dir: None,
        mode: BusMode::Live {
            env: "test".to_string(),
            dir: String::new(),
        },
    };
    let (policy, budget) = unconfigured();
    let result = remedy_check_in(
        &Slots::new(2),
        policy,
        budget,
        RemedyRequest {
            incident_id: "bus:1".to_string(),
            remedy: Remedy::StopAgent {
                subject: "not an agent".to_string(),
            },
        },
        &bus,
        None,
    )
    .await;
    assert!(
        matches!(result, Err(RemedyError::Refused { .. })),
        "{result:?}"
    );
}

#[tokio::test]
async fn an_incident_with_no_event_row_is_refused_by_name() {
    let dir = temp_dir("no-row");
    let (bus, _id) = bus(&dir, "budget_exhausted");
    for incident_id in ["money:5", "bus:99999", "not-an-id", ""] {
        let (policy, budget) = unconfigured();
        let result = remedy_check_in(
            &Slots::new(2),
            policy,
            budget,
            RemedyRequest {
                incident_id: incident_id.to_string(),
                remedy: stop(),
            },
            &bus,
            None,
        )
        .await;
        assert!(
            matches!(result, Err(RemedyError::Incident { .. })),
            "{incident_id:?}: {result:?}"
        );
    }
}
