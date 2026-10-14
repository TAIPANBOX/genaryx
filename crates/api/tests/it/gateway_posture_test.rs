//! The gateway's posture through `credentials_posture`, against a stub
//! gateway: its own settings when it reports them, "not reported by this
//! gateway version" when it does not, the keys and identity-map summary, and
//! the price book with its tiers. The stub is the same path-routed one the
//! Money tests use; only the routes differ.

use crate::cloud_stub::spawn_cloud;
use genaryx_api::credentials::commands::CredentialsError;
use genaryx_api::credentials::env::EnvSource;
use genaryx_api::credentials::posture::{GatewayPostureDto, WITHHELD, credentials_posture};
use genaryx_api::credentials::state::{CredentialsClient, CredentialsInner, CredentialsState};
use genaryx_connectors::GatewayClient;
use serde_json::{Value, json};
use std::sync::Arc;

async fn credentials_at(base: &str) -> CredentialsState {
    let state = CredentialsState::pending();
    *state.inner.lock().await = CredentialsInner::Ready(CredentialsClient {
        client: Arc::new(
            GatewayClient::new(base)
                .expect("client")
                .with_admin_key(Some("admin-test".to_string())),
        ),
        source: EnvSource::Taipan {
            name: "forge".to_string(),
        },
        gateway_url: base.to_string(),
    });
    state
}

/// The forge lab's gateway of 2026-10-14 (stack-single v1.1.32): firewall in
/// enforce, PII in shadow, a 5.00 USD run ceiling, the Anthropic wire, client
/// keys on, the identity map in enforce.
fn settings() -> String {
    json!({
        "mode": "enforce", "firewall": "enforce", "dlp": "on", "dlp_pii": "shadow",
        "identity_strict": "enforce", "max_run_budget_usd": 5.0, "require_run_id": true,
        "wardryx_mode": "enforce", "failmode": "closed", "wire": "anthropic",
        "cache": "off", "router": "off", "closure_enabled": false, "client_keys": true
    })
    .to_string()
}

fn keys() -> String {
    json!({
        "strict_mode": "enforce", "identity_map_configured": true, "history_available": false,
        "unauthorized_since_startup": {"attempts": 4, "last_millis": 1_791_985_800_000_i64},
        "keys": [
            {"key_id": "lab", "configured": true, "bound": true, "unit": "lab", "agents": [],
             "since_startup": {"calls": 12, "identity_mismatches": 2}},
            {"key_id": "flint2", "configured": true, "bound": true, "unit": "routers",
             "agents": [], "since_startup": {"calls": 3}},
            {"key_id": "retired", "configured": false, "bound": false, "agents": [],
             "since_startup": {"calls": 0}}
        ]
    })
    .to_string()
}

fn price_book() -> String {
    json!({
        "rows": 61, "built_in_rows": 60, "operator_file": true, "overridden": 0, "added": 1,
        "overridden_models": [], "added_models": ["openai/gpt-4.1-mini"],
        "fallback": {"input_per_mtok_microusd": 15_000_000, "output_per_mtok_microusd": 75_000_000},
        "long_context": [{"model": "gpt-6-astra", "above_prompt_tokens": 200_000,
                          "input_per_mtok_microusd": 4_000_000, "output_per_mtok_microusd": 18_000_000}],
        "long_context_dropped_models": [], "fallback_models_seen": ["anthropic/claude-x"],
        "fallback_models_seen_total": 1, "detail": "61 rows"
    })
    .to_string()
}

async fn posture(routes: Vec<(&'static str, &'static str, String)>) -> GatewayPostureDto {
    let state = credentials_at(&spawn_cloud(routes)).await;
    credentials_posture(&state).await.expect("posture")
}

fn setting(dto: &GatewayPostureDto, key: &str) -> Option<String> {
    dto.settings
        .as_ref()
        .expect("settings section")
        .items
        .iter()
        .find(|i| i.key == key)
        .unwrap_or_else(|| panic!("{key} is not listed"))
        .value
        .clone()
}

#[tokio::test]
async fn the_posture_view_lists_the_gateways_own_settings_when_it_reports_them() {
    let dto = posture(vec![
        ("/v1/settings", "200 OK", settings()),
        ("/v1/keys", "200 OK", keys()),
        ("/v1/price-book", "200 OK", price_book()),
    ])
    .await;
    assert!(dto.notes.is_empty(), "{:?}", dto.notes);
    assert!(dto.settings.as_ref().expect("settings").reported);
    assert_eq!(setting(&dto, "firewall").as_deref(), Some("enforce"));
    assert_eq!(setting(&dto, "dlp_pii").as_deref(), Some("shadow"));
    assert_eq!(setting(&dto, "max_run_budget_usd").as_deref(), Some("5.0"));
    assert_eq!(setting(&dto, "wire").as_deref(), Some("anthropic"));
    assert_eq!(setting(&dto, "client_keys").as_deref(), Some("on"));
    assert_eq!(setting(&dto, "closure_enabled").as_deref(), Some("off"));

    let keys = dto.keys.as_ref().expect("keys");
    assert_eq!(keys.identity_strict, "enforce");
    assert!(keys.identity_map_configured);
    assert_eq!((keys.configured_keys, keys.keys_listed), (2, 3));
    assert_eq!(keys.unauthorized_attempts, 4);

    let book = dto.price_book.as_ref().expect("price book");
    assert_eq!((book.rows, book.built_in_rows), (61, 60));
    assert!(book.operator_file);
    assert_eq!(book.added_models, ["openai/gpt-4.1-mini"]);
    assert_eq!(book.fallback_output_per_mtok_usd, Some(75.0));
    let tiers = book.tiers.as_ref().expect("tiers reported");
    assert_eq!(tiers[0].model, "gpt-6-astra");
    assert_eq!(tiers[0].above_prompt_tokens, 200_000);
    assert_eq!(tiers[0].input_per_mtok_usd, 4.0);
    assert_eq!(book.fallback_models_seen, ["anthropic/claude-x"]);
}

#[tokio::test]
async fn a_gateway_without_the_settings_route_says_not_reported_never_off() {
    let dto = posture(vec![
        ("/v1/keys", "200 OK", keys()),
        ("/v1/price-book", "200 OK", price_book()),
    ])
    .await;
    let settings = dto.settings.as_ref().expect("a 404 is an answer");
    assert!(!settings.reported);
    assert!(settings.items.is_empty(), "no switch may read off or on");
    assert!(dto.keys.is_some() && dto.price_book.is_some());
}

#[tokio::test]
async fn a_gateway_older_than_the_price_book_route_says_not_reported() {
    let dto = posture(vec![
        ("/v1/settings", "200 OK", settings()),
        ("/v1/keys", "200 OK", keys()),
    ])
    .await;
    assert!(dto.price_book.is_none());
    assert_eq!(dto.notes.len(), 1, "{:?}", dto.notes);
    assert!(dto.notes[0].contains("price-book"), "{:?}", dto.notes);
    // An older book without tiers says tiers are not reported, not none.
    let dto = posture(vec![(
        "/v1/price-book",
        "200 OK",
        r#"{"rows":60,"built_in_rows":60}"#.to_string(),
    )])
    .await;
    assert!(dto.price_book.as_ref().expect("book").tiers.is_none());
}

#[tokio::test]
async fn a_secret_the_gateway_echoes_under_another_name_is_never_shown() {
    let mut body: Value = serde_json::from_str(&settings()).expect("json");
    body["admin_key"] = json!("s3cr3t-admin-key");
    body["closure_secret"] = json!("s3cr3t-closure");
    body["upstream_api_key"] = json!("sk-ant-s3cr3t");
    let dto = posture(vec![("/v1/settings", "200 OK", body.to_string())]).await;
    let text = serde_json::to_string(&dto).expect("serialize");
    assert!(
        !text.contains("s3cr3t"),
        "a secret reached the view: {text}"
    );
    assert_eq!(setting(&dto, "admin_key").as_deref(), Some(WITHHELD));
    assert_eq!(setting(&dto, "upstream_api_key").as_deref(), Some(WITHHELD));
}

#[tokio::test]
async fn a_gateway_that_refuses_every_read_is_an_error_not_an_empty_posture() {
    let refused = || {
        (
            "401 Unauthorized",
            r#"{"error":"unauthorized"}"#.to_string(),
        )
    };
    let (s, b) = refused();
    let (k, kb) = refused();
    let (p, pb) = refused();
    let state = credentials_at(&spawn_cloud(vec![
        ("/v1/settings", s, b),
        ("/v1/keys", k, kb),
        ("/v1/price-book", p, pb),
    ]))
    .await;
    let err = credentials_posture(&state).await.expect_err("all refused");
    assert!(
        matches!(
            err,
            CredentialsError::Gateway {
                status: Some(401),
                ..
            }
        ),
        "{err:?}"
    );
    // One refused read costs only its own section.
    let dto = posture(vec![
        (
            "/v1/settings",
            "500 Internal Server Error",
            "{}".to_string(),
        ),
        ("/v1/keys", "200 OK", keys()),
    ])
    .await;
    assert!(dto.settings.is_none() && dto.keys.is_some());
    assert!(
        dto.notes.iter().any(|n| n.contains("/v1/settings")),
        "{:?}",
        dto.notes
    );
}
