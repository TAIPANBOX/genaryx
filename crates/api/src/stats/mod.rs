//! Per-agent counts of the things an operator asks about by name: how often an
//! agent was stopped, how often it behaved oddly, and how often it ran into its
//! budget.
//!
//! # WHY COUNTS LIVE HERE AND MONEY DOES NOT
//!
//! The Statistics view shows two column groups side by side, and only one of
//! them is this module's. Spend, calls and budgets come from the money plane
//! (`money_runs`), which is a different store with a different retention window
//! and its own idea of when a number stops existing. Folding both into one
//! aggregate here would produce a row whose columns silently disagree about
//! what "this month" means.
//!
//! So this module answers exactly one question, per agent: how many events of
//! each kind are on the bus right now. The frontend joins that to the money
//! numbers by `agent_id` and labels the two windows separately.
//!
//! # WHY THE VOCABULARY IS PINNED HERE AND ALSO OPEN
//!
//! The three named counters below are a reading of agent-passport SPEC 6.2, and
//! a reading can be wrong or go stale. So every event also lands in
//! [`AgentStats::by_type`] under its own raw type, whether or not this build
//! recognizes it. A type this console has never heard of is then visible as
//! itself instead of being folded into an "other" bucket or dropped, which is
//! the same rule `egress::EgressTotals::by_verdict` follows and for the same
//! reason: the vocabulary belongs to the products, not to the console.
//!
//! # THE ONE THING THIS MUST NOT DO
//!
//! It must not return zeros when it could not look. `bus::recent_events` falls
//! back to `mock_events` when its store is unavailable, which is right for an
//! explorer that must render something and wrong here, where a zero in the
//! "blocked" column is read as "this agent was never stopped". So the shape
//! carries [`StatsPanel::measured`] and a note, and the frontend is required to
//! render the note rather than the table.

use serde::Serialize;
use std::collections::BTreeMap;

use crate::bus::AppState;

/// Events that mean an agent was stopped by one of our services.
///
/// Read from agent-passport SPEC 6.2 plus the two planes that emit outside it
/// today: the TokenFuse gateway's enforcement path and the egress plane.
///
/// WHO stopped it is a separate axis, carried in
/// [`AgentStats::blocked_by_operator`]: the same event set, split by whether a
/// human pulled the switch or a service did. See that field for what the split
/// can and cannot see.
///
/// `console_command` is deliberately NOT here, and it is the one exclusion
/// worth stating. One operator kill writes two lines: a `console_command` into
/// the console's own chain, and a `run_killed` from the money plane that
/// actually enforced it. Counting both would report every manual kill twice,
/// and the second copy would look like an independent enforcement rather than
/// an audit record of the first.
const BLOCKED_TYPES: &[&str] = &[
    // wardryx, the policy decision point.
    "policy_deny",
    "approval_denied",
    "approval_timeout",
    "approval_unanswered",
    // tokenfuse, the enforcement path.
    "breaker_tripped",
    "dlp_block",
    "taint_block",
    "mcp_drift",
    "identity_mismatch",
    "run_killed",
    "unit_cap_exceeded",
    // scopyx, the egress plane.
    "web_blocked",
];

/// Events that mean an agent did something odd without necessarily being
/// stopped for it.
///
/// Idryx's seven detector names (`behavior_anomaly`, `impossible_travel` and
/// the rest) are deliberately absent. SPEC 6.2 marks that whole row RESERVED:
/// idryx's detections leave by OTLP and by Slack and never enter this envelope,
/// so a column counting them would sit at zero forever while an operator read
/// the zero as good news.
const ANOMALY_TYPES: &[&str] = &[
    // tokenfuse, the three runaway shapes.
    "sustained_loop",
    "spend_spike",
    "fanout_explosion",
    // verdryx, engram, mockryx.
    "quality_drift",
    "contradiction_found",
    "sim_finding",
];

/// Events about an agent's budget.
///
/// `budget_threshold` is the 80% early warning and `budget_exhausted` is the
/// cap being reached. Both are counted here. Neither is where the amounts
/// reliably come from (the Cloud exports `budget_exhausted` without them), so
/// [`overshoot_of`] reads any event that carried a budget and a spend, not just
/// these two.
const BUDGET_TYPES: &[&str] = &["budget_exhausted", "budget_threshold"];

/// The console's own `data.action` values that mean "an operator halted this".
///
/// A freeze or a unit stop is enforced by writing an ordinary deny-all wardryx
/// policy, so the refusals that follow are indistinguishable from any other
/// policy denial on the bus. The ACTION, though, is journaled: one
/// `console_command` per toggle, carrying the entity in `data.target` and the
/// agents it actually halted in `data.members`
/// (`genaryx/crates/web/src/dispatch.rs`, `journal_block`). That record is what
/// this reads, which is why it is the one `console_command` shape counted here.
///
/// The UNBLOCK actions are absent on purpose: starting something again is not a
/// stop, and counting it would make an operator who froze and unfroze an agent
/// look twice as heavy-handed as one who left it frozen.
///
/// `console.kill_run` is absent for a different reason: the money plane emits
/// its own `run_killed` for the same kill, and that one is already counted.
const OPERATOR_BLOCK_ACTIONS: &[&str] = &[
    "console.block_agent",
    "console.block_unit",
    "console.block_user",
];

/// The console's own event type, whose `agent_id` is the CONSOLE rather than
/// the agent it acted on.
const CONSOLE_COMMAND: &str = "console_command";

/// Every agent one console block halted, read from the record it wrote.
///
/// `data.target` is the entity: an agent id for a freeze, a unit id or a user
/// handle otherwise. `data.members` is the list of agents the policies were
/// actually written for, which is the same single agent for a freeze and the
/// whole membership for a unit or user stop. Reading members rather than
/// re-deriving the membership matters: a unit's roster at read time is not
/// necessarily its roster at the moment the operator stopped it.
fn agents_halted_by(data: Option<&serde_json::Value>) -> Vec<String> {
    let Some(data) = data else { return Vec::new() };
    let action = data.get("action").and_then(|v| v.as_str()).unwrap_or("");
    if !OPERATOR_BLOCK_ACTIONS.contains(&action) {
        return Vec::new();
    }
    let members: Vec<String> = data
        .get("members")
        .and_then(|v| v.as_array())
        .map(|xs| {
            xs.iter()
                .filter_map(|x| x.as_str())
                .filter(|x| x.starts_with("agent://"))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    if !members.is_empty() {
        return members;
    }
    // A record written before `members` was added, or a stop that halted
    // nothing. Fall back to the target when it is itself an agent; a unit id
    // or a user handle names no agent and is deliberately not guessed at.
    data.get("target")
        .and_then(|v| v.as_str())
        .filter(|t| t.starts_with("agent://"))
        .map(|t| vec![t.to_string()])
        .unwrap_or_default()
}

/// One agent's counts. Every field is a count of lines actually on the bus, so
/// a zero means "none in this window", never "not measured": that distinction
/// is carried once, by [`StatsPanel::measured`], for the whole panel.
#[derive(Debug, Clone, Serialize, Default)]
pub struct AgentStats {
    pub agent_id: String,
    /// Every stop, whoever caused it.
    pub blocked: usize,

    /// The subset of `blocked` a HUMAN caused, so the other part of the column
    /// is what the services did on their own.
    ///
    /// Two events say so in their own data and nothing else does:
    ///
    /// - `run_killed` carries `data.actor` when an operator pulled it and
    ///   omits the field entirely when the plane killed the run itself
    ///   (`tokenfuse/crates/cloud/src/store.rs`, `emit_run_killed`).
    /// - `approval_denied` carries `data.decided_by`: a person answered the
    ///   hold with a no. Its siblings `approval_timeout` and
    ///   `approval_unanswered` are the opposite case, nobody answered, so they
    ///   stay on the system side.
    ///
    /// WHAT THIS CANNOT SEE, and it matters: an operator FREEZE is enforced by
    /// writing an ordinary deny-all wardryx policy
    /// (`genaryx/crates/web/src/lifecycle.rs`), so the refusals that follow
    /// arrive as plain `policy_deny` with an ordinary PDP reason. Nothing on
    /// the bus marks them as the operator's doing, and this counter does not
    /// guess: they land on the system side. The freeze ITSELF emits no event at
    /// all, so it is never counted here as a stop in its own right.
    pub blocked_by_operator: usize,

    pub anomalies: usize,
    pub budget_events: usize,

    /// The WORST single breach recorded for this agent, in micro-USD, not the
    /// sum of them.
    ///
    /// A sum is the wrong shape here and it is wrong in the direction that
    /// flatters nobody: one runaway run trips its breaker on every call, so
    /// twenty-six events describing ONE overspend of $0.40 would add up to
    /// $10.40 of overspend that never happened. How often it happened is
    /// already the `budget_events` column; this is how bad it got.
    ///
    /// `None` when no event in this window carried both amounts, which is a
    /// real case rather than an exotic one: the Cloud's incident aggregator
    /// exports `budget_exhausted` with `{org, occurrences}` and no amounts at
    /// all. `None` and `Some(0)` are different facts and the frontend renders
    /// them differently: "not recorded" against "did not exceed".
    pub worst_overshoot_microusd: Option<i64>,

    /// Every event type seen for this agent, counted under its own raw name,
    /// including the ones no counter above recognizes. See the module doc.
    pub by_type: BTreeMap<String, usize>,

    /// The newest event timestamp seen for this agent, so a table can sort by
    /// recency without a second read.
    pub last_seen: String,
}

/// What the panel renders.
#[derive(Debug, Clone, Serialize)]
pub struct StatsPanel {
    /// False when nothing could be read. The frontend must show `note` in that
    /// case and must NOT render the empty table as an answer.
    pub measured: bool,
    pub note: Option<String>,
    /// How many bus lines this panel actually looked at, so a reader can tell a
    /// quiet estate from a short window.
    pub scanned: usize,
    pub agents: Vec<AgentStats>,
}

impl StatsPanel {
    fn unmeasured(note: impl Into<String>) -> Self {
        Self {
            measured: false,
            note: Some(note.into()),
            scanned: 0,
            agents: Vec::new(),
        }
    }
}

/// Whether a stop was a person's decision, read from the event's own data.
///
/// Absent fields mean "no, or not recorded", and both fall to the system side:
/// the honest default for an unattributed enforcement is the service that
/// enforced it, not a human nobody named.
fn is_operator_stop(type_: &str, data: Option<&serde_json::Value>) -> bool {
    let field = match type_ {
        "run_killed" => "actor",
        "approval_denied" => "decided_by",
        _ => return false,
    };
    data.and_then(|d| d.get(field))
        .and_then(|v| v.as_str())
        .is_some_and(|v| !v.trim().is_empty())
}

/// How far over budget one event says the agent went, in micro-USD.
///
/// Two spellings, because two real producers write two different ones and a
/// reader that knew only one would report an empty column on live data:
///
/// - `budget_micros` / `spent_micros`: the Cloud's `budget_threshold` export
///   (`crates/cloud/src/store.rs`).
/// - `budget_usd` / `spent_usd`: the TokenFuse gateway's `breaker_tripped`
///   (`crates/gateway/src/proxy.rs`), and the demo feeder's own lines.
///
/// Read from ANY event carrying the pair rather than only from the named budget
/// types, because `breaker_tripped` is the enforcement event and is precisely
/// where the live gateway records the two numbers.
///
/// Returns `None` when neither pair is present, which is not the same as zero:
/// see [`AgentStats::worst_overshoot_microusd`].
fn overshoot_of(data: &serde_json::Value) -> Option<i64> {
    if let (Some(budget), Some(spent)) = (
        data.get("budget_micros").and_then(|v| v.as_i64()),
        data.get("spent_micros").and_then(|v| v.as_i64()),
    ) {
        return Some((spent - budget).max(0));
    }
    let budget = data.get("budget_usd")?.as_f64()?;
    let spent = data.get("spent_usd")?.as_f64()?;
    let over = (spent - budget).max(0.0) * 1_000_000.0;
    // The demo feeder works in fractions of a cent, so rounding rather than
    // truncating keeps a real sub-cent breach from reading as no breach.
    Some(over.round() as i64)
}

/// Per-agent counts over the recent bus window.
///
/// `scan` bounds the lines READ. Unlike a per-row panel there is nothing to cap
/// on the way out: the result is one row per agent seen, and an estate with
/// more agents than that is a fact the operator should see rather than a list
/// this function truncates.
pub fn stats_counts(scan: usize, state: &AppState) -> StatsPanel {
    let Some(dir) = &state.events_dir else {
        return StatsPanel::unmeasured(
            "The console has no event store on this box, so nothing here was counted. \
             This is not a report that your agents were never stopped.",
        );
    };

    let db_path = dir.join("console.sqlite");
    let store = match genaryx_core::store::Store::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            return StatsPanel::unmeasured(format!(
                "The event store could not be opened ({e}), so nothing here was counted. \
                 This is not a report that your agents were never stopped."
            ));
        }
    };

    let rows = match store.recent_events(scan) {
        Ok(r) => r,
        Err(e) => {
            return StatsPanel::unmeasured(format!(
                "The event store could not be queried ({e}), so nothing here was counted. \
                 This is not a report that your agents were never stopped."
            ));
        }
    };

    let scanned = rows.len();
    let mut by_agent: BTreeMap<String, AgentStats> = BTreeMap::new();
    let mut halts: Vec<String> = Vec::new();

    for e in rows {
        let entry = by_agent
            .entry(e.agent_id.clone())
            .or_insert_with(|| AgentStats {
                agent_id: e.agent_id.clone(),
                ..Default::default()
            });

        let t = e.type_.as_str();
        *entry.by_type.entry(e.type_.clone()).or_insert(0) += 1;

        if BLOCKED_TYPES.contains(&t) {
            entry.blocked += 1;
            if is_operator_stop(t, e.data.as_ref()) {
                entry.blocked_by_operator += 1;
            }
        }
        if ANOMALY_TYPES.contains(&t) {
            entry.anomalies += 1;
        }
        if BUDGET_TYPES.contains(&t) {
            entry.budget_events += 1;
        }
        // Deliberately outside the `BUDGET_TYPES` check: `breaker_tripped` is
        // the gateway's enforcement event, not one of the named budget types,
        // and it is where a live box actually records the two amounts.
        if let Some(over) = e.data.as_ref().and_then(overshoot_of) {
            entry.worst_overshoot_microusd =
                Some(entry.worst_overshoot_microusd.unwrap_or(0).max(over));
        }

        // A console block names the agents it halted, and they are not this
        // event's own `agent_id` (that is the console). Collected here and
        // applied after the loop, so one operator action lands on every agent
        // it actually stopped.
        for halted in agents_halted_by(e.data.as_ref()) {
            if e.type_ == CONSOLE_COMMAND {
                halts.push(halted);
            }
        }

        // `recent_events` is newest-first by insertion id, but `ts` is the
        // producer's clock and the ingest pipeline drains one file at a time,
        // so the first row seen for an agent is not reliably its latest event.
        // Keep the maximum instead of the first.
        if e.ts > entry.last_seen {
            entry.last_seen = e.ts;
        }
    }

    // An operator stop counts for every agent it halted, on top of whatever
    // the services did to them. The agent gets a row even if the bus holds
    // nothing else about it: "frozen by an operator, and otherwise silent" is
    // a true and useful line.
    for agent_id in halts {
        let entry = by_agent
            .entry(agent_id.clone())
            .or_insert_with(|| AgentStats {
                agent_id,
                ..Default::default()
            });
        entry.blocked += 1;
        entry.blocked_by_operator += 1;
    }

    StatsPanel {
        measured: true,
        note: Some(format!(
            "Counted from the {scanned} most recent events on the bus, which is what this \
             console has ingested since it started. An older event than that is not counted here."
        )),
        scanned,
        agents: by_agent.into_values().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::BusMode;
    use genaryx_core::event::{AgentEvent, ConsoleEvent, Provenance, SchemaVersion};
    use genaryx_core::store::Store;
    use std::path::PathBuf;

    fn empty_state() -> AppState {
        AppState {
            events_dir: None,
            source_events_dir: None,
            mode: BusMode::Unavailable {
                reason: "test".into(),
            },
        }
    }

    fn nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }

    fn event(agent: &str, ts: &str, type_: &str, data: serde_json::Value) -> ConsoleEvent {
        ConsoleEvent {
            event: AgentEvent {
                schema: SchemaVersion::SCHEMA_V0_2.to_string(),
                ts: ts.to_string(),
                source: "test".into(),
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
                connector: "test".into(),
                file: None,
                offset: None,
                endpoint: None,
                received_ts: ts.to_string(),
            },
            raw: "{}".into(),
            schema_version: SchemaVersion::V0_2,
        }
    }

    /// A temp events dir holding a `console.sqlite` seeded with `events`, in
    /// the exact place [`stats_counts`] looks for it.
    ///
    /// `tag` is per TEST and is not decoration. Keyed on pid and a timestamp
    /// alone, two of these tests running on parallel cargo threads landed in
    /// the same directory and read each other's events: the first version of
    /// this file failed two tests that the change under test could not
    /// possibly have affected, which is how the collision was found. A flaky
    /// test is worse than no test, so the name carries something that cannot
    /// collide.
    fn seeded_state(tag: &str, events: &[ConsoleEvent]) -> (AppState, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "genaryx-stats-test-{}-{tag}-{}",
            std::process::id(),
            nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create_dir_all");
        let store = Store::open(&dir.join("console.sqlite")).expect("Store::open");
        store.insert_batch(events).expect("insert_batch");
        (
            AppState {
                events_dir: Some(dir.clone()),
                source_events_dir: None,
                mode: BusMode::Unavailable {
                    reason: "test".into(),
                },
            },
            dir,
        )
    }

    /// The counting itself, over a store that really holds these lines: one
    /// agent stopped twice by two different planes, once odd, once over
    /// budget, plus a type this build has never heard of.
    #[test]
    fn it_counts_each_kind_and_keeps_an_unknown_type_visible() {
        let a = "agent://acme.local/support/tier1";
        let b = "agent://acme.local/sre/janitor";
        // Inserted deliberately OUT of timestamp order, with the newest line
        // first. `recent_events` returns newest-INSERTED first, so the first
        // row this fold sees for `a` is 10:04 while its latest event is 10:05.
        // That is the real shape (the ingest pipeline drains one file's whole
        // backlog before the next), and an implementation that kept the first
        // ts instead of the maximum passes any test seeded in order.
        let (state, dir) = seeded_state(
            "counts",
            &[
                event(
                    a,
                    "2026-08-09T10:05:00Z",
                    "some_future_type",
                    serde_json::json!({}),
                ),
                event(
                    a,
                    "2026-08-09T10:00:00Z",
                    "policy_deny",
                    serde_json::json!({}),
                ),
                event(
                    a,
                    "2026-08-09T10:01:00Z",
                    "dlp_block",
                    serde_json::json!({}),
                ),
                event(
                    a,
                    "2026-08-09T10:02:00Z",
                    "sustained_loop",
                    serde_json::json!({}),
                ),
                event(
                    a,
                    "2026-08-09T10:03:00Z",
                    "budget_threshold",
                    serde_json::json!({ "budget_micros": 1_000_000, "spent_micros": 1_250_000 }),
                ),
                // The gateway's own shape: USD floats on an enforcement event that
                // is not one of the named budget types, and a WORSE breach than
                // the threshold line above.
                event(
                    a,
                    "2026-08-09T10:03:30Z",
                    "breaker_tripped",
                    serde_json::json!({ "budget_usd": 1.0, "spent_usd": 1.75, "reason": "budget_exceeded" }),
                ),
                // A second, smaller breach: the worst must win, and the three must
                // not be added into an overspend that never happened.
                event(
                    a,
                    "2026-08-09T10:03:45Z",
                    "breaker_tripped",
                    serde_json::json!({ "budget_usd": 1.0, "spent_usd": 1.10, "reason": "budget_exceeded" }),
                ),
                // The Cloud's own export shape: a budget event with no amounts.
                event(
                    a,
                    "2026-08-09T10:04:00Z",
                    "budget_exhausted",
                    serde_json::json!({ "org": "acme", "occurrences": 2 }),
                ),
                // A second agent, so the fold is proven to separate them.
                event(
                    b,
                    "2026-08-09T09:00:00Z",
                    "policy_deny",
                    serde_json::json!({}),
                ),
            ],
        );

        let p = stats_counts(500, &state);
        assert!(p.measured);
        assert_eq!(p.scanned, 9);
        assert_eq!(p.agents.len(), 2, "two agents seen, two rows");

        let row = p
            .agents
            .iter()
            .find(|r| r.agent_id == a)
            .expect("the busy agent must have a row");
        assert_eq!(
            row.blocked, 4,
            "policy_deny, dlp_block and the two breaker_tripped enforcement events"
        );
        assert_eq!(row.anomalies, 1);
        assert_eq!(row.budget_events, 2, "the warning and the exhaustion");
        assert_eq!(
            row.worst_overshoot_microusd,
            Some(750_000),
            "the worst single breach ($1.75 against $1.00), read from the gateway's USD \
             spelling, and NOT the sum of the three breaches"
        );
        assert_eq!(
            row.last_seen, "2026-08-09T10:05:00Z",
            "the newest ts for this agent, not the first row read"
        );
        assert_eq!(
            row.by_type.get("some_future_type"),
            Some(&1),
            "an unrecognized type stays visible under its own name"
        );

        let quiet = p
            .agents
            .iter()
            .find(|r| r.agent_id == b)
            .expect("the second agent must have its own row");
        assert_eq!(quiet.blocked, 1);
        assert_eq!(quiet.anomalies, 0);
        assert_eq!(
            quiet.worst_overshoot_microusd, None,
            "no event carried amounts at all, which is 'not recorded', not zero"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The property this module exists to have. A panel reporting zero blocks
    /// when it could not look tells an operator their agents were never
    /// stopped, which is the one wrong answer that reads as good news.
    #[test]
    fn with_no_store_it_says_it_could_not_look_rather_than_reporting_zero() {
        let p = stats_counts(500, &empty_state());
        assert!(!p.measured, "an unread panel must not claim to be measured");
        let note = p.note.expect("an unmeasured panel must say why");
        assert!(
            note.contains("not a report that your agents were never stopped"),
            "the note must refuse the wrong reading explicitly, got: {note}"
        );
        assert!(p.agents.is_empty());
        assert_eq!(p.scanned, 0);
    }

    /// `console_command` and `run_killed` are two lines for one kill. Counting
    /// both would double every operator kill in the blocked column.
    #[test]
    fn a_console_command_is_not_counted_as_a_block() {
        assert!(
            !BLOCKED_TYPES.contains(&"console_command"),
            "console_command is the audit record of a kill, and run_killed is the kill"
        );
        assert!(BLOCKED_TYPES.contains(&"run_killed"));
    }

    /// SPEC 6.2 marks the whole idryx row RESERVED: those detections leave by
    /// OTLP and by Slack and never enter this envelope. A counter for one would
    /// read zero forever, and an operator would read the zero as good news.
    #[test]
    fn the_reserved_idryx_detector_names_are_not_counted() {
        for reserved in [
            "behavior_anomaly",
            "impossible_travel",
            "excessive_privilege",
            "mfa_fatigue",
            "new_device",
            "blast_radius_change",
            "attestation_missing",
        ] {
            assert!(
                !ANOMALY_TYPES.contains(&reserved) && !BLOCKED_TYPES.contains(&reserved),
                "{reserved} is RESERVED in SPEC 6.2 and nothing emits it, so a column \
                 counting it would sit at zero forever"
            );
        }
    }

    /// A freeze emits no enforcement event of its own: the deny-all policy it
    /// writes produces ordinary `policy_deny` lines that name nobody. The one
    /// record of WHO did it is the console's own journal line, and this is the
    /// test that it reaches the agent it froze rather than the console.
    #[test]
    fn an_operator_freeze_counts_against_the_agent_it_froze() {
        let frozen = "agent://acme.local/sre/janitor";
        let console = "agent://acme.local/console/box";
        let (state, dir) = seeded_state(
            "freeze",
            &[event(
                console,
                "2026-08-09T10:00:00Z",
                "console_command",
                serde_json::json!({
                    "action": "console.block_agent",
                    "target": frozen,
                    "members": [frozen],
                    "policies": 1,
                }),
            )],
        );

        let p = stats_counts(500, &state);
        let row = p
            .agents
            .iter()
            .find(|r| r.agent_id == frozen)
            .expect("the frozen agent must get a row even with nothing else on the bus");
        assert_eq!(row.blocked, 1);
        assert_eq!(row.blocked_by_operator, 1);

        let console_row = p.agents.iter().find(|r| r.agent_id == console).unwrap();
        assert_eq!(
            console_row.blocked, 0,
            "the console did the stopping, it was not stopped"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Stopping a unit halts everything in it, and the record says which. The
    /// membership at the time of the stop is not recoverable later, so reading
    /// it back out of the event is the only honest attribution.
    #[test]
    fn a_unit_stop_counts_against_every_agent_it_halted() {
        let a = "agent://acme.local/sre/one";
        let b = "agent://acme.local/sre/two";
        let (state, dir) = seeded_state(
            "unit-stop",
            &[event(
                "agent://acme.local/console/box",
                "2026-08-09T10:00:00Z",
                "console_command",
                serde_json::json!({
                    "action": "console.block_unit",
                    "target": "sre",
                    "members": [a, b],
                    "policies": 2,
                }),
            )],
        );

        let p = stats_counts(500, &state);
        for id in [a, b] {
            let row = p.agents.iter().find(|r| r.agent_id == id).unwrap();
            assert_eq!(row.blocked_by_operator, 1, "{id} was halted by the stop");
        }
        assert!(
            !p.agents.iter().any(|r| r.agent_id == "sre"),
            "a unit id is not an agent and must never become a row"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Starting something again is not a stop. An operator who froze and then
    /// unfroze must not read as twice as heavy-handed as one who left it
    /// frozen.
    #[test]
    fn an_unblock_is_not_counted_as_a_stop() {
        let agent = "agent://acme.local/sre/janitor";
        let (state, dir) = seeded_state(
            "unblock",
            &[event(
                "agent://acme.local/console/box",
                "2026-08-09T10:01:00Z",
                "console_command",
                serde_json::json!({
                    "action": "console.unblock_agent",
                    "target": agent,
                    "members": [agent],
                    "policies": 1,
                }),
            )],
        );

        let p = stats_counts(500, &state);
        assert!(
            !p.agents.iter().any(|r| r.blocked_by_operator > 0),
            "an unblock is a release, not a stop"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The kill still must not double count. The console journals
    /// `console.kill_run` AND the money plane emits `run_killed` for the same
    /// act, and only the second is counted.
    #[test]
    fn a_console_kill_is_counted_once_not_twice() {
        let agent = "agent://acme.local/sre/janitor";
        let (state, dir) = seeded_state(
            "kill-once",
            &[
                event(
                    "agent://acme.local/console/box",
                    "2026-08-09T10:00:00Z",
                    "console_command",
                    serde_json::json!({ "action": "console.kill_run", "target": "run-1" }),
                ),
                event(
                    agent,
                    "2026-08-09T10:00:01Z",
                    "run_killed",
                    serde_json::json!({ "org": "acme", "actor": "user://acme.local/d.hayes" }),
                ),
            ],
        );

        let p = stats_counts(500, &state);
        let row = p.agents.iter().find(|r| r.agent_id == agent).unwrap();
        assert_eq!(row.blocked, 1, "one kill, one stop");
        assert_eq!(row.blocked_by_operator, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A kill by a person and a kill by the plane are the same event type and
    /// differ only by one field. Getting this backwards would credit the
    /// services with an operator's decisions, or blame an operator for the
    /// breaker doing its job.
    #[test]
    fn a_stop_is_the_operators_only_when_the_event_names_one() {
        let a = "agent://acme.local/sre/janitor";
        let (state, dir) = seeded_state(
            "operator",
            &[
                // The plane killed this one: no actor field at all.
                event(
                    a,
                    "2026-08-09T10:00:00Z",
                    "run_killed",
                    serde_json::json!({ "org": "acme" }),
                ),
                // An operator killed this one.
                event(
                    a,
                    "2026-08-09T10:01:00Z",
                    "run_killed",
                    serde_json::json!({ "org": "acme", "actor": "user://acme.local/d.hayes" }),
                ),
                // A person answered the hold with a no.
                event(
                    a,
                    "2026-08-09T10:02:00Z",
                    "approval_denied",
                    serde_json::json!({ "approval_id": "ap-1", "decided_by": "d.hayes" }),
                ),
                // Nobody answered: the opposite case, and it stays with the system.
                event(
                    a,
                    "2026-08-09T10:03:00Z",
                    "approval_unanswered",
                    serde_json::json!({}),
                ),
                // An empty actor names nobody, so it is not an operator stop.
                event(
                    a,
                    "2026-08-09T10:04:00Z",
                    "run_killed",
                    serde_json::json!({ "org": "acme", "actor": "  " }),
                ),
                // Ordinary enforcement.
                event(
                    a,
                    "2026-08-09T10:05:00Z",
                    "policy_deny",
                    serde_json::json!({}),
                ),
            ],
        );

        let p = stats_counts(500, &state);
        let row = &p.agents[0];
        assert_eq!(row.blocked, 6, "every one of them is a stop");
        assert_eq!(
            row.blocked_by_operator, 2,
            "the named kill and the answered denial, and nothing else"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An operator freeze writes an ordinary deny-all policy, so its refusals
    /// arrive as plain `policy_deny` and nothing on the bus marks them. The
    /// counter must not guess: it credits them to the system rather than
    /// inventing an attribution the event does not carry.
    #[test]
    fn a_frozen_agents_refusals_are_not_guessed_to_be_the_operators() {
        assert!(!is_operator_stop(
            "policy_deny",
            Some(&serde_json::json!({ "reason": "cost above threshold" }))
        ));
        assert!(!is_operator_stop(
            "breaker_tripped",
            Some(&serde_json::json!({ "reason": "budget_exceeded" }))
        ));
    }

    /// An event with no amounts must leave `overshoot` absent rather than
    /// zero: "not recorded" and "did not exceed" are different facts, and the
    /// Cloud's own `budget_exhausted` export carries no amounts at all.
    #[test]
    fn an_absent_amount_is_not_an_overshoot_of_zero() {
        let no_amounts = serde_json::json!({ "org": "acme", "occurrences": 3 });
        assert_eq!(overshoot_of(&no_amounts), None);

        let under = serde_json::json!({ "budget_micros": 2_000_000, "spent_micros": 1_600_000 });
        assert_eq!(
            overshoot_of(&under),
            Some(0),
            "an 80% warning is a budget event with no overshoot, which is not the same \
             as an event that never said"
        );

        let over = serde_json::json!({ "budget_micros": 2_000_000, "spent_micros": 2_500_000 });
        assert_eq!(overshoot_of(&over), Some(500_000));
    }
}
