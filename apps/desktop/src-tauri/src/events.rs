//! UI-facing event shape for the Bus Explorer, plus a mock data source.
//!
//! `UiEvent` is a serde mirror of `genaryx_core::store::StoredEvent`, shaped
//! for the frontend: every field the Bus Explorer renders (row + expand
//! panel) round-trips through here. Today [`mock_events`] fabricates the
//! list; see the `FOLLOW-UP WIRING POINT` doc comment below the `From` impl
//! for exactly where that gets replaced by real data.

use chrono::{Duration, SecondsFormat, Utc};
use genaryx_core::event::SchemaVersion;
use genaryx_core::store::StoredEvent;
use serde::Serialize;
use serde_json::{Map, Value, json};

/// UI-facing mirror of [`StoredEvent`]. Field-for-field: every byte the
/// console ever stores about an event is a byte the Bus Explorer can show
/// (06 §0.8 provenance), so nothing is dropped between core and shell here.
///
/// `type_` is renamed to `"type"` on the wire since the frontend has no
/// reason to inherit Rust's keyword-escaping quirk.
#[derive(Debug, Clone, Serialize)]
pub struct UiEvent {
    pub id: i64,
    pub env: String,
    pub ts: String,
    pub source: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub agent_id: String,
    pub run_id: Option<String>,
    pub severity: Option<String>,
    pub schema: String,
    pub on_behalf_of: Vec<String>,
    pub data: Option<Value>,
    pub prev_hash: Option<String>,
    pub raw: String,
    pub file: Option<String>,
    pub off: Option<u64>,
}

// ============================================================================
// FOLLOW-UP WIRING POINT
// ============================================================================
// This `From` impl is where the real bus gets connected. Today `recent_events`
// (in `lib.rs`) calls [`mock_events`] directly. The follow-up task:
//   1. holds a `genaryx_core::ingest::IngestService` (or a handle to its
//      `Store`) in Tauri's managed state instead of nothing,
//   2. replaces the `mock_events(limit)` call in the `recent_events` command
//      with `store.recent_events(limit)?.into_iter().map(UiEvent::from).collect()`,
//   3. subscribes once at startup via `IngestService::subscribe()` and forwards
//      each `ConsoleEvent` to the frontend as a Tauri event (e.g. `app.emit`),
//      converting it the same way after wrapping it into a `StoredEvent`-shaped
//      value (or by extending this `From` site with a `ConsoleEvent` variant).
// Everything downstream (`UiEvent`, the whole `src/` frontend) already
// consumes this exact shape, so that swap is the entire follow-up task.
impl From<StoredEvent> for UiEvent {
    fn from(e: StoredEvent) -> Self {
        Self {
            id: e.id,
            env: e.env,
            ts: e.ts,
            source: e.source,
            event_type: e.type_,
            agent_id: e.agent_id,
            run_id: e.run_id,
            severity: e.severity,
            schema: e.schema,
            on_behalf_of: e.on_behalf_of,
            data: e.data,
            prev_hash: e.prev_hash,
            raw: e.raw,
            file: e.file,
            off: e.off,
        }
    }
}

/// Topics cycled through by mock engram memory events (mirrors the shape of
/// `genaryx_core::demo`'s `TOPICS`, without depending on that module).
const TOPICS: [&str; 6] = [
    "customer_refund_policy",
    "kyc_verification_steps",
    "fraud_hold_criteria",
    "sla_response_times",
    "chargeback_procedure",
    "aml_screening_rules",
];

/// Eval suites cycled through by mock verdryx quality events.
const EVAL_SUITES: [&str; 5] = [
    "refund-policy-qa",
    "kyc-accuracy-qa",
    "fraud-triage-qa",
    "sla-compliance-qa",
    "aml-screening-qa",
];

/// Scenarios cycled through by mock mockryx fire-drill events.
const SCENARIOS: [&str; 5] = [
    "prod-deploy-rehearsal",
    "budget-exhaustion-drill",
    "policy-bypass-drill",
    "credential-leak-drill",
    "runaway-agent-drill",
];

/// One row of the hardcoded mock timeline: enough to derive a full
/// `UiEvent`, deliberately mirroring the real event/type/severity/schema
/// combinations `genaryx_core::demo` uses, so the Bus Explorer looks the way
/// a real validation run looks (never invented domain shapes).
struct Seed {
    source: &'static str,
    /// `true` -> schema v0.2, `false` -> schema v0.1 (matches the real
    /// per-source split in `genaryx_core::demo`).
    v2: bool,
    event_type: &'static str,
    severity: &'static str,
    agent: &'static str,
    run: u32,
    /// Whether this row carries the demo delegation chain
    /// (`user://.../j.doe` -> `agent://.../demo/orchestrator`).
    delegated: bool,
}

impl Seed {
    const fn new(
        source: &'static str,
        v2: bool,
        event_type: &'static str,
        severity: &'static str,
        agent: &'static str,
        run: u32,
        delegated: bool,
    ) -> Self {
        Self {
            source,
            v2,
            event_type,
            severity,
            agent,
            run,
            delegated,
        }
    }
}

/// The ~40-event mock timeline (oldest first; [`mock_events`] reverses it to
/// newest-first before returning, matching `Store::recent_events`). Grouped
/// in small bursts by `run`, the same way a real agent run emits a handful of
/// correlated events across a few planes rather than one event at a time.
fn seeds() -> Vec<Seed> {
    vec![
        Seed::new("wardryx", true, "policy_allow", "info", "tier1-bot", 1, false),
        Seed::new("tokenfuse", false, "budget_exhausted", "critical", "tier1-bot", 1, false),
        Seed::new("engram", false, "memory_written", "info", "tier1-bot", 1, false),
        Seed::new("wardryx", true, "policy_allow", "info", "tier2-bot", 2, false),
        Seed::new("tokenfuse", false, "breaker_tripped", "critical", "tier2-bot", 2, false),
        Seed::new("engram", false, "memory_written", "info", "tier2-bot", 2, false),
        Seed::new("wardryx", true, "policy_deny", "high", "ci-fixer", 3, true),
        Seed::new("wardryx", true, "approval_requested", "medium", "ci-fixer", 3, true),
        Seed::new("wardryx", true, "approval_granted", "info", "ci-fixer", 3, true),
        Seed::new("tokenfuse", false, "spend_spike", "high", "fraud-bot", 4, false),
        Seed::new("verdryx", true, "quality_score", "info", "fraud-bot", 4, false),
        Seed::new("engram", false, "memory_written", "info", "kyc-bot", 5, false),
        Seed::new("engram", false, "contradiction_found", "medium", "kyc-bot", 5, false),
        Seed::new("verdryx", true, "quality_score", "info", "refund-bot", 6, false),
        Seed::new("verdryx", true, "quality_drift", "high", "refund-bot", 6, false),
        Seed::new("mockryx", true, "sim_run", "info", "audit-bot", 7, false),
        Seed::new("mockryx", true, "sim_finding", "medium", "audit-bot", 7, false),
        Seed::new("mockryx", true, "blast_radius_measured", "medium", "audit-bot", 7, false),
        Seed::new("qryx", false, "crypto_finding", "medium", "verifier", 8, false),
        Seed::new("qryx", false, "evidence_signed", "info", "verifier", 8, false),
        Seed::new("tokenfuse", false, "sustained_loop", "high", "router", 9, false),
        Seed::new("wardryx", true, "policy_allow", "info", "router", 9, false),
        Seed::new("tokenfuse", false, "fanout_explosion", "high", "orchestrator", 10, false),
        Seed::new("engram", false, "memory_written", "info", "orchestrator", 10, false),
        Seed::new("wardryx", true, "policy_deny", "high", "deploy-bot", 11, true),
        Seed::new("wardryx", true, "approval_requested", "medium", "deploy-bot", 11, true),
        Seed::new("wardryx", true, "approval_granted", "info", "deploy-bot", 11, true),
        Seed::new("verdryx", true, "quality_score", "info", "collections-bot", 12, false),
        Seed::new("mockryx", true, "sim_run", "info", "sentinel", 13, false),
        Seed::new("mockryx", true, "sim_finding", "medium", "sentinel", 13, false),
        Seed::new("qryx", false, "crypto_finding", "medium", "auditor", 14, false),
        Seed::new("qryx", false, "evidence_signed", "info", "auditor", 14, false),
        Seed::new("tokenfuse", false, "budget_exhausted", "critical", "billing-bot", 15, false),
        Seed::new("engram", false, "memory_written", "info", "billing-bot", 15, false),
        Seed::new("wardryx", true, "policy_allow", "info", "scheduler", 16, false),
        Seed::new("verdryx", true, "quality_drift", "high", "onboarding-bot", 17, false),
        Seed::new("mockryx", true, "blast_radius_measured", "medium", "reconciler", 18, false),
        Seed::new("qryx", false, "crypto_finding", "medium", "translator", 19, false),
        Seed::new("wardryx", true, "policy_deny", "high", "support-bot", 20, true),
        Seed::new("tokenfuse", false, "breaker_tripped", "critical", "support-bot", 20, true),
    ]
}

/// The canonical `data` payload for one mock event type, matching the shapes
/// `genaryx_core::demo` emits for the same `event_type` (08 §5 conventions),
/// with `run`/`agent_id` folded in where a real payload would vary per-run.
fn seed_data(event_type: &str, run: u32, agent_id: &str) -> Value {
    let topic = TOPICS[(run as usize) % TOPICS.len()];
    let eval_suite = EVAL_SUITES[(run as usize) % EVAL_SUITES.len()];
    let scenario = SCENARIOS[(run as usize) % SCENARIOS.len()];

    match event_type {
        "budget_exhausted" => json!({
            "budget_usd": 0.0012,
            "spent_usd": 0.0028,
            "reason": "budget_exceeded",
            "policy_id": "default",
        }),
        "breaker_tripped" => json!({
            "budget_usd": 0.0009,
            "spent_usd": 0.0021,
            "reason": "budget_exceeded",
            "policy_id": "default",
        }),
        "spend_spike" => json!({
            "window_s": 60,
            "spend_usd": 7.42,
            "baseline_usd": 1.15,
            "multiplier": 6.4,
        }),
        "sustained_loop" => json!({
            "calls": 88,
            "window_s": 120,
            "pattern": "repeated_tool_call",
        }),
        "fanout_explosion" => json!({
            "child_agents": 7,
            "depth": 3,
            "budget_usd": 2.85,
        }),
        "policy_allow" => json!({
            "policy": "default-allow",
            "reason": "within policy",
        }),
        "policy_deny" => json!({
            "policy": "prod-deploy-requires-approval",
            "reason": "no approval on file for deploy:prod scope",
        }),
        "approval_requested" => json!({
            "policy": "prod-deploy-requires-approval",
            "reason": "awaiting operator approval",
        }),
        "approval_granted" => json!({
            "policy": "prod-deploy-requires-approval",
            "granted_by": "user://taipanbox.dev/j.doe",
        }),
        "memory_written" => json!({
            "memory_id": format!("mem-{:04}", 3000 + run),
            "topic": topic,
        }),
        "contradiction_found" => json!({
            "memory_id": format!("mem-{:04}", 3000 + run),
            "conflicting_memory_id": format!("mem-{:04}", 2000 + run),
            "topic": topic,
        }),
        "quality_score" => json!({
            "eval_suite": eval_suite,
            "current_score": 0.93,
        }),
        "quality_drift" => json!({
            "eval_suite": eval_suite,
            "baseline_score": 0.97,
            "current_score": 0.89,
            "delta": -0.08,
        }),
        "sim_run" => json!({
            "scenario": scenario,
            "status": "completed",
        }),
        "sim_finding" => json!({
            "scenario": scenario,
            "finding": "gap_found",
        }),
        "blast_radius_measured" => json!({
            "scenario": scenario,
            "blast_radius_score": 0.52,
            "affected_resources": 14,
        }),
        "crypto_finding" => json!({
            "algorithm": "rsa-2048",
            "risk": "quantum-vulnerable",
            "recommended": "ml-dsa-65",
        }),
        "evidence_signed" => json!({
            "evidence_id": format!("ev-{:05}", 55_000 + run),
            "algorithm": "ml-dsa-65",
            "subject": agent_id,
        }),
        _ => json!({}),
    }
}

/// Re-render one mock row as the NDJSON line it would have been ingested
/// from, so "click to expand raw JSON" has an authentic-looking `raw` to
/// show, not just the structured `data`. Shape matches
/// `genaryx_core::demo::render_line` (schema/ts/source/type/agent_id/
/// severity/run_id/on_behalf_of/data, in that order).
fn raw_line(
    schema: &str,
    ts: &str,
    source: &str,
    event_type: &str,
    agent_id: &str,
    severity: &str,
    run_id: &str,
    on_behalf_of: &[String],
    data: &Value,
) -> String {
    let mut obj = Map::new();
    obj.insert("schema".to_string(), Value::String(schema.to_string()));
    obj.insert("ts".to_string(), Value::String(ts.to_string()));
    obj.insert("source".to_string(), Value::String(source.to_string()));
    obj.insert("type".to_string(), Value::String(event_type.to_string()));
    obj.insert("agent_id".to_string(), Value::String(agent_id.to_string()));
    obj.insert("severity".to_string(), Value::String(severity.to_string()));
    obj.insert("run_id".to_string(), Value::String(run_id.to_string()));
    if !on_behalf_of.is_empty() {
        obj.insert(
            "on_behalf_of".to_string(),
            Value::Array(on_behalf_of.iter().cloned().map(Value::String).collect()),
        );
    }
    obj.insert("data".to_string(), data.clone());
    // A `Value::Object` built entirely from strings/clones of `data` (itself
    // already a valid `Value`) always serializes; `unwrap_or_default` just
    // keeps this path panic-free without claiming a real failure mode.
    serde_json::to_string(&Value::Object(obj)).unwrap_or_default()
}

/// Mock data source for the Bus Explorer (see the `FOLLOW-UP WIRING POINT`
/// doc comment above): ~40 events spanning all six emitting planes, newest
/// first by `id`, with realistic types/severities/agent ids and timestamps
/// anchored to "now" so the console never looks frozen in the past.
pub fn mock_events(limit: usize) -> Vec<UiEvent> {
    let now = Utc::now();
    let seeds = seeds();
    let n = seeds.len();

    let mut events: Vec<UiEvent> = seeds
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            let id = (i + 1) as i64;
            // Oldest row furthest in the past, newest row closest to `now`;
            // ~45s cadence reads as a live, busy-but-not-overwhelming console.
            let ts_dt = now - Duration::seconds(((n - i) * 45) as i64);
            let ts = ts_dt.to_rfc3339_opts(SecondsFormat::Millis, true);

            let schema = if s.v2 {
                SchemaVersion::SCHEMA_V0_2
            } else {
                SchemaVersion::SCHEMA_V0_1
            };
            let agent_id = format!("agent://taipanbox.dev/demo/{}", s.agent);
            let run_id = format!("demo-run-{:03}", s.run);
            let data = seed_data(s.event_type, s.run, &agent_id);
            let on_behalf_of = if s.delegated {
                vec![
                    "user://taipanbox.dev/j.doe".to_string(),
                    "agent://taipanbox.dev/demo/orchestrator".to_string(),
                ]
            } else {
                Vec::new()
            };
            let raw = raw_line(
                schema,
                &ts,
                s.source,
                s.event_type,
                &agent_id,
                s.severity,
                &run_id,
                &on_behalf_of,
                &data,
            );

            UiEvent {
                id,
                env: "local".to_string(),
                ts,
                source: s.source.to_string(),
                event_type: s.event_type.to_string(),
                agent_id,
                run_id: Some(run_id),
                severity: Some(s.severity.to_string()),
                schema: schema.to_string(),
                on_behalf_of,
                data: Some(data),
                // Mock rows are never chained; a real prev_hash implies a
                // real predecessor, which fabricated data does not have.
                prev_hash: None,
                raw,
                file: Some(format!("~/.taipan/events/{}.ndjson", s.source)),
                off: Some((i as u64) * 128),
            }
        })
        .collect();

    events.reverse(); // newest first by id, matching Store::recent_events
    events.truncate(limit);
    events
}
