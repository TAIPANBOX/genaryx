//! Opening a held call's sealed preview through the real
//! `policy_approval_preview`, against a stub wardryx, with the ciphertexts
//! wardryx's own sealer made for the interop test (CLAUDE.md invariant 48).
//!
//! The preview is shown only when it opens under the console key AND the
//! binding the hold records; a tampered, foreign or mismatched ciphertext, the
//! wrong key, or a preview that is not the one the hold names is refused, and
//! a refused preview is never remembered as seen.

use crate::cloud_stub::spawn_recording_cloud;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use genaryx_api::policy::PolicyState;
use genaryx_api::policy::commands::PolicyError;
use genaryx_api::policy::env::EnvSource;
use genaryx_api::policy::preview::{
    PreviewKeyState, STATE_NONE, SeenPreviews, generate_key_file, policy_approval_preview,
};
use genaryx_api::policy::state::{PolicyClient, PolicyInner};
use genaryx_connectors::WardryxClient;
use genaryx_signing::preview::{KEM_X25519, KEM_XWING, PreviewKey};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Ciphertexts wardryx's `internal/previewseal` made with throwaway test keys.
const INTEROP: &str = include_str!("../../../signing/testdata/preview-interop.json");

const OPERATOR: &str = "user://wardryx.local/test";
const HOLDS: &str = "/v1/approvals";
const PREVIEW: &str = "/v1/approvals/ap_1/preview";

struct Vector {
    kem: u16,
    private: String,
    org: String,
    tcd: String,
    pvd: String,
    ruleset: String,
    sealed_b64: String,
    document: String,
}

fn vector_from(x: &Value) -> Vector {
    let s = |k: &str| x[k].as_str().expect(k).to_string();
    Vector {
        kem: if s("suite").starts_with("kem=0x647a") {
            KEM_XWING
        } else {
            KEM_X25519
        },
        private: s("private"),
        org: s("org"),
        tcd: s("tool_call_digest"),
        pvd: s("pvd"),
        ruleset: s("ruleset"),
        sealed_b64: s("sealed_b64"),
        document: s("document"),
    }
}

fn vectors() -> Vec<Vector> {
    let v: Value = serde_json::from_str(INTEROP).expect("vectors parse");
    v["vectors"]
        .as_array()
        .expect("vectors")
        .iter()
        .map(vector_from)
        .collect()
}

fn mismatch_vector() -> Vector {
    let v: Value = serde_json::from_str(INTEROP).expect("vectors parse");
    vector_from(&v["mismatch"])
}

fn key_of(v: &Vector) -> PreviewKeyState {
    PreviewKeyState::Ready(PreviewKey::parse(&v.private).expect("the vector's key parses"))
}

fn hold_list(context: Value) -> String {
    json!([{
        "approval_id": "ap_1",
        "agent_id": "agent://acme/agents/a",
        "run_id": "run-1",
        "requested_at": "2026-11-12T00:00:00Z",
        "pending": true,
        "context": context,
    }])
    .to_string()
}

fn sealed_hold(v: &Vector, org: &str) -> String {
    hold_list(json!({
        "org": org,
        "tool_call": {"name": "t", "target": "s", "digest": v.tcd},
        "preview": {"state": "sealed", "pvd": v.pvd, "ruleset": v.ruleset},
    }))
}

fn preview_answer(v: &Vector, tcd: &str, pvd: &str, sealed: Option<&str>) -> String {
    json!({
        "approval_id": "ap_1",
        "state": "sealed",
        "tool_call_digest": tcd,
        "pvd": pvd,
        "ruleset": v.ruleset,
        "sealed": sealed,
        "expires_at": "2026-11-12T00:15:00Z",
    })
    .to_string()
}

async fn policy_at(base: &str) -> PolicyState {
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
        bus: None,
    });
    state
}

/// Serve `holds` and `preview`, ask for `ap_1`'s preview with `key`.
async fn open_with(
    holds: String,
    preview: Option<String>,
    key: &PreviewKeyState,
    seen: &SeenPreviews,
) -> Result<genaryx_api::policy::preview::PreviewDto, PolicyError> {
    let mut routes = vec![(HOLDS, "200 OK", holds)];
    if let Some(body) = preview {
        routes.push((PREVIEW, "200 OK", body));
    }
    let (base, _log) = spawn_recording_cloud(routes);
    let state = policy_at(&base).await;
    policy_approval_preview("ap_1".to_string(), &state, key, seen).await
}

fn preview_reason(result: Result<genaryx_api::policy::preview::PreviewDto, PolicyError>) -> String {
    match result {
        Err(PolicyError::Preview { reason }) => reason,
        other => panic!("expected a Preview refusal, got {other:?}"),
    }
}

/// `sealed_b64` with one byte of its ciphertext flipped.
fn tampered(sealed_b64: &str, at: usize) -> String {
    let mut bytes = STANDARD.decode(sealed_b64).expect("base64");
    let i = at.min(bytes.len() - 1);
    bytes[i] ^= 0x01;
    STANDARD.encode(bytes)
}

const NOT_OPENED: &str =
    "the sealed preview does not open under the console key and the hold it names";

#[tokio::test]
async fn a_sealed_preview_opens_under_each_kem_and_is_remembered_as_seen() {
    for v in vectors() {
        let seen = SeenPreviews::default();
        let dto = open_with(
            sealed_hold(&v, &v.org),
            Some(preview_answer(&v, &v.tcd, &v.pvd, Some(&v.sealed_b64))),
            &key_of(&v),
            &seen,
        )
        .await
        .expect("the interop ciphertext opens");
        assert_eq!(dto.approval_id, "ap_1");
        assert_eq!(dto.state, "sealed");
        assert_eq!(dto.pvd.as_deref(), Some(v.pvd.as_str()));
        assert_eq!(dto.ruleset.as_deref(), Some(v.ruleset.as_str()));
        assert_eq!(dto.expires_at.as_deref(), Some("2026-11-12T00:15:00Z"));
        let want: Value = serde_json::from_str(&v.document).expect("the vector's document");
        assert_eq!(dto.document, Some(want));
        assert!(seen.confirms(OPERATOR, "ap_1", &v.pvd), "the open is recorded");
        assert!(!seen.confirms("user://wardryx.local/someone-else", "ap_1", &v.pvd));
    }
}

#[tokio::test]
async fn a_tampered_ciphertext_is_refused_and_not_remembered() {
    for v in vectors() {
        let len = STANDARD.decode(&v.sealed_b64).expect("base64").len();
        // A flipped byte in the encapsulated key, in the middle, and in the
        // tag at the very end: each must fail to open.
        for at in [0, len / 2, len - 1] {
            let seen = SeenPreviews::default();
            let forged = tampered(&v.sealed_b64, at);
            let reason = preview_reason(
                open_with(
                    sealed_hold(&v, &v.org),
                    Some(preview_answer(&v, &v.tcd, &v.pvd, Some(&forged))),
                    &key_of(&v),
                    &seen,
                )
                .await,
            );
            assert_eq!(reason, NOT_OPENED, "byte {at}");
            assert!(!seen.confirms(OPERATOR, "ap_1", &v.pvd));
        }
    }
}

#[tokio::test]
async fn a_preview_sealed_for_another_org_does_not_open_here() {
    for v in vectors() {
        let seen = SeenPreviews::default();
        let reason = preview_reason(
            open_with(
                sealed_hold(&v, "another-org"),
                Some(preview_answer(&v, &v.tcd, &v.pvd, Some(&v.sealed_b64))),
                &key_of(&v),
                &seen,
            )
            .await,
        );
        assert_eq!(reason, NOT_OPENED);
        assert!(!seen.confirms(OPERATOR, "ap_1", &v.pvd));
    }
}

#[tokio::test]
async fn the_wrong_key_does_not_open_a_preview() {
    for v in vectors() {
        let (private, _recipient) = PreviewKey::generate(v.kem).expect("a fresh key");
        let other = PreviewKeyState::Ready(PreviewKey::parse(&private).expect("parses"));
        let seen = SeenPreviews::default();
        let reason = preview_reason(
            open_with(
                sealed_hold(&v, &v.org),
                Some(preview_answer(&v, &v.tcd, &v.pvd, Some(&v.sealed_b64))),
                &other,
                &seen,
            )
            .await,
        );
        assert_eq!(reason, NOT_OPENED);
        assert!(!seen.confirms(OPERATOR, "ap_1", &v.pvd));
    }
}

#[tokio::test]
async fn a_ciphertext_that_opens_to_another_document_is_refused_by_its_digest() {
    let m = mismatch_vector();
    let seen = SeenPreviews::default();
    let reason = preview_reason(
        open_with(
            sealed_hold(&m, &m.org),
            Some(preview_answer(&m, &m.tcd, &m.pvd, Some(&m.sealed_b64))),
            &key_of(&m),
            &seen,
        )
        .await,
    );
    assert_eq!(reason, "the opened preview is not the one the hold records");
    assert!(!seen.confirms(OPERATOR, "ap_1", &m.pvd));
}

#[tokio::test]
async fn a_preview_for_another_digest_than_the_hold_records_is_refused_before_opening() {
    for v in vectors() {
        let other = "3".repeat(64);
        for (tcd, pvd) in [(v.tcd.as_str(), other.as_str()), (other.as_str(), v.pvd.as_str())] {
            let seen = SeenPreviews::default();
            let reason = preview_reason(
                open_with(
                    sealed_hold(&v, &v.org),
                    Some(preview_answer(&v, tcd, pvd, Some(&v.sealed_b64))),
                    &key_of(&v),
                    &seen,
                )
                .await,
            );
            assert_eq!(
                reason,
                "the preview wardryx sent is not the one the hold records"
            );
            assert!(!seen.confirms(OPERATOR, "ap_1", &v.pvd));
            assert!(!seen.confirms(OPERATOR, "ap_1", pvd));
        }
    }
}

#[tokio::test]
async fn a_hold_whose_digests_are_not_digests_is_refused_without_fetching() {
    let all = vectors();
    let v = &all[0];
    for (tcd, pvd) in [
        (v.tcd.clone(), "not-a-digest".to_string()),
        (v.tcd.to_uppercase(), v.pvd.clone()),
    ] {
        let holds = hold_list(json!({
            "org": v.org,
            "tool_call": {"name": "t", "digest": tcd},
            "preview": {"state": "sealed", "pvd": pvd, "ruleset": v.ruleset},
        }));
        let (base, log) = spawn_recording_cloud(vec![(HOLDS, "200 OK", holds)]);
        let state = policy_at(&base).await;
        let reason = preview_reason(
            policy_approval_preview("ap_1".to_string(), &state, &key_of(v), &SeenPreviews::default())
                .await,
        );
        assert_eq!(
            reason,
            "the hold does not record a well-formed call or preview digest"
        );
        let paths: Vec<String> = log.lock().unwrap().iter().map(|s| s.path.clone()).collect();
        assert_eq!(paths, vec![HOLDS.to_string()], "the preview is never fetched");
    }
}

#[tokio::test]
async fn a_sealed_answer_with_no_readable_ciphertext_is_refused() {
    let all = vectors();
    let v = &all[0];
    for sealed in [None, Some("%%% not base64 %%%")] {
        let reason = preview_reason(
            open_with(
                sealed_hold(v, &v.org),
                Some(preview_answer(v, &v.tcd, &v.pvd, sealed)),
                &key_of(v),
                &SeenPreviews::default(),
            )
            .await,
        );
        assert_eq!(
            reason,
            "wardryx sent a sealed preview with no readable ciphertext"
        );
    }
}

#[tokio::test]
async fn without_a_usable_key_a_sealed_preview_is_refused_and_says_why() {
    let all = vectors();
    let v = &all[0];
    let reason = preview_reason(
        open_with(
            sealed_hold(v, &v.org),
            None,
            &PreviewKeyState::NotConfigured,
            &SeenPreviews::default(),
        )
        .await,
    );
    assert!(reason.contains("GENARYX_PREVIEW_KEY_FILE"), "{reason}");
    let reason = preview_reason(
        open_with(
            sealed_hold(v, &v.org),
            None,
            &PreviewKeyState::Failed("/etc/genaryx/preview.key: unreadable".to_string()),
            &SeenPreviews::default(),
        )
        .await,
    );
    assert_eq!(
        reason,
        "the preview key file cannot be used: /etc/genaryx/preview.key: unreadable"
    );
}

#[tokio::test]
async fn a_hold_with_no_sealed_preview_reports_its_state_and_opens_nothing() {
    let all = vectors();
    let v = &all[0];
    for (context, want) in [
        (
            json!({"tool_call": {"name": "t", "digest": v.tcd}, "preview": {"state": "truncated"}}),
            "truncated",
        ),
        (json!({"tool_call": {"name": "t", "digest": v.tcd}}), STATE_NONE),
    ] {
        let seen = SeenPreviews::default();
        let dto = open_with(hold_list(context), None, &key_of(v), &seen)
            .await
            .expect("a state, not an error");
        assert_eq!(dto.state, want);
        assert_eq!(dto.pvd, None);
        assert_eq!(dto.ruleset, None);
        assert_eq!(dto.expires_at, None);
        assert_eq!(dto.document, None);
    }
}

#[tokio::test]
async fn an_unknown_hold_and_a_gone_preview_are_wardryx_answers() {
    let all = vectors();
    let v = &all[0];
    let (base, _log) = spawn_recording_cloud(vec![(HOLDS, "200 OK", sealed_hold(v, &v.org))]);
    let state = policy_at(&base).await;
    match policy_approval_preview(
        "ap_unknown".to_string(),
        &state,
        &key_of(v),
        &SeenPreviews::default(),
    )
    .await
    {
        Err(PolicyError::Wardryx { status, message }) => {
            assert_eq!(status, Some(404));
            assert_eq!(message, "approval not found");
        }
        other => panic!("expected a 404, got {other:?}"),
    }

    let (base, _log) = spawn_recording_cloud(vec![
        (HOLDS, "200 OK", sealed_hold(v, &v.org)),
        (PREVIEW, "410 Gone", r#"{"error":"gone"}"#.to_string()),
    ]);
    let state = policy_at(&base).await;
    match policy_approval_preview("ap_1".to_string(), &state, &key_of(v), &SeenPreviews::default())
        .await
    {
        Err(PolicyError::Wardryx { status, .. }) => assert_eq!(status, Some(410)),
        other => panic!("expected a 410, got {other:?}"),
    }
}

#[tokio::test]
async fn a_console_not_ready_refuses_before_any_call() {
    let state = PolicyState::pending();
    let result = policy_approval_preview(
        "ap_1".to_string(),
        &state,
        &PreviewKeyState::NotConfigured,
        &SeenPreviews::default(),
    )
    .await;
    assert!(matches!(result, Err(PolicyError::Bootstrapping)));
}

#[test]
fn an_open_record_is_bounded_and_drops_the_oldest_first() {
    let seen = SeenPreviews::default();
    let pvd = "4".repeat(64);
    seen.record(OPERATOR, "ap_first", &pvd);
    // Strictly older than every later open, whatever the clock's resolution.
    std::thread::sleep(std::time::Duration::from_millis(5));
    for n in 0..4096 {
        seen.record(OPERATOR, &format!("ap_{n}"), &pvd);
    }
    assert!(
        !seen.confirms(OPERATOR, "ap_first", &pvd),
        "the oldest open is the one forgotten at the cap"
    );
    assert!(seen.confirms(OPERATOR, "ap_4095", &pvd));
    assert!(seen.confirms(OPERATOR, "ap_1", &pvd));
}

fn scratch(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-preview-itest-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn a_key_file_loads_or_says_what_is_wrong_with_it() {
    assert!(matches!(
        PreviewKeyState::load(None),
        PreviewKeyState::NotConfigured
    ));

    let dir = scratch("load");
    let missing = dir.join("absent.key");
    match PreviewKeyState::load(Some(&missing)) {
        PreviewKeyState::Failed(reason) => {
            assert!(reason.starts_with(&missing.display().to_string()), "{reason}");
        }
        _ => panic!("a missing file must be Failed"),
    }

    let garbage = dir.join("garbage.key");
    std::fs::write(&garbage, "not a preview key\n").expect("write");
    match PreviewKeyState::load(Some(&garbage)) {
        PreviewKeyState::Failed(reason) => {
            assert!(
                reason.contains("the preview key is not usable"),
                "{reason}"
            );
            assert!(!reason.contains("not a preview key"), "no key text echoed");
        }
        _ => panic!("garbage must be Failed"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_generated_key_file_is_private_loads_and_matches_its_recipient() {
    let dir = scratch("generate");
    for (name, x25519) in [("x25519.key", true), ("xwing.key", false)] {
        let path = dir.join(name);
        let recipient = generate_key_file(&path, x25519).expect("generated");
        let want_prefix = if x25519 { "hpke:0020:" } else { "hpke:647a:" };
        assert!(recipient.starts_with(want_prefix), "{recipient}");
        match PreviewKeyState::load(Some(&path)) {
            PreviewKeyState::Ready(key) => assert_eq!(key.recipient(), recipient),
            _ => panic!("a freshly generated key file must load"),
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
            assert_eq!(mode & 0o777, 0o400, "readable by its owner only");
        }
        // Never overwrites an existing key.
        let err = generate_key_file(&path, x25519).expect_err("the file exists");
        assert!(err.starts_with(&path.display().to_string()), "{err}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
