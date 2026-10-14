//! Which gateways the Cloud still hears from, through `money_gateways`
//! against a stub Cloud: a gateway that stopped pushing reads silent, and a
//! Cloud without `GET /v1/gateways` is "not reported", never "none silent".

use crate::cloud_stub::{money_at, spawn_cloud};
use genaryx_api::money::gateways::{GatewayState, money_gateways};
use serde_json::json;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[tokio::test]
async fn a_gateway_that_stopped_pushing_reads_silent_and_leads_the_list() {
    let now = now_ms();
    let body = json!([
        {"site": "forge", "spent_microusd": 5168, "calls": 12, "tool_calls": 0,
         "pushes": 930, "first_push_millis": now - 86_400_000,
         "last_push_millis": now - 20_000, "last_record_millis": now - 25_000},
        {"site": "forge-openai", "spent_microusd": 2584, "calls": 4, "tool_calls": 0,
         "pushes": 410, "first_push_millis": now - 86_400_000,
         "last_push_millis": now - 47 * 60_000, "last_record_millis": now - 48 * 60_000},
        {"site": "unnamed", "spent_microusd": 0, "calls": 0, "tool_calls": 0,
         "pushes": 3, "first_push_millis": now - 120_000,
         "last_push_millis": now - 90_000, "last_record_millis": 0},
    ])
    .to_string();
    let state = money_at(&spawn_cloud(vec![("/v1/gateways", "200 OK", body)])).await;
    let dto = money_gateways(&state).await.expect("gateways");
    assert!(dto.reported);
    assert_eq!(dto.silent_after_secs, 300);
    assert_eq!(dto.gateways.len(), 3);

    let first = &dto.gateways[0];
    assert_eq!(first.site, "forge-openai", "the silent gateway leads");
    assert_eq!(first.state, GatewayState::Silent);
    assert!(
        (2_800..=2_840).contains(&first.last_push_age_secs),
        "about 47 minutes, got {}",
        first.last_push_age_secs
    );
    assert_eq!(first.calls, 4);
    assert_eq!(first.spent_usd, 0.002584);

    let forge = dto
        .gateways
        .iter()
        .find(|g| g.site == "forge")
        .expect("forge");
    assert_eq!(forge.state, GatewayState::Reporting);
    let unnamed = dto
        .gateways
        .iter()
        .find(|g| g.site == "unnamed")
        .expect("unnamed");
    assert_eq!(unnamed.state, GatewayState::Reporting);
    assert_eq!(unnamed.last_record, None, "no call yet, so no record time");
}

#[tokio::test]
async fn a_cloud_without_the_gateways_route_says_not_reported_never_none_silent() {
    let state = money_at(&spawn_cloud(vec![])).await;
    let dto = money_gateways(&state).await.expect("a 404 is an answer");
    assert!(!dto.reported);
    assert!(dto.gateways.is_empty());
}

#[tokio::test]
async fn a_cloud_that_refuses_the_gateways_read_is_an_error_not_an_empty_list() {
    let state = money_at(&spawn_cloud(vec![(
        "/v1/gateways",
        "500 Internal Server Error",
        "{}".to_string(),
    )]))
    .await;
    assert!(money_gateways(&state).await.is_err());
}
