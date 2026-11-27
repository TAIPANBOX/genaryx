//! `admission_check` driven through the real gateway client against a stub
//! gateway: what the console reports for a key the gateway knows, one it does
//! not, and every way the read can fail; and that `admission_baseline` refuses
//! before it looks for anything when the gateway leg is not ready.

use crate::cloud_stub::spawn_recording_cloud;
use genaryx_api::admission::AdmissionState;
use genaryx_api::admission::commands::{AdmissionError, admission_baseline, admission_check};
use genaryx_api::admission::env::EnvSource;
use genaryx_api::admission::state::{AdmissionClient, AdmissionInner};
use genaryx_connectors::GatewayClient;
use serde_json::json;
use std::sync::Arc;

const KEYS: &str = "/v1/keys";
const ADMIN_KEY: &str = "test-admin-key-not-a-secret";

fn report() -> String {
    json!({
        "strict_mode": "enforce",
        "identity_map_configured": true,
        "history_available": true,
        "unauthorized_since_startup": {"attempts": 2, "last_millis": 1_700_000_000_000_i64},
        "keys": [
            {
                "key_id": "k-billing",
                "configured": true,
                "bound": true,
                "unit": "billing",
                "agents": ["agent://acme.example/billing/*"],
                "since_startup": {"calls": 7, "identity_mismatches": 1},
                "history": {"calls": 40, "first_seen_millis": 1, "last_seen_millis": 2}
            },
            {
                "key_id": "k-ops",
                "configured": true,
                "bound": false,
                "agents": ["agent://acme.example/ops/runner", "bad*pattern", ""],
                "since_startup": {"calls": 0}
            }
        ]
    })
    .to_string()
}

async fn admission_at(base: &str) -> AdmissionState {
    let state = AdmissionState::pending();
    *state.inner.lock().await = AdmissionInner::Ready(AdmissionClient {
        client: Arc::new(
            GatewayClient::new(base)
                .expect("gateway client")
                .with_admin_key(Some(ADMIN_KEY.to_string())),
        ),
        source: EnvSource::Taipan {
            name: "test".to_string(),
        },
        gateway_url: base.to_string(),
    });
    state
}

fn gateway_error(result: Result<impl std::fmt::Debug, AdmissionError>) -> (Option<u16>, String) {
    match result {
        Err(AdmissionError::Gateway { status, message }) => (status, message),
        other => panic!("expected a Gateway error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_known_key_is_reported_with_its_entry_and_the_map_match() {
    let (base, log) = spawn_recording_cloud(vec![(KEYS, "200 OK", report())]);
    let state = admission_at(&base).await;
    let dto = admission_check(
        "k-billing".to_string(),
        "agent://acme.example/billing/invoicer".to_string(),
        &state,
    )
    .await
    .expect("the report reads");
    assert_eq!(dto.key_id, "k-billing");
    assert_eq!(dto.agent_id, "agent://acme.example/billing/invoicer");
    assert_eq!(dto.strict_mode, "enforce");
    assert!(dto.identity_map_configured);
    assert!(dto.in_map, "the trailing-star pattern binds it");
    let key = dto.key.expect("the gateway knows this key");
    assert!(key.bound);
    assert_eq!(key.unit.as_deref(), Some("billing"));
    assert_eq!(key.since_startup.calls, 7);
    assert_eq!(key.history.map(|h| h.calls), Some(40));

    let seen = log.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].path, KEYS);
    assert_eq!(
        seen[0].header("authorization"),
        Some(format!("Bearer {ADMIN_KEY}").as_str()),
        "the console's admin key reaches the gateway"
    );
}

#[tokio::test]
async fn an_unknown_key_is_none_and_an_unbound_agent_is_not_in_the_map() {
    let (base, _log) = spawn_recording_cloud(vec![(KEYS, "200 OK", report())]);
    let state = admission_at(&base).await;
    let dto = admission_check(
        "k-nobody".to_string(),
        "agent://acme.example/ops/other".to_string(),
        &state,
    )
    .await
    .expect("the report reads");
    assert!(dto.key.is_none(), "a key the gateway does not list");
    assert!(
        !dto.in_map,
        "a literal pattern for another agent and malformed patterns bind nothing"
    );

    let literal = admission_check(
        "k-ops".to_string(),
        "agent://acme.example/ops/runner".to_string(),
        &state,
    )
    .await
    .expect("the report reads");
    assert!(literal.in_map, "an exact literal binds");
    assert!(!literal.key.expect("listed").bound);
}

#[tokio::test]
async fn a_refused_key_names_the_key_variables_not_the_network() {
    let (base, _log) = spawn_recording_cloud(vec![(
        KEYS,
        "401 Unauthorized",
        r#"{"error":"unauthorized"}"#.to_string(),
    )]);
    let state = admission_at(&base).await;
    let (status, message) =
        gateway_error(admission_check("k".to_string(), "agent://a/b".to_string(), &state).await);
    assert_eq!(status, Some(401));
    assert!(message.contains("TOKENFUSE_GATEWAY_ADMIN_KEY"), "{message}");
    assert!(message.contains("TOKENFUSE_ADMIN_KEYS"), "{message}");
}

#[tokio::test]
async fn a_gateway_that_requires_admin_keys_says_so() {
    let (base, _log) = spawn_recording_cloud(vec![(
        KEYS,
        "403 Forbidden",
        r#"{"error":"admin_keys_required"}"#.to_string(),
    )]);
    let state = admission_at(&base).await;
    let (status, message) =
        gateway_error(admission_check("k".to_string(), "agent://a/b".to_string(), &state).await);
    assert_eq!(status, Some(403));
    assert!(message.contains("non-loopback"), "{message}");
}

#[tokio::test]
async fn any_other_status_carries_the_gateways_own_body() {
    let (base, _log) = spawn_recording_cloud(vec![(
        KEYS,
        "500 Internal Server Error",
        r#"{"error":"store locked"}"#.to_string(),
    )]);
    let state = admission_at(&base).await;
    let (status, message) =
        gateway_error(admission_check("k".to_string(), "agent://a/b".to_string(), &state).await);
    assert_eq!(status, Some(500));
    assert_eq!(message, r#"{"error":"store locked"}"#);
}

#[tokio::test]
async fn a_report_of_the_wrong_shape_is_a_shape_error_without_a_status() {
    let (base, _log) = spawn_recording_cloud(vec![(KEYS, "200 OK", r#"{"keys":"no"}"#.to_string())]);
    let state = admission_at(&base).await;
    let (status, message) =
        gateway_error(admission_check("k".to_string(), "agent://a/b".to_string(), &state).await);
    assert_eq!(status, None);
    assert!(
        message.starts_with("unexpected response shape from the gateway: "),
        "{message}"
    );
}

#[tokio::test]
async fn a_gateway_nobody_answers_at_is_a_transport_error() {
    // Bind, learn the port, and close it again: nothing listens there.
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.local_addr().expect("addr").port()
    };
    let state = admission_at(&format!("http://127.0.0.1:{port}")).await;
    let (status, message) =
        gateway_error(admission_check("k".to_string(), "agent://a/b".to_string(), &state).await);
    assert_eq!(status, None);
    assert!(
        message.starts_with("could not reach the gateway: "),
        "{message}"
    );
}

#[tokio::test]
async fn a_gateway_leg_that_is_not_ready_refuses_both_commands_by_its_state() {
    let pending = AdmissionState::pending();
    assert!(matches!(
        admission_check("k".to_string(), "agent://a/b".to_string(), &pending).await,
        Err(AdmissionError::Bootstrapping)
    ));

    let none = AdmissionState::pending();
    *none.inner.lock().await = AdmissionInner::NoEnvironment;
    assert!(matches!(
        admission_check("k".to_string(), "agent://a/b".to_string(), &none).await,
        Err(AdmissionError::NoEnvironment)
    ));

    let down = AdmissionState::pending();
    *down.inner.lock().await = AdmissionInner::Unreachable {
        source: EnvSource::Taipan {
            name: "test".to_string(),
        },
        gateway_url: "http://127.0.0.1:1".to_string(),
        reason: "connection refused".to_string(),
    };
    match admission_check("k".to_string(), "agent://a/b".to_string(), &down).await {
        Err(AdmissionError::Unreachable { reason }) => assert_eq!(reason, "connection refused"),
        other => panic!("expected Unreachable, got {other:?}"),
    }

    // The baseline is refused on the same state, before it looks for a
    // verdryx binary or database and long before it could spend anything.
    for state in [&pending, &none] {
        let result = admission_baseline(
            "/tmp/evalset.jsonl".to_string(),
            "claude-test".to_string(),
            "agent://a/b".to_string(),
            "test-key-not-a-secret".to_string(),
            state,
        )
        .await;
        assert!(
            matches!(
                result,
                Err(AdmissionError::Bootstrapping | AdmissionError::NoEnvironment)
            ),
            "{result:?}"
        );
    }
    match admission_baseline(
        "/tmp/evalset.jsonl".to_string(),
        "claude-test".to_string(),
        "agent://a/b".to_string(),
        "test-key-not-a-secret".to_string(),
        &down,
    )
    .await
    {
        Err(AdmissionError::Unreachable { reason }) => assert_eq!(reason, "connection refused"),
        other => panic!("expected Unreachable, got {other:?}"),
    }
}
