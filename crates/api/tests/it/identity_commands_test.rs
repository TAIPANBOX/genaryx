//! The Identity panel's reads and its rescan, driven through the real commands
//! against a stub idryx: what each read carries, how a refused read reaches
//! the operator, and that a rescan with no binary says so rather than running.

use crate::cloud_stub::spawn_recording_cloud;
use genaryx_api::identity::IdentityState;
use genaryx_api::identity::commands::{
    IdentityError, IdentityStatusDto, identity_list_alerts, identity_list_identities,
    identity_list_remediations, identity_rescan, identity_status,
};
use genaryx_api::identity::env::EnvSource;
use genaryx_api::identity::state::{IdentityClient, IdentityInner};
use genaryx_connectors::IdryxClient;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

const AGENT: &str = "agent://acme.example/agents/a";

async fn identity_at(base: &str, idryx_bin: Option<PathBuf>) -> IdentityState {
    let state = IdentityState::pending();
    *state.inner.lock().await = IdentityInner::Ready(IdentityClient {
        client: Arc::new(IdryxClient::new(base).expect("idryx client")),
        source: EnvSource::Taipan {
            name: "test".to_string(),
        },
        idryx_url: base.to_string(),
        idryx_bin,
        rescan_loads: vec![("wardryx".to_string(), PathBuf::from("/tmp/wardryx.ndjson"))],
    });
    state
}

fn identities() -> String {
    json!([{
        "id": AGENT, "type": "agent", "privileged": true, "source": "agents",
        "owner": "ops", "on_behalf_of": ["user://acme.example/owner"],
        "permissions": [{"name": "s3:*", "admin": true, "used": false}],
        "events": 4, "alerts": 1
    }])
    .to_string()
}

fn alerts() -> String {
    json!([{
        "detector": "runaway_agent", "identity": AGENT, "severity": "high",
        "time": "2026-11-20T10:00:00Z", "summary": "spend rose"
    }])
    .to_string()
}

fn remediations() -> String {
    json!([{
        "identity": AGENT, "kind": "right_size",
        "explanation": "s3:* was never used", "code": "drop s3:*"
    }])
    .to_string()
}

#[tokio::test]
async fn each_read_carries_what_idryx_answered() {
    let (base, log) = spawn_recording_cloud(vec![
        ("/api/identities", "200 OK", identities()),
        ("/api/alerts", "200 OK", alerts()),
        ("/api/remediations", "200 OK", remediations()),
    ]);
    let state = identity_at(&base, None).await;

    let ids = identity_list_identities(&state).await.expect("identities");
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0].id, AGENT);
    assert!(ids[0].privileged);
    assert_eq!(ids[0].on_behalf_of, vec!["user://acme.example/owner".to_string()]);

    let found = identity_list_alerts(&state).await.expect("alerts");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].detector, "runaway_agent");
    assert_eq!(found[0].severity, "high");

    let fixes = identity_list_remediations(&state).await.expect("remediations");
    assert_eq!(fixes.len(), 1);
    assert_eq!(fixes[0].kind, "right_size");
    assert_eq!(fixes[0].created_at, "", "absent on the wire, not invented");

    let paths: Vec<String> = log.lock().unwrap().iter().map(|s| s.path.clone()).collect();
    assert_eq!(paths, ["/api/identities", "/api/alerts", "/api/remediations"]);
}

#[tokio::test]
async fn a_refused_read_carries_idryxs_status_and_body() {
    let (base, _log) = spawn_recording_cloud(vec![(
        "/api/alerts",
        "503 Service Unavailable",
        r#"{"error":"snapshot loading"}"#.to_string(),
    )]);
    let state = identity_at(&base, None).await;
    match identity_list_alerts(&state).await {
        Err(IdentityError::Idryx { status, message }) => {
            assert_eq!(status, Some(503));
            assert_eq!(message, r#"{"error":"snapshot loading"}"#);
        }
        other => panic!("expected an Idryx error, got {other:?}"),
    }
    // A route the stub does not have answers 404, which is a status too.
    match identity_list_identities(&state).await {
        Err(IdentityError::Idryx { status, .. }) => assert_eq!(status, Some(404)),
        other => panic!("expected an Idryx error, got {other:?}"),
    }
}

#[tokio::test]
async fn an_answer_of_the_wrong_shape_is_a_shape_error() {
    let (base, _log) = spawn_recording_cloud(vec![(
        "/api/remediations",
        "200 OK",
        r#"{"not":"a list"}"#.to_string(),
    )]);
    let state = identity_at(&base, None).await;
    match identity_list_remediations(&state).await {
        Err(IdentityError::Idryx { status, message }) => {
            assert_eq!(status, None);
            assert!(
                message.starts_with("unexpected response shape from idryx: "),
                "{message}"
            );
        }
        other => panic!("expected an Idryx error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_rescan_with_no_binary_is_unavailable_and_one_that_cannot_spawn_says_so() {
    let (base, log) = spawn_recording_cloud(vec![]);
    let without = identity_at(&base, None).await;
    assert!(matches!(
        identity_rescan(&without).await,
        Err(IdentityError::RescanUnavailable)
    ));

    let missing = identity_at(
        &base,
        Some(PathBuf::from("/nonexistent/genaryx-identity-test/idryx")),
    )
    .await;
    match identity_rescan(&missing).await {
        Err(IdentityError::Idryx { status, message }) => {
            assert_eq!(status, None);
            assert!(message.starts_with("idryx detect failed: spawn "), "{message}");
        }
        other => panic!("expected an Idryx error, got {other:?}"),
    }
    assert!(log.lock().unwrap().is_empty(), "a rescan reads no REST route");
}

#[tokio::test]
async fn a_panel_that_is_not_ready_refuses_each_command_by_its_state() {
    let pending = IdentityState::pending();
    assert!(matches!(
        identity_list_identities(&pending).await,
        Err(IdentityError::Bootstrapping)
    ));
    assert!(matches!(
        identity_status(&pending).await,
        Ok(IdentityStatusDto::Bootstrapping)
    ));

    let none = IdentityState::pending();
    *none.inner.lock().await = IdentityInner::NoEnvironment;
    assert!(matches!(
        identity_list_alerts(&none).await,
        Err(IdentityError::NoEnvironment)
    ));
    assert!(matches!(
        identity_rescan(&none).await,
        Err(IdentityError::NoEnvironment)
    ));

    let down = IdentityState::pending();
    *down.inner.lock().await = IdentityInner::Unreachable {
        source: EnvSource::Taipan {
            name: "test".to_string(),
        },
        idryx_url: "http://127.0.0.1:1".to_string(),
        reason: "healthz failed".to_string(),
    };
    match identity_list_remediations(&down).await {
        Err(IdentityError::Unreachable { reason }) => assert_eq!(reason, "healthz failed"),
        other => panic!("expected Unreachable, got {other:?}"),
    }
    match identity_status(&down).await {
        Ok(IdentityStatusDto::Unreachable { reason, .. }) => {
            assert_eq!(reason, "healthz failed");
        }
        other => panic!("expected Unreachable, got {other:?}"),
    }

    let ready = identity_at("http://127.0.0.1:9", Some(PathBuf::from("/opt/idryx"))).await;
    match identity_status(&ready).await {
        Ok(IdentityStatusDto::Ready {
            idryx_url,
            rescan_available,
            ..
        }) => {
            assert_eq!(idryx_url, "http://127.0.0.1:9");
            assert!(rescan_available);
        }
        other => panic!("expected Ready, got {other:?}"),
    }
}
