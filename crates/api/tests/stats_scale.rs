//! What the Statistics panel costs on an estate big enough to hurt.
//!
//! `#[ignore]`d, because it seeds 378,000 events and takes minutes. Run it by
//! name:
//!
//! ```sh
//! cargo test -p genaryx-api --release --test stats_scale -- --ignored --nocapture
//! ```
//!
//! # WHY THIS IS A COMMITTED TEST AND NOT A SCRIPT SOMEBODY RAN ONCE
//!
//! Two decisions in this codebase are justified by numbers from this bench and
//! by nothing else: the `idx_events_agent_type_ts_ms` index (migration v6 in
//! `genaryx_core::store`) and the 100,000 detail cap in
//! `apps/web/src/lib/stats.ts`. Both say "@measured" in their own comments. A
//! measurement whose command no longer exists is an assertion wearing a badge,
//! so the command exists.
//!
//! It asserts almost nothing, deliberately: timings vary by machine and a
//! threshold here would go red on somebody's laptop and get deleted. It prints,
//! and it holds the one thing that is not a timing, that the aggregate counts
//! every seeded event rather than a capped slice of them.

use genaryx_core::event::{AgentEvent, ConsoleEvent, Provenance, SchemaVersion};
use genaryx_core::store::Store;

const AGENTS: u32 = 42;
const DAYS: i64 = 90;
const PER_AGENT_PER_DAY: u32 = 100;

/// The event mix, weighted out of 100 and shaped like the demo feeder: mostly
/// ordinary traffic, a minority carrying something in `data` that no count can
/// express. The proportion is the number that matters, because it decides what
/// fraction of the bus the detail read has to open.
const MIX: &[(&str, u32)] = &[
    ("policy_allow", 55),
    ("tool_call", 20),
    ("policy_deny", 8),
    ("identity_finding", 5),
    ("breaker_tripped", 4),
    ("run_killed", 3),
    ("approval_denied", 2),
    ("budget_exhausted", 2),
    ("web_blocked", 1),
];

/// The same two lists `genaryx_api::stats` passes, copied rather than exported:
/// this bench measures the SHAPE of the query, and a private constant should
/// not become public API to be benchmarked.
const DETAIL_TYPES: &[&str] = &[
    "identity_finding",
    "run_killed",
    "approval_denied",
    "console_command",
];
const AMOUNT_FIELDS: &[(&str, &str)] = &[
    ("budget_micros", "spent_micros"),
    ("budget_usd", "spent_usd"),
];

fn event(agent: &str, ts: &str, type_: &str, data: serde_json::Value) -> ConsoleEvent {
    let raw = serde_json::json!({
        "schema": SchemaVersion::SCHEMA_V0_2,
        "ts": ts,
        "type": type_,
        "agent_id": agent,
        "data": data,
    })
    .to_string();
    ConsoleEvent {
        event: AgentEvent {
            schema: SchemaVersion::SCHEMA_V0_2.to_string(),
            ts: ts.to_string(),
            source: "bench".into(),
            event_type: type_.to_string(),
            agent_id: agent.to_string(),
            severity: None,
            run_id: None,
            on_behalf_of: Vec::new(),
            data: Some(data),
            prev_hash: None,
            extra: Default::default(),
        },
        provenance: Provenance {
            env: "local".into(),
            connector: "bench".into(),
            file: None,
            offset: None,
            endpoint: None,
            received_ts: ts.to_string(),
        },
        raw,
        schema_version: SchemaVersion::V0_2,
    }
}

fn type_for(k: u32) -> &'static str {
    let mut pick = k % 100;
    for (name, weight) in MIX {
        if pick < *weight {
            return name;
        }
        pick -= *weight;
    }
    "policy_allow"
}

fn data_for(type_: &str, k: u32) -> serde_json::Value {
    match type_ {
        "identity_finding" => serde_json::json!({ "detector": "over_privileged_nhi" }),
        "breaker_tripped" => serde_json::json!({ "budget_usd": 1.0, "spent_usd": 1.4 }),
        "run_killed" => serde_json::json!({ "actor": "user://acme.local/d.hayes" }),
        "approval_denied" => serde_json::json!({ "decided_by": "d.hayes" }),
        _ => serde_json::json!({ "seq": k }),
    }
}

#[test]
#[ignore = "seeds 378,000 events; run with --ignored"]
fn what_the_statistics_panel_costs_at_scale() {
    let dir = std::env::temp_dir().join(format!("genaryx-stats-scale-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create_dir_all");
    let db = dir.join("console.sqlite");
    let store = Store::open(&db).expect("Store::open");

    let now = chrono::Utc::now();
    let mut batch = Vec::new();
    let mut seeded = 0usize;
    for day in 0..DAYS {
        for a in 0..AGENTS {
            let agent = format!("agent://acme.local/u{}/a{}", a % 9, a);
            for k in 0..PER_AGENT_PER_DAY {
                let type_ = type_for(k);
                let ts = (now - chrono::Duration::days(DAYS - 1 - day)
                    + chrono::Duration::seconds(i64::from(k) * 37))
                .to_rfc3339();
                batch.push(event(&agent, &ts, type_, data_for(type_, k)));
                seeded += 1;
            }
        }
        // One transaction per day, so peak memory is a day and not a quarter.
        store.insert_batch(&batch).expect("insert_batch");
        batch.clear();
    }
    let bytes = std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0);
    println!(
        "\nseeded {seeded} events, {:.0} MB on disk",
        bytes as f64 / 1e6
    );

    let cutoff = Some(now.timestamp_millis() - DAYS * 86_400_000);

    let t = std::time::Instant::now();
    let counts = store.type_counts_since(cutoff).expect("type_counts_since");
    let elapsed = t.elapsed();
    let counted: u64 = counts.iter().map(|c| c.count).sum();
    println!(
        "aggregate: {} groups covering {counted} events in {elapsed:?}",
        counts.len()
    );

    // The one real assertion. Every seeded event is in the window, so an
    // aggregate that reports fewer has a cap in it somewhere, which is the
    // exact defect this design replaced.
    assert_eq!(
        counted, seeded as u64,
        "the aggregate must count the whole window, not a slice of it"
    );

    let t = std::time::Instant::now();
    let all = store
        .events_of_types_since(DETAIL_TYPES, AMOUNT_FIELDS, cutoff, usize::MAX)
        .expect("events_of_types_since");
    println!(
        "detail, uncapped: {} rows ({:.1}% of the bus) in {:?}",
        all.len(),
        all.len() as f64 * 100.0 / counted as f64,
        t.elapsed()
    );

    // What the shipped cap does on this estate. `STATS_SCAN` in
    // apps/web/src/lib/stats.ts is the first of these.
    for cap in [100_000usize, 20_000] {
        let t = std::time::Instant::now();
        let rows = store
            .events_of_types_since(DETAIL_TYPES, AMOUNT_FIELDS, cutoff, cap)
            .expect("events_of_types_since");
        let truncated = rows.len() >= cap;
        println!(
            "detail, cap {cap}: {} rows in {:?}{}",
            rows.len(),
            t.elapsed(),
            if truncated { "  <- TRUNCATED" } else { "" }
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}
