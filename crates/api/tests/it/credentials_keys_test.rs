//! The Credentials panel's key report, read through the real
//! `credentials_keys` against a stub gateway: the report as the gateway sent
//! it, and each refusal named in the operator's terms.

use crate::cloud_stub::spawn_recording_cloud;
use genaryx_api::credentials::commands::{
    CredentialsError, CredentialsStatusDto, credentials_keys, credentials_status,
};
use genaryx_api::credentials::env::EnvSource;
use genaryx_api::credentials::state::{CredentialsClient, CredentialsInner, CredentialsState};
use genaryx_connectors::GatewayClient;
use serde_json::json;
use std::sync::Arc;

const KEYS: &str = "/v1/keys";

async fn credentials_at(base: &str) -> CredentialsState {
    let state = CredentialsState::pending();
    *state.inner.lock().await = CredentialsInner::Ready(CredentialsClient {
        client: Arc::new(
            GatewayClient::new(base)
                .expect("client")
                .with_admin_key(Some("test-admin-key-not-a-secret".to_string())),
        ),
        source: EnvSource::Taipan {
            name: "lab".to_string(),
        },
        gateway_url: base.to_string(),
    });
    state
}

fn gateway_error<T: std::fmt::Debug>(result: Result<T, CredentialsError>) -> (Option<u16>, String) {
    match result {
        Err(CredentialsError::Gateway { status, message }) => (status, message),
        other => panic!("expected a Gateway error, got {other:?}"),
    }
}

#[tokio::test]
async fn the_key_report_is_the_gateways_own() {
    let report = json!({
        "strict_mode": "warn", "identity_map_configured": false, "history_available": false,
        "unauthorized_since_startup": {"attempts": 3, "last_millis": null},
        "keys": [{"key_id": "lab", "configured": true, "bound": false,
                  "since_startup": {"calls": 12, "identity_mismatches": 2}}]
    })
    .to_string();
    let (base, log) = spawn_recording_cloud(vec![(KEYS, "200 OK", report)]);
    let state = credentials_at(&base).await;
    let got = credentials_keys(&state).await.expect("the report");
    assert_eq!(got.strict_mode, "warn");
    assert!(!got.identity_map_configured);
    assert_eq!(got.unauthorized_since_startup.attempts, 3);
    assert_eq!(got.unauthorized_since_startup.last_millis, None);
    assert_eq!(got.keys.len(), 1);
    assert_eq!(got.keys[0].key_id, "lab");
    assert_eq!(got.keys[0].since_startup.identity_mismatches, 2);
    assert_eq!(
        log.lock().unwrap()[0].header("authorization"),
        Some("Bearer test-admin-key-not-a-secret")
    );
}

#[tokio::test]
async fn each_refusal_is_named_in_the_operators_terms() {
    let cases = [
        (
            "401 Unauthorized",
            r#"{"error":"unauthorized"}"#,
            Some(401),
            "TOKENFUSE_GATEWAY_ADMIN_KEY",
        ),
        (
            "403 Forbidden",
            r#"{"error":"admin_keys_required"}"#,
            Some(403),
            "non-loopback",
        ),
        ("403 Forbidden", r#"{"error":"other"}"#, Some(403), "other"),
        ("200 OK", r#"{"keys":7}"#, None, "unexpected response shape"),
    ];
    for (status_line, body, want_status, want_text) in cases {
        let (base, _log) = spawn_recording_cloud(vec![(KEYS, status_line, body.to_string())]);
        let state = credentials_at(&base).await;
        let (status, message) = gateway_error(credentials_keys(&state).await);
        assert_eq!(status, want_status, "{status_line} {body}");
        assert!(message.contains(want_text), "{status_line}: {message}");
    }
}

#[tokio::test]
async fn a_panel_that_is_not_ready_says_which_state_it_is_in() {
    let pending = CredentialsState::pending();
    assert!(matches!(
        credentials_keys(&pending).await,
        Err(CredentialsError::Bootstrapping)
    ));
    assert!(matches!(
        credentials_status(&pending).await,
        Ok(CredentialsStatusDto::Bootstrapping)
    ));
    let none = CredentialsState::pending();
    *none.inner.lock().await = CredentialsInner::NoEnvironment;
    assert!(matches!(
        credentials_keys(&none).await,
        Err(CredentialsError::NoEnvironment)
    ));
    let ready = credentials_at("http://127.0.0.1:9").await;
    match credentials_status(&ready).await {
        Ok(CredentialsStatusDto::Ready { gateway_url, .. }) => {
            assert_eq!(gateway_url, "http://127.0.0.1:9");
        }
        other => panic!("expected Ready, got {other:?}"),
    }
}
