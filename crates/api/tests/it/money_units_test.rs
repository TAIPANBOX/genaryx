//! The Money panel's unit month caps: every unit, its central cap, this
//! month's spend and where it stands, and the signed change of one cap.
//!
//! TokenFuse Cloud folds month-to-date spend per unit (`GET /v1/units`) and
//! holds the central caps every gateway polls and enforces (`GET
//! /v1/unit-budgets`, `POST /v1/units/{id}/budget`). The console read neither
//! for a view until 2026-10-14; only Felyx's `budgets` tool did.
//!
//! The figures are the forge lab's of 2026-10-14 (B8 unit cap, B12 cross-door
//! cap): unit `capped` with a USD 0.00005 cap, over it and refused with
//! `unit_budget_exceeded`; unit `w2x` with a 7168 uUSD central cap.

use crate::cloud_stub::{money_at, money_with_device_at, spawn_cloud, spawn_recording_cloud};
use genaryx_api::money::commands::MoneyError;
use genaryx_api::money::units::{
    UNASSIGNED, UnitCapDto, UnitCapState, UnitCapsDto, money_set_unit_budget, money_units,
};
use serde_json::{Value, json};
use std::collections::HashMap;

fn unit(name: &str, month_spent: i64, month_calls: u64) -> Value {
    json!({
        "unit": name, "spent_microusd": month_spent, "calls": month_calls, "runs": 1,
        "last_seen_millis": 1_791_985_800_000_i64, "tool_calls": 0, "month": "2026-10",
        "month_spent_microusd": month_spent, "month_calls": month_calls,
    })
}

fn forge_units() -> String {
    json!([
        unit("capped", 61, 2),
        unit("w2x", 13_376, 6),
        unit("lab", 2_400, 3),
        unit("console", 2_000, 1),
        unit(UNASSIGNED, 900, 4),
    ])
    .to_string()
}

const FORGE_CAPS: &str = r#"{"capped":50,"w2x":7168,"lab":6000,"finance":250000}"#;

fn row<'a>(dto: &'a UnitCapsDto, name: &str) -> &'a UnitCapDto {
    dto.units
        .iter()
        .find(|u| u.unit == name)
        .unwrap_or_else(|| panic!("{name} is missing from the unit caps"))
}

async fn caps_for(routes: Vec<(&'static str, &'static str, String)>) -> UnitCapsDto {
    let state = money_at(&spawn_cloud(routes)).await;
    money_units(&state).await.expect("unit caps")
}

#[tokio::test]
async fn the_units_section_joins_month_spend_with_each_units_cap() {
    let dto = caps_for(vec![
        ("/v1/units", "200 OK", forge_units()),
        ("/v1/unit-budgets", "200 OK", FORGE_CAPS.to_string()),
    ])
    .await;
    assert!(dto.units_read && dto.caps_read, "{:?}", dto.notes);
    assert_eq!(dto.month.as_deref(), Some("2026-10"));

    let capped = row(&dto, "capped");
    assert_eq!(capped.state, UnitCapState::OverCap, "61 spent on a 50 cap");
    assert_eq!(capped.cap_usd, Some(0.00005));
    assert_eq!(capped.month_spent_usd, Some(0.000061));
    assert_eq!(capped.month_calls, Some(2));

    assert_eq!(row(&dto, "w2x").state, UnitCapState::OverCap);
    assert_eq!(row(&dto, "lab").state, UnitCapState::UnderCap);
    assert_eq!(row(&dto, "lab").fraction, Some(0.4));

    // No central cap: a real answer from a map that was read.
    let console = row(&dto, "console");
    assert_eq!(console.state, UnitCapState::NoCap);
    assert_eq!(console.cap_usd, None);

    // A cap on a unit with no spend row this month: the spend is unknown,
    // never a zero the Cloud did not report.
    let finance = row(&dto, "finance");
    assert_eq!(finance.state, UnitCapState::SpendUnknown);
    assert_eq!(finance.month_spent_usd, None);
    assert_eq!(finance.cap_usd, Some(0.25));

    // The unmapped bucket is listed and offered no cap.
    assert!(!row(&dto, UNASSIGNED).cappable);
    assert!(row(&dto, "lab").cappable);

    // Over-cap units lead.
    assert_eq!(dto.units[0].state, UnitCapState::OverCap);
    assert_eq!(dto.units[1].state, UnitCapState::OverCap);
}

#[tokio::test]
async fn a_unit_near_its_cap_reads_near() {
    let dto = caps_for(vec![
        (
            "/v1/units",
            "200 OK",
            json!([unit("lab", 5_400, 3)]).to_string(),
        ),
        ("/v1/unit-budgets", "200 OK", r#"{"lab":6000}"#.to_string()),
    ])
    .await;
    assert_eq!(
        row(&dto, "lab").state,
        UnitCapState::NearCap,
        "0.9 of the cap"
    );
    assert_eq!(row(&dto, "lab").fraction, Some(0.9));
}

#[tokio::test]
async fn a_cloud_without_unit_budgets_shows_caps_unknown_never_no_cap() {
    for status in [
        "404 Not Found",
        "500 Internal Server Error",
        "403 Forbidden",
    ] {
        let mut routes = vec![("/v1/units", "200 OK", forge_units())];
        if status != "404 Not Found" {
            routes.push(("/v1/unit-budgets", status, r#"{"error":"x"}"#.to_string()));
        }
        let dto = caps_for(routes).await;
        assert!(!dto.caps_read, "{status}");
        assert!(dto.units_read, "{status}");
        assert!(
            dto.units
                .iter()
                .all(|u| u.state == UnitCapState::CapUnknown),
            "{status}: no unit may read 'no cap' when the map was not read"
        );
        assert_eq!(dto.notes.len(), 1, "{status}: {:?}", dto.notes);
        assert!(dto.notes[0].contains("unit-budgets"), "{status}");
    }
}

#[tokio::test]
async fn a_cloud_without_units_still_lists_its_caps_with_spend_unknown() {
    let dto = caps_for(vec![("/v1/unit-budgets", "200 OK", FORGE_CAPS.to_string())]).await;
    assert!(!dto.units_read && dto.caps_read);
    assert_eq!(dto.month, None);
    assert_eq!(dto.units.len(), 4);
    assert!(dto.units.iter().all(|u| u.month_spent_usd.is_none()));
    assert_eq!(row(&dto, "w2x").state, UnitCapState::SpendUnknown);
}

#[tokio::test]
async fn a_cloud_that_answers_neither_is_an_error_not_an_empty_section() {
    let base = spawn_cloud(vec![
        ("/v1/units", "500 Internal Server Error", "{}".to_string()),
        (
            "/v1/unit-budgets",
            "500 Internal Server Error",
            "{}".to_string(),
        ),
    ]);
    let state = money_at(&base).await;
    assert!(matches!(
        money_units(&state).await,
        Err(MoneyError::Cloud {
            status: Some(500),
            ..
        })
    ));
}

/// The cap map is bytes from outside the process: whatever parses as a unit
/// to micros map is shown exactly, whatever does not reads "cap unknown".
#[tokio::test]
async fn hostile_unit_cap_bodies_never_cost_the_section_or_invent_a_cap() {
    const ATOMS: &[&str] = &[
        "{", "}", "[", "]", ":", ",", "\"lab\"", "\"w2x\"", "6000", "-1", "0", "1e400", "null",
        "true", "\"x\"", " ",
    ];
    let mut state: u64 = 0x5851_F42D_4C95_7F2D;
    for seed in 0..200u32 {
        let mut body = String::new();
        for _ in 0..1 + (seed as usize % 12) {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            body.push_str(ATOMS[(state >> 33) as usize % ATOMS.len()]);
        }
        let parsed: Option<HashMap<String, i64>> = serde_json::from_str(&body).ok();
        let dto = caps_for(vec![
            (
                "/v1/units",
                "200 OK",
                json!([unit("lab", 5_400, 3)]).to_string(),
            ),
            ("/v1/unit-budgets", "200 OK", body.clone()),
        ])
        .await;
        assert_eq!(
            dto.caps_read,
            parsed.is_some(),
            "seed {seed}, body {body:?}"
        );
        let lab = row(&dto, "lab");
        match &parsed {
            Some(map) => assert_eq!(
                lab.cap_usd,
                map.get("lab").map(|m| *m as f64 / 1_000_000.0),
                "seed {seed}, body {body:?}"
            ),
            None => assert_eq!(lab.state, UnitCapState::CapUnknown, "seed {seed}"),
        }
    }
}

// ---- the signed cap change --------------------------------------------------

#[tokio::test]
async fn setting_a_unit_cap_posts_a_signed_budget_to_the_cloud() {
    let (base, seen) = spawn_recording_cloud(vec![(
        "/v1/units/w2x/budget",
        "200 OK",
        r#"{"unit":"w2x","budget_micros":7168}"#.to_string(),
    )]);
    let state = money_with_device_at(&base).await;
    let outcome = money_set_unit_budget(
        "w2x".to_string(),
        0.007168,
        "B12 lab cap".to_string(),
        &state,
    )
    .await
    .expect("the Cloud accepted the cap");
    assert_eq!(outcome.verify_result, "budget_micros:7168");
    assert!(outcome.summary.contains("w2x"), "{}", outcome.summary);
    // No live bus in this test: the journal attempt says so, never silently.
    assert!(!outcome.bus_recorded);
    assert!(outcome.bus_error.is_some());

    let seen = seen.lock().expect("log").clone();
    assert_eq!(seen.len(), 1, "{seen:?}");
    let req = &seen[0];
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/v1/units/w2x/budget");
    let body: Value = serde_json::from_str(&req.body).expect("json body");
    assert_eq!(body, json!({ "budget_usd": 0.007168 }));
    assert_eq!(req.header("x-fuse-device"), Some("dev-test"));
    assert!(req.header("x-fuse-sig").is_some_and(|s| !s.is_empty()));
    assert_eq!(req.header("authorization"), Some("Bearer device-token"));
}

#[tokio::test]
async fn a_unit_cap_change_that_cannot_be_a_cap_never_reaches_the_cloud() {
    let (base, seen) = spawn_recording_cloud(vec![]);
    let state = money_with_device_at(&base).await;
    for (unit, usd, reason) in [
        ("w2x", 1.0, ""),
        ("w2x", 1.0, "   "),
        ("", 1.0, "reason"),
        (UNASSIGNED, 1.0, "reason"),
        ("w2x", -1.0, "reason"),
        ("w2x", f64::NAN, "reason"),
    ] {
        let err = money_set_unit_budget(unit.to_string(), usd, reason.to_string(), &state)
            .await
            .expect_err("refused before the network");
        assert!(
            matches!(
                err,
                MoneyError::BreakGlassMissingReason | MoneyError::InvalidArgument { .. }
            ),
            "{unit:?} {usd} {reason:?}: {err:?}"
        );
    }
    assert!(
        seen.lock().expect("log").is_empty(),
        "no refused cap change may reach the Cloud"
    );
}

#[tokio::test]
async fn a_cloud_that_refuses_the_cap_change_is_reported_as_itself() {
    let (base, _seen) = spawn_recording_cloud(vec![(
        "/v1/units/w2x/budget",
        "403 Forbidden",
        r#"{"error":"admin role required"}"#.to_string(),
    )]);
    let state = money_with_device_at(&base).await;
    let err = money_set_unit_budget("w2x".to_string(), 1.0, "r".to_string(), &state)
        .await
        .expect_err("refused by the Cloud");
    assert!(
        matches!(
            err,
            MoneyError::Cloud {
                status: Some(403),
                ..
            }
        ),
        "{err:?}"
    );
}
