//! The Money panel's runs table carries how the gateway decided each run, as
//! the Cloud reports it: refused calls and the latest decision, calls charged
//! at the fallback rate, calls forwarded under identity warn, and the site
//! that pushed the run. A Cloud that does not report a field leaves it
//! unknown, never zero.
//!
//! TokenFuse Cloud has carried these on every `GET /v1/runs` row since v1.2.0
//! (`site`) and v1.8.0 (`blocked`, `last_decision`, `last_decision_millis`,
//! `fallback_calls`, `basis_unreported_calls`, `identity_warned`,
//! `last_identity_reason`; tokenfuse invariants 86 and 87). The console's
//! `RunAgg` mirrored the older shape and serde dropped every one of them, so
//! a run whose every call was refused read as "live", a call charged at the
//! 15/75 fallback rate read as an ordinary charge, and a call forwarded under
//! identity warn read as honest.
//!
//! The figures are the forge lab's of 2026-10-14 (B3 runaway, B1 benign on
//! the OpenAI door, B4 identity), carried by a stub Cloud.

use crate::cloud_stub::{money_at, spawn_cloud};
use genaryx_api::money::commands::{RunDto, money_runs};
use serde_json::{Value, json};

fn base_run(run_id: &str, agent: &str, spent: i64) -> Value {
    json!({
        "run_id": run_id, "model": "claude-haiku-5-5", "agent_id": agent,
        "spent_microusd": spent, "calls": 2, "cache_hits": 0, "steps": 2,
        "last_seen_millis": 1_791_985_800_000_i64, "killed": false,
    })
}

/// A v1.8 row: every decision field present.
fn v18_run(run_id: &str, extra: Value) -> Value {
    let mut row = base_run(run_id, "agent://lab.taipanbox.dev/w2/runaway", 190);
    let fields = json!({
        "site": "forge", "tool_calls": 0, "blocked": 0, "last_decision": "allow",
        "last_decision_millis": 1_791_985_800_000_i64, "fallback_calls": 0,
        "basis_unreported_calls": 0, "identity_warned": 0, "last_identity_reason": "",
    });
    for (k, v) in fields.as_object().expect("object") {
        row[k] = v.clone();
    }
    for (k, v) in extra.as_object().expect("object") {
        row[k] = v.clone();
    }
    row
}

fn forge_runs() -> String {
    json!([
        // B3 runaway: refused at turn 2 with budget_exceeded.
        v18_run(
            "w2-b3-runaway",
            json!({"blocked": 1, "last_decision": "budget_exceeded",
                   "last_decision_millis": 1_791_985_812_345_i64}),
        ),
        // B1 on the OpenAI door: openai/gpt-4.1-mini has no row in the book.
        v18_run(
            "w2-b1-openai",
            json!({"model": "openai/gpt-4.1-mini", "site": "forge-openai",
                   "fallback_calls": 2, "spent_microusd": 5300}),
        ),
        // Identity warn: forwarded although the check would have refused it.
        v18_run(
            "w2-b4-warn",
            json!({"identity_warned": 2,
                   "last_identity_reason": "agent_id_not_allowed"}),
        ),
        // An older gateway pushed this one: no price basis named.
        v18_run(
            "w2-old-gw",
            json!({"basis_unreported_calls": 3, "site": ""})
        ),
    ])
    .to_string()
}

fn row<'a>(rows: &'a [RunDto], run_id: &str) -> &'a RunDto {
    rows.iter()
        .find(|r| r.run_id == run_id)
        .unwrap_or_else(|| panic!("{run_id} is missing from the runs table"))
}

async fn rows_for(runs_body: String) -> Vec<RunDto> {
    let base = spawn_cloud(vec![
        ("/v1/runs", "200 OK", runs_body),
        ("/v1/budgets", "200 OK", "{}".to_string()),
    ]);
    let state = money_at(&base).await;
    money_runs(&state).await.expect("runs table")
}

#[tokio::test]
async fn a_refused_run_reaches_the_runs_table_with_its_reason_and_count() {
    let rows = rows_for(forge_runs()).await;
    let runaway = row(&rows, "w2-b3-runaway");
    assert_eq!(runaway.blocked, Some(1), "one call refused on this run");
    assert_eq!(runaway.last_decision.as_deref(), Some("budget_exceeded"));
    assert_eq!(
        runaway.last_decision_at.as_deref(),
        Some("2026-10-14T13:50:12.345Z"),
        "the moment the latest decision was taken, as ISO-8601 UTC"
    );

    // An admitted run says so, and its zero refusals are a count the Cloud
    // made, not an absence.
    let benign = row(&rows, "w2-b4-warn");
    assert_eq!(benign.blocked, Some(0));
    assert_eq!(benign.last_decision.as_deref(), Some("allow"));
}

#[tokio::test]
async fn fallback_pricing_identity_warnings_and_the_site_reach_the_runs_table() {
    let rows = rows_for(forge_runs()).await;

    let openai = row(&rows, "w2-b1-openai");
    assert_eq!(openai.fallback_calls, Some(2));
    assert_eq!(openai.site.as_deref(), Some("forge-openai"));

    let warned = row(&rows, "w2-b4-warn");
    assert_eq!(warned.identity_warned, Some(2));
    assert_eq!(
        warned.last_identity_reason.as_deref(),
        Some("agent_id_not_allowed")
    );
    assert_eq!(warned.fallback_calls, Some(0));

    let old_gw = row(&rows, "w2-old-gw");
    assert_eq!(old_gw.basis_unreported_calls, Some(3));
    // Reported, and empty: no site-bound key pushed this run. Not the same
    // statement as a Cloud that does not report sites at all.
    assert_eq!(old_gw.site.as_deref(), Some(""));
}

#[tokio::test]
async fn a_cloud_older_than_the_decision_fields_leaves_them_unknown_never_zero() {
    let r = "agent://taipanbox.dev/routers";
    let body = json!([
        base_run("p1-beryl2", &format!("{r}/beryl2"), 4019),
        base_run("mig-flint", &format!("{r}/flint"), 3210),
    ])
    .to_string();
    let rows = rows_for(body).await;
    assert_eq!(rows.len(), 2);
    for run in &rows {
        assert_eq!(run.blocked, None, "{}: refusals not reported", run.run_id);
        assert_eq!(run.last_decision, None, "{}", run.run_id);
        assert_eq!(run.last_decision_at, None, "{}", run.run_id);
        assert_eq!(run.fallback_calls, None, "{}", run.run_id);
        assert_eq!(run.basis_unreported_calls, None, "{}", run.run_id);
        assert_eq!(run.identity_warned, None, "{}", run.run_id);
        assert_eq!(run.last_identity_reason, None, "{}", run.run_id);
        assert_eq!(run.site, None, "{}: sites not reported", run.run_id);
    }
}

/// The run list is bytes from outside the process. A decision field of the
/// wrong type must cost neither the table nor the row, and must not become a
/// value the body did not hold.
#[tokio::test]
async fn hostile_decision_fields_never_cost_the_table_or_invent_a_value() {
    const VALUES: &[&str] = &[
        "3",
        "0",
        "-1",
        "1.5",
        "18446744073709551616",
        "\"3\"",
        "\"budget_exceeded\"",
        "\"\"",
        "null",
        "true",
        "[]",
        "{}",
        "[1,2]",
        "{\"x\":1}",
    ];
    const FIELDS: &[&str] = &[
        "blocked",
        "fallback_calls",
        "basis_unreported_calls",
        "identity_warned",
        "last_decision",
        "last_identity_reason",
        "site",
        "last_decision_millis",
    ];
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    for seed in 0..200u32 {
        let mut row_json = base_run("hostile", "agent://lab/x", 10);
        let mut planted: Vec<(&str, Value)> = Vec::new();
        for field in FIELDS {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            if (state >> 60).is_multiple_of(3) {
                continue; // absent
            }
            let raw = VALUES[(state >> 33) as usize % VALUES.len()];
            let v: Value = serde_json::from_str(raw).expect("valid JSON atom");
            row_json[*field] = v.clone();
            planted.push((*field, v));
        }
        let body = json!([row_json, base_run("calm", "agent://lab/y", 5)]).to_string();
        let rows = rows_for(body.clone()).await;
        assert_eq!(
            rows.len(),
            2,
            "seed {seed}: the table survives, body {body}"
        );
        let got = row(&rows, "hostile");
        for (field, v) in &planted {
            let want_count = v.as_u64();
            let want_text = v.as_str().map(str::to_string);
            match *field {
                "blocked" => assert_eq!(got.blocked, want_count, "seed {seed} {field}={v}"),
                "fallback_calls" => {
                    assert_eq!(got.fallback_calls, want_count, "seed {seed} {field}={v}")
                }
                "basis_unreported_calls" => assert_eq!(
                    got.basis_unreported_calls, want_count,
                    "seed {seed} {field}={v}"
                ),
                "identity_warned" => {
                    assert_eq!(got.identity_warned, want_count, "seed {seed} {field}={v}")
                }
                "last_decision" => {
                    assert_eq!(got.last_decision, want_text, "seed {seed} {field}={v}")
                }
                "last_identity_reason" => assert_eq!(
                    got.last_identity_reason, want_text,
                    "seed {seed} {field}={v}"
                ),
                "site" => assert_eq!(got.site, want_text, "seed {seed} {field}={v}"),
                // A timestamp that is not a positive integer names no moment.
                "last_decision_millis" => {
                    if v.as_i64().is_none_or(|m| m <= 0) {
                        assert_eq!(got.last_decision_at, None, "seed {seed} {field}={v}");
                    }
                }
                other => panic!("unplanned field {other}"),
            }
        }
        // The row next to it is untouched by its neighbour's garbage.
        assert_eq!(row(&rows, "calm").blocked, None, "seed {seed}");
    }
}

/// The forge lab's B11 run of 2026-10-14 went from Opus to Haiku; the Cloud
/// keeps `model: claude-haiku-5-5`, the last call's. When the Cloud also
/// reports `models`, the table carries every one with its spend; when it does
/// not, the models are unknown, never "one model".
#[tokio::test]
async fn the_models_a_run_used_reach_the_runs_table_only_when_the_cloud_reports_them() {
    let switched = v18_run(
        "w2-forge-switch-a3-133712",
        json!({"model": "claude-haiku-5-5", "spent_microusd": 566, "model_switches": 1,
               "models": [{"model": "claude-opus-5-5", "calls": 1, "spent_microusd": 408},
                          {"model": "claude-haiku-5-5", "calls": 1, "spent_microusd": 158}]}),
    );
    let today = v18_run("w2-forge-switch-a2", json!({"model": "claude-opus-5-5"}));
    let rows = rows_for(json!([switched, today]).to_string()).await;

    let s = row(&rows, "w2-forge-switch-a3-133712");
    assert_eq!(
        s.model, "claude-haiku-5-5",
        "the last call's model, unchanged"
    );
    let models = s.models.as_ref().expect("reported");
    assert_eq!(
        models
            .iter()
            .map(|m| (m.model.as_str(), m.calls, m.spent_usd))
            .collect::<Vec<_>>(),
        [
            ("claude-opus-5-5", 1, 0.000408),
            ("claude-haiku-5-5", 1, 0.000158)
        ]
    );
    assert_eq!(s.model_switches, Some(1));

    let t = row(&rows, "w2-forge-switch-a2");
    assert!(t.models.is_none(), "not reported is not one model");
    assert_eq!(t.model_switches, None);
}

/// PII found on a run (tokenfuse#433, the forge lab's B6 shadow run of
/// 2026-10-14: card, email and phone found and forwarded), carried as a
/// count when the Cloud reports it and unknown when it does not.
#[tokio::test]
async fn the_calls_a_run_carried_personal_data_in_reach_the_runs_table_when_reported() {
    let found = v18_run("w2-b6-pii", json!({"pii_calls": 3}));
    let older = v18_run("w2-b1", json!({}));
    let rows = rows_for(json!([found, older]).to_string()).await;
    assert_eq!(row(&rows, "w2-b6-pii").pii_calls, Some(3));
    assert_eq!(
        row(&rows, "w2-b1").pii_calls,
        None,
        "not reported is not zero"
    );
}
