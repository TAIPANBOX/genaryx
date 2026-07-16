//! `taipan demo` data generator: a deterministic event stream mirroring the
//! form of real validation campaigns (08 §2 "Demo mode"), so the product
//! never shows an empty screen on first run.
//!
//! Emits one NDJSON file per emitting service into `events_dir`, matching the
//! `taipan up` layout (`~/.taipan/events/<service>.ndjson`, 07 §3/§7). Idryx is
//! deliberately absent: it never emits to the bus, its findings come from its
//! API instead (07 §2), so writing an `idryx.ndjson` here would misrepresent
//! the real contract.
//!
//! Volume mirrors real campaigns (08 §2): a ~65-run, ~34-agent burst with
//! ~12 tokenfuse budget/breaker blocks and 170+ total events. Every line is
//! built through `serde_json` (never string concatenation), and every line
//! this module writes is asserted, in `tests/demo_test.rs`, to pass
//! [`crate::conform::Conformer`]: the loop from "demo data" to "conforming
//! agent-event" is closed by that test, not just by inspection.
//!
//! Determinism: no wall-clock reads and no `rand`. Timestamps come from a
//! fixed base instant plus an increasing `chrono::Duration`; all numeric
//! texture (amounts, scores, counts) comes from a tiny inline LCG seeded with
//! a fixed constant. Same input, same output: two calls to [`generate`]
//! always produce byte-identical files.

use crate::error::{Error, Result};
use crate::event::SchemaVersion;
use chrono::{DateTime, Duration, SecondsFormat, TimeZone, Utc};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Emitting services, in the order their files are written. Idryx is
/// deliberately absent (see module docs).
const SOURCES: [&str; 6] = [
    "tokenfuse",
    "wardryx",
    "engram",
    "verdryx",
    "mockryx",
    "qryx",
];

/// Distinct demo agents (34, matching the "34-agent burst" target), each
/// addressed as `agent://taipanbox.dev/demo/<name>`. Names read as plausible
/// bank-in-a-box roles, echoing `tests/fixtures/campaign-bank.ndjson`.
const AGENTS: [&str; 34] = [
    "tier1-bot",
    "tier2-bot",
    "orchestrator",
    "refund-bot",
    "kyc-bot",
    "fraud-bot",
    "onboarding-bot",
    "collections-bot",
    "support-bot",
    "billing-bot",
    "ci-fixer",
    "ci-orchestrator",
    "deploy-bot",
    "audit-bot",
    "reconciler",
    "scheduler",
    "router",
    "cache-warmer",
    "summarizer",
    "classifier",
    "triage-bot",
    "escalation-bot",
    "notifier",
    "watcher",
    "sentinel",
    "auditor",
    "verifier",
    "planner",
    "executor",
    "retriever",
    "indexer",
    "translator",
    "responder",
    "analyzer",
];

/// Number of synthesized runs (matches the "~65 runs" target).
const RUN_COUNT: usize = 65;

/// The first N runs are the tokenfuse budget/breaker incident storyline: one
/// block event per incident run, matching the "~12 blocks" target exactly.
const BLOCK_RUN_COUNT: usize = 12;

/// Delegation chain applied to a fraction of runs: root user, then the demo
/// orchestrator agent (root-first, per the `on_behalf_of` schema field).
const DELEGATION_CHAIN: [&str; 2] = [
    "user://taipanbox.dev/j.doe",
    "agent://taipanbox.dev/demo/orchestrator",
];

/// Topics cycled through by engram memory events.
const TOPICS: [&str; 6] = [
    "customer_refund_policy",
    "kyc_verification_steps",
    "fraud_hold_criteria",
    "sla_response_times",
    "chargeback_procedure",
    "aml_screening_rules",
];

/// Eval suites cycled through by verdryx quality events.
const EVAL_SUITES: [&str; 5] = [
    "refund-policy-qa",
    "kyc-accuracy-qa",
    "fraud-triage-qa",
    "sla-compliance-qa",
    "aml-screening-qa",
];

/// Scenarios cycled through by mockryx fire-drill events.
const SCENARIOS: [&str; 5] = [
    "prod-deploy-rehearsal",
    "budget-exhaustion-drill",
    "policy-bypass-drill",
    "credential-leak-drill",
    "runaway-agent-drill",
];

/// Fixed LCG seed. Any constant works: determinism only requires that it
/// never change and never come from the clock.
const LCG_SEED: u64 = 20_260_709;

/// A tiny inline linear congruential generator (Knuth's MMIX multiplier),
/// used only for cosmetic numeric texture (amounts, scores, counts). No
/// `rand` dependency: the same seed always yields the same sequence, so
/// output stays byte-identical across calls.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    /// Next value in `0..bound`. Returns 0 when `bound` is 0.
    fn next_range(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            0
        } else {
            self.next_u64() % bound
        }
    }
}

/// Round `value` to `decimals` places. Cosmetic only (JSON numbers do not
/// require it), but it keeps synthesized amounts readable in the UI.
fn round_to(value: f64, decimals: i32) -> f64 {
    let factor = 10f64.powi(decimals);
    (value * factor).round() / factor
}

/// A deterministic pseudo-random `f64` in `[lo, hi)`, rounded to `decimals`.
fn jitter(lcg: &mut Lcg, lo: f64, hi: f64, decimals: i32) -> f64 {
    let frac = lcg.next_range(10_000) as f64 / 10_000.0;
    round_to(lo + (hi - lo) * frac, decimals)
}

/// One synthesized event, prior to attaching its run-level context (agent,
/// run id, timestamp, delegation chain).
struct Call {
    source: &'static str,
    schema: SchemaVersion,
    event_type: &'static str,
    severity: &'static str,
    data: Value,
}

impl Call {
    fn new(
        source: &'static str,
        schema: SchemaVersion,
        event_type: &'static str,
        severity: &'static str,
        data: Value,
    ) -> Self {
        Self {
            source,
            schema,
            event_type,
            severity,
            data,
        }
    }
}

/// The fixed base instant every demo timestamp is offset from. Never
/// `Utc::now()`, so output is reproducible byte-for-byte.
fn base_instant() -> Result<DateTime<Utc>> {
    Utc.with_ymd_and_hms(2026, 7, 9, 3, 0, 0)
        .single()
        .ok_or_else(|| Error::Other("invalid base demo timestamp".to_string()))
}

/// Generate a demo event stream into `events_dir` (one file per service).
/// Returns the number of events written.
pub fn generate(events_dir: &Path) -> Result<usize> {
    fs::create_dir_all(events_dir)?;

    let mut buffers: BTreeMap<&'static str, String> =
        SOURCES.iter().map(|s| (*s, String::new())).collect();

    let mut lcg = Lcg::new(LCG_SEED);
    let base = base_instant()?;
    let mut cursor_ms: i64 = 0;
    let mut total = 0usize;

    for i in 0..RUN_COUNT {
        let agent = AGENTS[i % AGENTS.len()];
        let agent_id = format!("agent://taipanbox.dev/demo/{agent}");
        let run_id = format!("demo-run-{i:03}");

        // A fraction of runs carry a delegation chain; skip it for the
        // orchestrator's own runs so it never appears to delegate to itself.
        let on_behalf_of: &[&str] = if i.is_multiple_of(4) && agent != "orchestrator" {
            &DELEGATION_CHAIN
        } else {
            &[]
        };

        for call in run_calls(i, &agent_id, &mut lcg) {
            let ts = base + Duration::milliseconds(cursor_ms);
            cursor_ms += 250 + lcg.next_range(1_500) as i64;

            let line = render_line(&call, &ts, &agent_id, &run_id, on_behalf_of)?;
            let buf = buffers
                .get_mut(call.source)
                .ok_or_else(|| Error::Other(format!("unknown demo source: {}", call.source)))?;
            buf.push_str(&line);
            buf.push('\n');
            total += 1;
        }
    }

    for source in SOURCES {
        let path = events_dir.join(format!("{source}.ndjson"));
        let content = buffers.remove(source).unwrap_or_default();
        fs::write(&path, content)?;
    }

    Ok(total)
}

/// Build the events for run index `i` (0-based), given the LCG for cosmetic
/// texture. The first [`BLOCK_RUN_COUNT`] runs are the tokenfuse incident
/// storyline; the rest cycle through six domain buckets (one per emitting
/// plane) plus a small cross-plane filler, matching the density of real
/// campaigns without inflating the block count.
fn run_calls(i: usize, agent_id: &str, lcg: &mut Lcg) -> Vec<Call> {
    if i < BLOCK_RUN_COUNT {
        return block_run_calls(i, lcg);
    }

    let sub = i - BLOCK_RUN_COUNT;
    let bucket = sub % 6;
    let rank = sub / 6;

    let mut calls = match bucket {
        0 => money_alert_calls(rank, lcg),
        1 => policy_calls(rank),
        2 => memory_calls(rank, i),
        3 => quality_calls(rank, i, lcg),
        4 => drill_calls(rank, i, lcg),
        _ => crypto_calls(rank, i, agent_id),
    };
    calls.push(filler_call(i, bucket));
    calls
}

/// The incident storyline: a routine policy allow, the tokenfuse block
/// itself (alternating type so both are represented), and an engram note
/// recording the incident, mirroring `tests/fixtures/campaign-bank.ndjson`.
fn block_run_calls(i: usize, lcg: &mut Lcg) -> Vec<Call> {
    let event_type = if i.is_multiple_of(2) {
        "budget_exhausted"
    } else {
        "breaker_tripped"
    };
    let budget_usd = jitter(lcg, 0.0005, 0.0020, 4);
    let spent_usd = round_to(budget_usd + jitter(lcg, 0.0008, 0.0018, 4), 4);

    vec![
        Call::new(
            "wardryx",
            SchemaVersion::V0_2,
            "policy_allow",
            "info",
            json!({"policy": "default-allow", "reason": "within policy"}),
        ),
        Call::new(
            "tokenfuse",
            SchemaVersion::V0_1,
            event_type,
            "critical",
            json!({
                "budget_usd": budget_usd,
                "spent_usd": spent_usd,
                "reason": "budget_exceeded",
                "policy_id": "default",
            }),
        ),
        Call::new(
            "engram",
            SchemaVersion::V0_1,
            "memory_written",
            "info",
            json!({
                "memory_id": format!("mem-{:04}", 8000 + i),
                "topic": "incident_runbook_step",
            }),
        ),
    ]
}

/// tokenfuse alert types short of a hard block (severity "high" per 08 §5).
fn money_alert_calls(rank: usize, lcg: &mut Lcg) -> Vec<Call> {
    const TYPES: [&str; 3] = ["spend_spike", "sustained_loop", "fanout_explosion"];
    let mut calls = vec![money_alert_call(TYPES[rank % 3], lcg)];
    if rank.is_multiple_of(2) {
        calls.push(money_alert_call(TYPES[(rank + 1) % 3], lcg));
    }
    calls
}

fn money_alert_call(event_type: &'static str, lcg: &mut Lcg) -> Call {
    let data = match event_type {
        "spend_spike" => json!({
            "window_s": 60,
            "spend_usd": jitter(lcg, 4.0, 12.0, 2),
            "baseline_usd": jitter(lcg, 0.8, 2.0, 2),
            "multiplier": jitter(lcg, 3.0, 8.0, 1),
        }),
        "sustained_loop" => json!({
            "calls": 40 + lcg.next_range(120),
            "window_s": 120,
            "pattern": "repeated_tool_call",
        }),
        _ => json!({
            "child_agents": 3 + lcg.next_range(10),
            "depth": 2 + lcg.next_range(3),
            "budget_usd": jitter(lcg, 1.0, 5.0, 2),
        }),
    };
    Call::new("tokenfuse", SchemaVersion::V0_1, event_type, "high", data)
}

/// Routine allow most of the time; a deny-then-approval flow otherwise
/// (matching the Approvals Inbox concept in 08 §2).
fn policy_calls(rank: usize) -> Vec<Call> {
    if rank.is_multiple_of(2) {
        vec![Call::new(
            "wardryx",
            SchemaVersion::V0_2,
            "policy_allow",
            "info",
            json!({"policy": "default-allow", "reason": "within policy"}),
        )]
    } else {
        vec![
            Call::new(
                "wardryx",
                SchemaVersion::V0_2,
                "policy_deny",
                "high",
                json!({
                    "policy": "prod-deploy-requires-approval",
                    "reason": "no approval on file for deploy:prod scope",
                }),
            ),
            Call::new(
                "wardryx",
                SchemaVersion::V0_2,
                "approval_requested",
                "medium",
                json!({
                    "policy": "prod-deploy-requires-approval",
                    "reason": "awaiting operator approval",
                }),
            ),
            Call::new(
                "wardryx",
                SchemaVersion::V0_2,
                "approval_granted",
                "info",
                json!({
                    "policy": "prod-deploy-requires-approval",
                    "granted_by": "user://taipanbox.dev/j.doe",
                }),
            ),
        ]
    }
}

/// A memory write, occasionally followed by a contradiction against an
/// earlier memory (matching the canonical engram shapes exactly).
fn memory_calls(rank: usize, run_index: usize) -> Vec<Call> {
    let topic = TOPICS[run_index % TOPICS.len()];
    let memory_id = format!("mem-{:04}", 3000 + run_index);
    let mut calls = vec![Call::new(
        "engram",
        SchemaVersion::V0_1,
        "memory_written",
        "info",
        json!({"memory_id": memory_id.clone(), "topic": topic}),
    )];
    if rank % 3 != 2 {
        let conflicting_memory_id = format!("mem-{:04}", 2000 + run_index);
        calls.push(Call::new(
            "engram",
            SchemaVersion::V0_1,
            "contradiction_found",
            "medium",
            json!({
                "memory_id": memory_id,
                "conflicting_memory_id": conflicting_memory_id,
                "topic": topic,
            }),
        ));
    }
    calls
}

/// A quality score, occasionally followed by a regression against baseline
/// (matching the canonical verdryx `quality_drift` shape exactly).
fn quality_calls(rank: usize, run_index: usize, lcg: &mut Lcg) -> Vec<Call> {
    let eval_suite = EVAL_SUITES[run_index % EVAL_SUITES.len()];
    let current_score = jitter(lcg, 0.85, 0.98, 2);
    let mut calls = vec![Call::new(
        "verdryx",
        SchemaVersion::V0_2,
        "quality_score",
        "info",
        json!({"eval_suite": eval_suite, "current_score": current_score}),
    )];
    if rank % 3 != 2 {
        let baseline_score = round_to(current_score + jitter(lcg, 0.08, 0.16, 2), 2);
        let delta = round_to(current_score - baseline_score, 2);
        calls.push(Call::new(
            "verdryx",
            SchemaVersion::V0_2,
            "quality_drift",
            "high",
            json!({
                "eval_suite": eval_suite,
                "baseline_score": baseline_score,
                "current_score": current_score,
                "delta": delta,
            }),
        ));
    }
    calls
}

/// A fire-drill run, with occasional findings and a blast-radius measurement
/// (matching the canonical mockryx `blast_radius_measured` shape exactly).
fn drill_calls(rank: usize, run_index: usize, lcg: &mut Lcg) -> Vec<Call> {
    let scenario = SCENARIOS[run_index % SCENARIOS.len()];
    let mut calls = vec![Call::new(
        "mockryx",
        SchemaVersion::V0_2,
        "sim_run",
        "info",
        json!({"scenario": scenario, "status": "completed"}),
    )];
    if rank.is_multiple_of(2) {
        let finding = if rank.is_multiple_of(4) {
            "gap_found"
        } else {
            "guardrail_held"
        };
        calls.push(Call::new(
            "mockryx",
            SchemaVersion::V0_2,
            "sim_finding",
            "medium",
            json!({"scenario": scenario, "finding": finding}),
        ));
    }
    if rank.is_multiple_of(3) {
        calls.push(Call::new(
            "mockryx",
            SchemaVersion::V0_2,
            "blast_radius_measured",
            "medium",
            json!({
                "scenario": scenario,
                "blast_radius_score": jitter(lcg, 0.3, 0.75, 2),
                "affected_resources": 4 + lcg.next_range(20),
            }),
        ));
    }
    calls
}

/// A post-quantum crypto finding, occasionally followed by a signed evidence
/// record (matching the canonical qryx `evidence_signed` shape exactly).
fn crypto_calls(rank: usize, run_index: usize, agent_id: &str) -> Vec<Call> {
    let mut calls = vec![Call::new(
        "qryx",
        SchemaVersion::V0_1,
        "crypto_finding",
        "medium",
        json!({
            "algorithm": "rsa-2048",
            "risk": "quantum-vulnerable",
            "recommended": "ml-dsa-65",
        }),
    )];
    if rank.is_multiple_of(2) {
        calls.push(Call::new(
            "qryx",
            SchemaVersion::V0_1,
            "evidence_signed",
            "info",
            json!({
                "evidence_id": format!("ev-{:05}", 55_000 + run_index),
                "algorithm": "ml-dsa-65",
                "subject": agent_id,
            }),
        ));
    }
    calls
}

/// A small cross-plane filler on every non-block run, so all six planes stay
/// populated even in runs whose primary bucket lies elsewhere. Rotates
/// through wardryx/engram/verdryx, skipping whichever one the run's own
/// bucket already used so a single run never repeats the same source twice.
fn filler_call(i: usize, bucket: usize) -> Call {
    const FILLERS: [&str; 3] = ["wardryx", "engram", "verdryx"];
    let bucket_primary = match bucket {
        0 => "tokenfuse",
        1 => "wardryx",
        2 => "engram",
        3 => "verdryx",
        4 => "mockryx",
        _ => "qryx",
    };
    let mut idx = i % FILLERS.len();
    if FILLERS[idx] == bucket_primary {
        idx = (idx + 1) % FILLERS.len();
    }

    match FILLERS[idx] {
        "wardryx" => Call::new(
            "wardryx",
            SchemaVersion::V0_2,
            "policy_allow",
            "info",
            json!({"policy": "default-allow", "reason": "within policy"}),
        ),
        "engram" => Call::new(
            "engram",
            SchemaVersion::V0_1,
            "memory_written",
            "info",
            json!({
                "memory_id": format!("mem-{:04}", 9000 + i),
                "topic": TOPICS[i % TOPICS.len()],
            }),
        ),
        _ => {
            let score = 0.90 + (i % 7) as f64 * 0.01;
            Call::new(
                "verdryx",
                SchemaVersion::V0_2,
                "quality_score",
                "info",
                json!({
                    "eval_suite": EVAL_SUITES[i % EVAL_SUITES.len()],
                    "current_score": round_to(score, 2),
                }),
            )
        }
    }
}

/// Render one call into its final NDJSON line: schema, timestamp, source,
/// type, agent, severity, run, optional delegation chain, then `data`. Built
/// through `serde_json`, never string concatenation, so escaping is correct.
fn render_line(
    call: &Call,
    ts: &DateTime<Utc>,
    agent_id: &str,
    run_id: &str,
    on_behalf_of: &[&str],
) -> Result<String> {
    let mut obj = Map::new();
    obj.insert(
        "schema".to_string(),
        Value::String(call.schema.as_str().to_string()),
    );
    obj.insert(
        "ts".to_string(),
        Value::String(ts.to_rfc3339_opts(SecondsFormat::Millis, true)),
    );
    obj.insert("source".to_string(), Value::String(call.source.to_string()));
    obj.insert(
        "type".to_string(),
        Value::String(call.event_type.to_string()),
    );
    obj.insert("agent_id".to_string(), Value::String(agent_id.to_string()));
    obj.insert(
        "severity".to_string(),
        Value::String(call.severity.to_string()),
    );
    obj.insert("run_id".to_string(), Value::String(run_id.to_string()));
    if !on_behalf_of.is_empty() {
        obj.insert(
            "on_behalf_of".to_string(),
            Value::Array(
                on_behalf_of
                    .iter()
                    .map(|s| Value::String(s.to_string()))
                    .collect(),
            ),
        );
    }
    obj.insert("data".to_string(), call.data.clone());

    Ok(serde_json::to_string(&Value::Object(obj))?)
}
