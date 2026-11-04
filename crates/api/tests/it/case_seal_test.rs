//! Sealing a case in a real store: the chain line whose target is the pack's
//! digest, the file named by that digest, the proofs that could not be made
//! carried as what they are, and every way a pack is refused or not handed back.
//!
//! The canonical form and the pack are held in `crates/api/src/seal/`'s own tests;
//! what is held here is the process around them.

use genaryx_api::bus::{AppState, BusMode};
use genaryx_api::case::commands::{CaseError, CasePlanes};
use genaryx_api::identity::IdentityState;
use genaryx_api::memory::MemoryState;
use genaryx_api::money::MoneyState;
use genaryx_api::money::state::BusHandle;
use genaryx_api::policy::PolicyState;
use genaryx_api::seal::commands::{
    CASE_CLOSE_ACTION, PACKS_DIR, PackError, SealFailure, SealRequest, case_pack, case_seal_at,
};
use genaryx_api::seal::{Decision, NOTE_MAX_CHARS, Outcome, ProofSpec, digest_of_text, verify};
use genaryx_core::store::Store;
use genaryx_core::{AgentEvent, ConsoleEvent, Provenance, SchemaVersion};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const SCHEMA: &str = "taipanbox.dev/agent-event/v0.2";
const AT: &str = "2026-11-04T12:00:00Z";

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-seal-test-{tag}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create_dir_all");
    dir
}

fn event() -> ConsoleEvent {
    ConsoleEvent {
        event: AgentEvent {
            schema: SCHEMA.to_string(),
            ts: "2026-11-04T10:00:00Z".to_string(),
            source: "tokenfuse".to_string(),
            event_type: "budget_exhausted".to_string(),
            agent_id: "agent://a/one".to_string(),
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
            offset: Some(1),
            endpoint: None,
            received_ts: "2026-11-04T10:00:01Z".to_string(),
        },
        raw: json!({ "n": 1 }).to_string(),
        schema_version: SchemaVersion::from_schema_str(SCHEMA).expect("a known schema"),
    }
}

/// A store with one incident, and the bus over its directory; returns the
/// directory, the bus and the incident's id.
fn setup(tag: &str) -> (PathBuf, AppState, i64) {
    let dir = temp_dir(tag);
    let store = Store::open(&dir.join("console.sqlite")).expect("Store::open");
    store.insert_batch(&[event()]).expect("insert_batch");
    let id = store.recent_events(10).expect("recent_events")[0].id;
    drop(store);
    let bus = AppState {
        events_dir: Some(dir.clone()),
        source_events_dir: Some(dir.clone()),
        mode: BusMode::Live {
            env: "test".to_string(),
            dir: dir.display().to_string(),
        },
    };
    (dir, bus, id)
}

struct Planes {
    money: MoneyState,
    policy: PolicyState,
    identity: IdentityState,
    memory: MemoryState,
}

fn pending() -> Planes {
    Planes {
        money: MoneyState::pending(),
        policy: PolicyState::pending(),
        identity: IdentityState::pending(),
        memory: MemoryState::pending(),
    }
}

fn planes<'a>(bus: &'a AppState, p: &'a Planes) -> CasePlanes<'a> {
    CasePlanes {
        bus,
        money: &p.money,
        policy: &p.policy,
        identity: &p.identity,
        memory: &p.memory,
    }
}

fn request(id: i64) -> SealRequest {
    SealRequest {
        incident_id: format!("bus:{id}"),
        proofs: vec![
            ProofSpec::Policy {
                candidate: "- name: p\n".to_string(),
            },
            ProofSpec::Budget {
                per_run_usd: Some("2".to_string()),
                per_step_usd: None,
                max_steps: None,
            },
        ],
        decision: Decision {
            outcome: Outcome::Applied,
            note: "limit set at the gateway".to_string(),
        },
    }
}

fn last_line(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).expect("the console's chain");
    let line = text.lines().last().expect("a line");
    serde_json::from_str(line).expect("a JSON line")
}

#[tokio::test]
async fn sealing_journals_the_digest_as_the_target_and_stores_the_pack() {
    let (dir, bus, id) = setup("journal");
    let p = pending();
    let journal = BusHandle::from_dirs(&dir, &dir);
    let sealed = case_seal_at(request(id), planes(&bus, &p), Some(&journal), AT)
        .await
        .expect("a seal");

    // The digest is the sha256 of the document, and the document is a canonical
    // pack of this format.
    assert_eq!(sealed.digest, digest_of_text(&sealed.pack));
    assert_eq!(verify(&sealed.pack, &sealed.digest), Ok(()));
    let pack: Value = serde_json::from_str(&sealed.pack).expect("JSON");
    assert_eq!(pack["sealed_at"], AT);
    assert_eq!(pack["decision"]["outcome"], "applied");
    assert_eq!(pack["case"]["incident_id"], format!("bus:{id}"));

    // The chain: one console_command, the case_close action, the digest as target.
    let line = last_line(&journal.console_events_path);
    assert_eq!(line["type"], "console_command");
    assert_eq!(line["data"]["action"], CASE_CLOSE_ACTION);
    assert_eq!(line["data"]["target"], sealed.digest);
    assert_eq!(line["data"]["verify_result"], "sealed:applied");

    // The file, named by the digest, private, and the same text.
    assert!(sealed.stored, "{:?}", sealed.store_error);
    let hex = sealed.digest.strip_prefix("sha256:").expect("sha256");
    let file = dir.join(PACKS_DIR).join(format!("{hex}.json"));
    assert_eq!(
        std::fs::read_to_string(&file).expect("the file"),
        sealed.pack
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "the pack is readable by others: {mode:o}");
    }
    assert_eq!(
        case_pack(&sealed.digest, Some(&dir)).expect("handed back"),
        sealed.pack
    );
}

#[tokio::test]
async fn the_proofs_that_could_not_be_made_are_in_the_pack_as_what_they_are() {
    let (dir, bus, id) = setup("proofs");
    let p = pending();
    let journal = BusHandle::from_dirs(&dir, &dir);
    let sealed = case_seal_at(request(id), planes(&bus, &p), Some(&journal), AT)
        .await
        .expect("a seal");
    let pack: Value = serde_json::from_str(&sealed.pack).expect("JSON");
    let proofs = pack["proofs"].as_array().expect("proofs");
    assert_eq!(proofs.len(), 2);
    assert_eq!(proofs[0]["kind"], "policy");
    assert_eq!(proofs[0]["request"]["candidate"], "- name: p\n");
    assert_eq!(proofs[0]["result"]["error"]["kind"], "not_configured");
    assert_eq!(proofs[1]["kind"], "budget");
    assert_eq!(proofs[1]["result"]["error"]["kind"], "not_configured");
    // Neither is dressed as a proof.
    assert!(proofs.iter().all(|p| p["result"].get("proof").is_none()));
}

#[tokio::test]
async fn a_seal_that_cannot_be_journaled_stores_nothing() {
    let (dir, bus, id) = setup("no-journal");
    let p = pending();
    let refused = case_seal_at(request(id), planes(&bus, &p), None, AT).await;
    assert!(
        matches!(refused, Err(SealFailure::NotJournaled { .. })),
        "{refused:?}"
    );
    assert!(
        !dir.join(PACKS_DIR).exists(),
        "a pack that was never sealed was stored"
    );
}

#[tokio::test]
async fn the_same_case_sealed_twice_at_the_same_time_is_the_same_pack() {
    let (dir, bus, id) = setup("twice");
    let p = pending();
    let journal = BusHandle::from_dirs(&dir, &dir);
    let first = case_seal_at(request(id), planes(&bus, &p), Some(&journal), AT)
        .await
        .expect("a seal");
    let second = case_seal_at(request(id), planes(&bus, &p), Some(&journal), AT)
        .await
        .expect("a seal");
    assert_eq!(first.digest, second.digest);
    assert_eq!(first.pack, second.pack);
    assert!(second.stored, "{:?}", second.store_error);
}

#[tokio::test]
async fn a_stored_pack_that_was_altered_is_not_handed_back() {
    let (dir, bus, id) = setup("altered");
    let p = pending();
    let journal = BusHandle::from_dirs(&dir, &dir);
    let sealed = case_seal_at(request(id), planes(&bus, &p), Some(&journal), AT)
        .await
        .expect("a seal");
    let hex = sealed.digest.strip_prefix("sha256:").expect("sha256");
    let file = dir.join(PACKS_DIR).join(format!("{hex}.json"));

    std::fs::write(&file, sealed.pack.replace("limit set", "limit not set")).expect("alter");
    assert!(matches!(
        case_pack(&sealed.digest, Some(&dir)),
        Err(PackError::NotThePack { .. })
    ));
    std::fs::write(&file, "not a pack").expect("alter");
    assert!(matches!(
        case_pack(&sealed.digest, Some(&dir)),
        Err(PackError::NotThePack { .. })
    ));
    std::fs::remove_file(&file).expect("remove");
    assert_eq!(
        case_pack(&sealed.digest, Some(&dir)),
        Err(PackError::NotFound)
    );
}

#[test]
fn a_digest_that_is_not_a_digest_never_reaches_the_filesystem() {
    let dir = temp_dir("digests");
    for bad in [
        "",
        "../../etc/passwd",
        "sha256:../../etc/passwd",
        "sha256:abc",
        &format!("sha256:{}", "AB".repeat(32)),
        &format!("sha256:{}/x", "ab".repeat(31)),
        &"ab".repeat(32),
    ] {
        assert_eq!(
            case_pack(bad, Some(&dir)),
            Err(PackError::BadDigest),
            "{bad:?}"
        );
    }
    let good = format!("sha256:{}", "ab".repeat(32));
    assert_eq!(case_pack(&good, None), Err(PackError::NoStore));
    assert_eq!(case_pack(&good, Some(&dir)), Err(PackError::NotFound));
}

#[tokio::test]
async fn a_request_that_is_not_sealed_is_refused_before_the_case_is_read() {
    // No store: a refused request is refused as such, not as a missing case.
    let bus = AppState {
        events_dir: None,
        source_events_dir: None,
        mode: BusMode::Live {
            env: "test".to_string(),
            dir: String::new(),
        },
    };
    let p = pending();
    let mut long_note = request(1);
    long_note.decision.note = "x".repeat(NOTE_MAX_CHARS + 1);
    let mut many = request(1);
    many.proofs = (0..5)
        .map(|_| ProofSpec::Policy {
            candidate: "x".to_string(),
        })
        .collect();
    for asked in [long_note, many] {
        let result = case_seal_at(asked, planes(&bus, &p), None, AT).await;
        assert!(
            matches!(result, Err(SealFailure::Refused { .. })),
            "{result:?}"
        );
    }
}

#[tokio::test]
async fn an_incident_the_store_does_not_hold_is_not_sealed() {
    let (dir, bus, _id) = setup("missing");
    let p = pending();
    let journal = BusHandle::from_dirs(&dir, &dir);
    let result = case_seal_at(request(99999), planes(&bus, &p), Some(&journal), AT).await;
    assert!(
        matches!(
            result,
            Err(SealFailure::Case {
                error: CaseError::NotFound { .. }
            })
        ),
        "{result:?}"
    );
    assert!(
        !journal.console_events_path.exists(),
        "a chain line was written"
    );
}
