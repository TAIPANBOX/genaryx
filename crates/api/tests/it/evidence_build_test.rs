//! The Evidence Center's build, end to end over the shared stub Cloud: a pack
//! is signed only by the Money panel's paired device, degrades to an honest
//! unsigned local-only build when Money is not ready, journals its own digest
//! when it can, and never claims a pack of nothing.
//!
//! No local tool is requested here (`include_qryx`/`include_idryx`/
//! `include_tokenfuse` stay false), so nothing is spawned: the Cloud's two
//! sources are the whole pack.

use crate::cloud_stub::{money_at, spawn_cloud};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use genaryx_api::evidence::commands::{
    EvidenceBuildRequest, EvidenceError, EvidenceStatusDto, evidence_build, evidence_status,
};
use genaryx_api::evidence::state::{EvidenceEnv, EvidenceInner, EvidenceState};
use genaryx_api::money::env::EnvSource;
use genaryx_api::money::state::{BusHandle, MoneyClient, MoneyInner, MoneyState};
use genaryx_connectors::CloudClient;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

fn scratch(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-evidence-itest-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

async fn ready_env() -> EvidenceState {
    let state = EvidenceState::pending();
    *state.inner.lock().await = EvidenceInner::Ready(EvidenceEnv::default());
    state
}

fn cloud_only(include_cloud: bool) -> EvidenceBuildRequest {
    EvidenceBuildRequest {
        include_cloud,
        include_qryx: false,
        qryx_target: None,
        include_idryx: false,
        include_tokenfuse: false,
        tokenfuse_traces_dir: None,
    }
}

/// A Ready Money panel with a paired software device and the given bus.
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

fn compliance_cloud(audit: Value) -> String {
    spawn_cloud(vec![
        (
            "/v1/compliance/evidence",
            "200 OK",
            json!({"controls": [{"id": "SOC2-CC6.1", "status": "covered"}]}).to_string(),
        ),
        ("/v1/audit/verify", "200 OK", audit.to_string()),
    ])
}

#[tokio::test]
async fn the_status_reports_each_local_source_and_waits_while_bootstrapping() {
    let pending = EvidenceState::pending();
    assert!(matches!(
        evidence_status(&pending).await,
        Ok(EvidenceStatusDto::Bootstrapping)
    ));
    match evidence_status(&ready_env().await).await {
        Ok(EvidenceStatusDto::Ready {
            qryx_available,
            idryx_available,
            tokenfuse_available,
            idryx_load_sources,
            ..
        }) => {
            assert!(!qryx_available && !idryx_available && !tokenfuse_available);
            assert!(idryx_load_sources.is_empty());
        }
        other => panic!("expected Ready, got {other:?}"),
    }
}

#[tokio::test]
async fn a_build_before_discovery_finishes_is_bootstrapping() {
    let money = MoneyState::pending();
    match evidence_build(cloud_only(true), &EvidenceState::pending(), &money).await {
        Err(EvidenceError::Bootstrapping) => {}
        other => panic!("expected Bootstrapping, got {:?}", other.err()),
    }
}

/// With Money not ready the Cloud is left out even when asked for, and a pack
/// with nothing in it is refused rather than signed or handed back empty.
#[tokio::test]
async fn with_money_not_ready_and_no_local_source_there_is_no_pack_of_nothing() {
    let money = MoneyState::pending();
    match evidence_build(cloud_only(true), &ready_env().await, &money).await {
        Err(EvidenceError::Build { message }) => {
            assert!(message.contains("no artifacts"), "{message}");
        }
        other => panic!("expected a refused empty pack, got {:?}", other.err()),
    }
}

/// A Cloud whose two sources both fail leaves nothing to pack, and that is
/// refused too.
#[tokio::test]
async fn a_cloud_whose_sources_both_fail_leaves_no_pack() {
    let base = spawn_cloud(vec![]);
    let money = signed_money_at(&base, None).await;
    match evidence_build(cloud_only(true), &ready_env().await, &money).await {
        Err(EvidenceError::Build { message }) => {
            assert!(message.contains("no artifacts"), "{message}");
        }
        other => panic!("expected a refused empty pack, got {:?}", other.err()),
    }
}

/// The signed path: both Cloud sources in the pack, the manifest signed by
/// the paired device, and one journal line naming the pack by the digest of
/// the very bytes handed back.
#[tokio::test]
async fn a_paired_console_builds_signs_and_journals_the_pack_by_its_digest() {
    let bus_dir = scratch("signed");
    let base = compliance_cloud(json!({"ok": true}));
    let money = signed_money_at(&base, Some(BusHandle::from_dirs(&bus_dir, &bus_dir))).await;
    let dto = evidence_build(cloud_only(true), &ready_env().await, &money)
        .await
        .unwrap_or_else(|e| panic!("the pack builds: {e:?}"));

    assert!(dto.signed, "a paired device signs the manifest");
    assert!(dto.cloud_included);
    assert_eq!(dto.manifest.artifacts.len(), 2);
    assert!(dto.manifest.missing.is_empty());
    let names: Vec<&str> = dto
        .manifest
        .artifacts
        .iter()
        .map(|a| a.filename.as_str())
        .collect();
    assert!(names.contains(&"compliance-evidence.json"), "{names:?}");
    assert!(names.contains(&"audit-verify.json"), "{names:?}");
    let audit = dto
        .manifest
        .artifacts
        .iter()
        .find(|a| a.filename == "audit-verify.json")
        .expect("the audit verdict");
    assert_eq!(
        audit.verify_status.as_deref(),
        Some("audit chain VERIFIED end-to-end")
    );
    assert_eq!(dto.manifest.operator, "user://acme.example/operator");
    assert_eq!(dto.manifest.org, "acme.example");
    assert!(
        dto.filename.starts_with("genaryx-evidence-") && dto.filename.ends_with(".zip"),
        "{}",
        dto.filename
    );
    assert!(
        !dto.filename.contains(':'),
        "a file name a browser can save: {}",
        dto.filename
    );

    let zip = B64.decode(&dto.zip_base64).expect("base64");
    assert!(zip.starts_with(b"PK"), "a zip archive");
    let digest = format!("sha256:{:x}", Sha256::digest(&zip));

    assert!(dto.journaled, "{:?}", dto.journal_error);
    assert_eq!(dto.journal_error, None);
    let body = std::fs::read_to_string(bus_dir.join("console.ndjson")).expect("journaled");
    let line: Value = serde_json::from_str(body.lines().last().expect("a line")).expect("parses");
    assert_eq!(line["data"]["action"], "console.evidence_built");
    assert_eq!(
        line["data"]["target"],
        digest.as_str(),
        "the journal names the pack by the digest of the bytes handed back"
    );
    assert_eq!(line["data"]["decision"], "allow");
    assert_eq!(
        line["data"]["verify_result"],
        "artifacts:2 missing:0 signed:true"
    );
    let _ = std::fs::remove_dir_all(&bus_dir);
}

/// No paired device is an honestly unsigned pack, never one labelled signed;
/// a broken chain is said to be broken, at its index; and with no bus the
/// missing journal line is reported.
#[tokio::test]
async fn an_unpaired_console_builds_an_unsigned_pack_and_says_it_was_not_journaled() {
    let base = compliance_cloud(json!({"ok": false, "break_index": 3}));
    let money = money_at(&base).await;
    let dto = evidence_build(cloud_only(true), &ready_env().await, &money)
        .await
        .unwrap_or_else(|e| panic!("the pack builds: {e:?}"));
    assert!(!dto.signed, "no device, no signature, and no claim of one");
    assert!(dto.cloud_included);
    let audit = dto
        .manifest
        .artifacts
        .iter()
        .find(|a| a.filename == "audit-verify.json")
        .expect("the audit verdict");
    assert_eq!(
        audit.verify_status.as_deref(),
        Some("audit chain BROKEN at index 3")
    );
    assert!(!dto.journaled);
    assert!(
        dto.journal_error
            .as_deref()
            .is_some_and(|e| e.contains("no live event bus")),
        "{:?}",
        dto.journal_error
    );
}

/// A pack the operator did not ask the Cloud for leaves the Cloud out even
/// with Money ready; with nothing else requested that is no pack.
#[tokio::test]
async fn the_cloud_is_left_out_when_the_operator_did_not_ask_for_it() {
    let base = compliance_cloud(json!({"ok": true}));
    let money = money_at(&base).await;
    match evidence_build(cloud_only(false), &ready_env().await, &money).await {
        Err(EvidenceError::Build { message }) => {
            assert!(message.contains("no artifacts"), "{message}");
        }
        other => panic!("expected a refused empty pack, got {:?}", other.err()),
    }
}

/// A store the journal cannot open costs the journal line, which is
/// reported, and never the pack.
#[tokio::test]
async fn an_unopenable_journal_store_is_reported_and_the_pack_still_returned() {
    let root = scratch("unopenable");
    let file = root.join("not-a-directory");
    std::fs::write(&file, "x").expect("write");
    let base = compliance_cloud(json!({"ok": true}));
    let money = signed_money_at(&base, Some(BusHandle::from_dirs(&file, &root))).await;
    let dto = evidence_build(cloud_only(true), &ready_env().await, &money)
        .await
        .unwrap_or_else(|e| panic!("the pack builds: {e:?}"));
    assert!(dto.signed);
    assert!(!dto.journaled);
    assert!(dto.journal_error.is_some());
    let _ = std::fs::remove_dir_all(&root);
}
