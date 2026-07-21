//! `ExceptionEngine`: consumes `CloudSse` + periodic `CloudClient` reads and
//! maintains the phone-facing exception queue (docs/PHASE5.md "exceptions"
//! module; itrat-console/13 D12.2b).
//!
//! Semantics are a deliberate PORT of `tokenfuse-cloud::push::PushPipeline`
//! (`~/Development/tokenfuse/crates/cloud/src/push.rs`), not a re-import: the
//! relay feeds from `/v1/stream`, not the Cloud's in-process store bus, so the
//! shapes differ, but the thresholds and dedup windows are copied faithfully
//! (`alert_pct` default 0.8, dedup 600s per (org, run, reason), incident dedup
//! keyed on the incident's own id) so behavior matches what the open stack
//! already defines.
//!
//! Reconcile-on-reconnect (D12.2b step 1): `CloudSse`'s public API does not
//! surface a discrete "just reconnected" event to its caller (the retry/
//! backoff loop is entirely internal, by design -- reused, not reimplemented,
//! per the W1 rules). This engine approximates the spec's intent with (a) one
//! reconcile at startup and (b) a periodic sweep (default 60s, matching risk
//! R6's own "belt-and-braces" framing in itrat-console/13 D12.6), which bounds
//! worst-case staleness after any gap -- reconnect or not -- rather than
//! trying to hook a reconnect signal `CloudSse` does not expose.

use genaryx_connectors::{
    AgentAgg, Alert, CloudClient, ConnectorError, Incident, RunAgg, Severity,
};
use genaryx_core::{EventSource, RawRecord};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Bound on the queue `GET /relay/v1/exceptions` returns: "bounded,
/// pre-computed" (docs/PHASE5.md). In practice alerts+incidents are already
/// far smaller than the 9k-run list the phone never pulls; this is a
/// defensive ceiling, not a normal-case limit.
const MAX_QUEUE_LEN: usize = 500;

/// Bound on the agent rollup `GET /relay/v1/money` returns, for the same
/// "bounded, pre-computed" reason `MAX_QUEUE_LEN` bounds the exception queue:
/// a real org's `/v1/agents` can run well past what a phone screen has any
/// use for, and the point of this whole surface is that the phone never has
/// to browse that list itself. `agents_truncated`/`others_spent_microusd`
/// account for whatever this cuts, so a cut fleet is never silent about it.
const MAX_AGENTS_ON_THE_WIRE: usize = 200;

/// How much burn-rate history to keep (10 minutes is enough for a
/// per-minute rate without growing unboundedly on a long-lived relay).
const BURN_WINDOW_SECS: i64 = 600;
const MAX_BURN_SAMPLES: usize = 64;

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---- wire / snapshot types --------------------------------------------------

/// The five-way taxonomy itrat-console/13 D12.2b names for the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExceptionClass {
    AtRisk,
    NearCap,
    OverCap,
    Runaway,
    /// No Cloud read in this crate's W1 reuse map (`CloudSse`/`CloudClient`/
    /// `genaryx-signing`) carries an approvals signal -- that is Wardryx's
    /// domain (D13 territory, not D12). Reserved so the wire shape is
    /// complete and stable now rather than a later breaking addition; wired
    /// the day a copilot/Wardryx read lands in the relay.
    #[allow(dead_code)]
    PendingApproval,
}

/// `HARD` (push immediately, deterministic, never suppressible) vs `SOFT`
/// (eligible for digesting/holding) per D12.2b's classification.
pub fn is_hard(class: ExceptionClass) -> bool {
    matches!(class, ExceptionClass::OverCap | ExceptionClass::Runaway)
}

/// One row in the phone-facing exception queue.
#[derive(Debug, Clone, Serialize)]
pub struct ExceptionItem {
    /// Stable key: `run:<run_id>` for budget/kill-derived items,
    /// `incident:<id>` for incident-derived items -- how repeated
    /// updates to the SAME exception collapse instead of duplicating.
    pub key: String,
    pub run_id: Option<String>,
    pub incident_id: Option<String>,
    /// Which agent this item belongs to, when something told us: an
    /// incident's own `agent_id`, or a `run_update` event's `RunAgg::agent_id`.
    /// `None` when nothing has -- `/v1/alerts` carries no agent info at all,
    /// so an alert-only item (no incident ever merged into it) genuinely has
    /// no signal to report, rather than a guessed one. Added so `GET
    /// /relay/v1/money` can join open exceptions back onto the agent rollup
    /// without a second, `/v1/runs`-shaped fetch just to learn this.
    pub agent_id: Option<String>,
    /// `"budget"` | `"kill"` | the incident kind (`budget_exhausted` |
    /// `sustained_loop` | `spend_spike` | `fanout_explosion`).
    pub kind: String,
    pub class: ExceptionClass,
    pub severity: Option<String>,
    /// Who detected this, when the money plane did not (`Incident::source`).
    /// Absent for everything TokenFuse measured itself, which is most of the
    /// queue: the phone shows it precisely because an operator about to kill a
    /// run should know whether the plane measured this or was told it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The reporting detector's own sentence, when it has one. Our thresholds
    /// need none; a shadow-tool or exfiltration finding is unreadable without
    /// it, since its kind alone says nothing an operator can act on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub headline: String,
    pub spent_microusd: i64,
    pub budget_micros: Option<i64>,
    pub fraction: Option<f64>,
    pub first_seen_unix: i64,
    pub last_seen_unix: i64,
    pub acknowledged: bool,
    pub killed: bool,
    /// C3 (docs/PHASE6-C3.md): a best-effort Felyx annotation, attached AFTER
    /// the deterministic push for a HARD event (or omitted). Enriches what the
    /// phone's poll shows; never gates the push. Omitted from the wire when
    /// absent so the phone's decoder (serde default) stays backward-compatible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copilot: Option<genaryx_copilot::CopilotAnnotation>,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Aggregates {
    pub spend_microusd: i64,
    pub headroom_microusd: i64,
    pub burn_rate_microusd_per_min: i64,
    pub updated_at_unix: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExceptionSnapshot {
    pub aggregates: Aggregates,
    /// What the operator can still act on: runs with a known budget position,
    /// plus detections of behaviour that is still happening. Killable.
    pub queue: Vec<ExceptionItem>,
    /// What already happened and was already contained, counted by kind rather
    /// than listed. See [`DigestRow`].
    #[serde(default)]
    pub digest: Vec<DigestRow>,
    /// How many actionable items did not fit in `queue`. Zero in every normal
    /// case. Present so a truncation is never silent on a governance surface:
    /// if this is nonzero the operator is being shown less than there is, and
    /// has to be told.
    #[serde(default)]
    pub queue_truncated: usize,
}

/// One rolled-up line of "this already happened, and the guardrail held".
///
/// ## Why this exists
///
/// Measured against a real fleet, the queue came back with 189 rows: 9 budget
/// alerts and 180 open incidents, of which about 150 were one `budget_exhausted`
/// per shard of a single fanned-out batch. That is not a pager, it is the fleet
/// browser this whole design exists to avoid, and it is unreadable on a 40mm
/// watch face.
///
/// The fix is not a smaller cap, it is noticing that the list was conflating
/// two different things. A run over its cap that is still spending is an
/// ACTION: you can kill it. A `budget_exhausted` incident is a REPORT that the
/// breaker already tripped and the spending already stopped. Both matter, but
/// only one of them is something to do, and 150 copies of "we stopped it" belong
/// on one line with a number, not on 150.
///
/// Nothing is hidden: the count is exact, and the kind is preserved, which is
/// the only place `fanout_explosion` is distinguished from `budget_exhausted`.
#[derive(Debug, Clone, Serialize)]
pub struct DigestRow {
    /// The incident kind, verbatim from the Cloud.
    pub kind: String,
    /// Which agent these belong to, when the Cloud told us. Grouping by
    /// (kind, agent) rather than kind alone is what turns an unreadable "171
    /// budget_exhausted" into "reconciliation-batch: 171 runs hit their
    /// ceiling", which is the sentence an operator actually needs. Costs
    /// nothing: `Incident` already carries `agent_id`.
    pub agent_id: Option<String>,
    /// Exactly how many were folded in. Never approximate, never capped.
    pub count: usize,
    /// The highest severity seen in the group.
    pub severity: Option<String>,
    /// The most recent occurrence in the group.
    pub last_seen_unix: i64,
}

/// `GET /relay/v1/money`: what a paired phone needs to see the money and the
/// agents without ever browsing the fleet -- a second bounded, pre-computed
/// read surface alongside [`ExceptionSnapshot`], not a doorway to
/// `/v1/agents` or `/v1/runs` (`proxy.rs`'s module docs: the read-proxy
/// allowlist stays at exactly one path, `/v1/summary`, for the identical
/// reason this stays pre-computed instead of a pass-through).
#[derive(Debug, Clone, Default, Serialize)]
pub struct MoneySnapshot {
    pub aggregates: Aggregates,
    pub savings: SavingsRollup,
    /// Highest spend first, capped at [`MAX_AGENTS_ON_THE_WIRE`]; see
    /// `agents_truncated`/`others_spent_microusd` for what did not fit.
    pub agents: Vec<AgentRollup>,
    /// How many agents did NOT fit in `agents`. Zero in the normal case;
    /// present so a cut fleet is never silent, the same rule
    /// `ExceptionSnapshot::queue_truncated` already applies to the queue.
    pub agents_truncated: usize,
    /// The combined spend of every agent `agents_truncated` cut, so
    /// `agents`'s own spend plus this always equals the fleet total: a phone
    /// shown only the top [`MAX_AGENTS_ON_THE_WIRE`] can still say the rest
    /// of the fleet exists, and how much it spent.
    pub others_spent_microusd: i64,
    /// Short fleet-level burn history for a sparkline, one point per
    /// reconcile, bounded the same way the underlying burn samples are.
    pub burn_series: Vec<BurnPoint>,
}

/// `GET /relay/v1/money`'s FinOps rollup: a field-for-field mirror of
/// Cloud's own `/v1/savings` shape (`genaryx_connectors::SavingsSummary`),
/// kept as this crate's own type rather than re-exporting the connectors DTO
/// directly so the relay's wire contract cannot silently change shape if the
/// connectors one ever does.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct SavingsRollup {
    pub blocked_spend_microusd: i64,
    pub cache_saved_microusd: i64,
    pub router_saved_microusd: i64,
    pub budget_breaks: u64,
    pub total_saved_microusd: i64,
}

/// One row of `GET /relay/v1/money`'s agent rollup: Cloud's own per-agent
/// totals (from `CloudClient::agents()`) plus what only the relay can add, a
/// rolling burn rate computed the same way the fleet aggregate already is,
/// and how many of this agent's own items are open on the exception queue
/// right now.
///
/// `open_exceptions`/`worst_severity` are joined by `agent_id` against the
/// live queue at snapshot time (see `money_snapshot`), not stored here: the
/// queue is already the one place that bookkeeping lives, and copying it into
/// a second place a reconcile might forget to update is exactly the kind of
/// drift `merge_incident`'s own doc describes paying for once already.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentRollup {
    /// `""` is the unattributed bucket -- Cloud's own `AgentAgg` convention,
    /// kept rather than renamed so the phone's decoder needs no special case.
    pub agent_id: String,
    pub spent_microusd: i64,
    pub calls: u64,
    pub runs: u64,
    pub burn_rate_microusd_per_min: i64,
    pub last_seen_unix: i64,
    pub open_exceptions: usize,
    pub worst_severity: Option<String>,
}

/// One point of a short burn-rate history, for a phone-side sparkline.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct BurnPoint {
    pub t_unix: i64,
    pub microusd_per_min: i64,
}

/// What the caller (main's event loop) should hand to an [`crate::push::ApnsSender`],
/// once it has resolved the current device's APNs token (the engine itself
/// does not know it -- that lives in the registry).
#[derive(Debug, Clone)]
pub struct PushIntent {
    pub title: String,
    pub body: String,
    pub run_id: Option<String>,
    pub incident_id: Option<String>,
    pub kind: String,
    /// HARD (D12.2b): would push unfiltered even with a D13 triage stage in
    /// front of it. No triage stage exists in W1, so today this only
    /// documents intent; every intent pushes.
    pub hard: bool,
}

// ---- pure classification (unit-tested directly) -----------------------------

/// An `/v1/alerts` fraction -> queue class. Cloud's own `/v1/alerts` already
/// pre-filters to `fraction >= alert_pct`, so in practice this only ever sees
/// "at least near-cap" input; it stays a total function over any fraction.
pub fn classify_fraction(fraction: f64) -> ExceptionClass {
    if fraction >= 1.0 {
        ExceptionClass::OverCap
    } else {
        ExceptionClass::NearCap
    }
}

/// An incident `kind`/`severity` -> queue class. `sustained_loop` and
/// `fanout_explosion` are always runaway (push.rs's own "running hot" copy);
/// `budget_exhausted` is a HARD over-cap event; anything else escalates to
/// runaway only at High/Critical severity, otherwise it is a SOFT at-risk
/// heads-up (`spend_spike` at Medium/Low, the common case).
pub fn classify_incident(kind: &str, severity: Severity) -> ExceptionClass {
    match kind {
        "sustained_loop" | "fanout_explosion" => ExceptionClass::Runaway,
        "budget_exhausted" => ExceptionClass::OverCap,
        _ if severity >= Severity::High => ExceptionClass::Runaway,
        _ => ExceptionClass::AtRisk,
    }
}

fn severity_str(s: Severity) -> &'static str {
    match s {
        Severity::Info => "info",
        Severity::Low => "low",
        Severity::Medium => "medium",
        Severity::High => "high",
        Severity::Critical => "critical",
    }
}

/// Port of `push.rs::PushPipeline::should_send`: at most one notification per
/// (scope, id, reason) per `window_secs`.
fn should_notify(
    dedup: &mut HashMap<(String, String, String), i64>,
    scope: &str,
    id: &str,
    reason: &str,
    now: i64,
    window_secs: i64,
) -> bool {
    let key = (scope.to_string(), id.to_string(), reason.to_string());
    if let Some(&last) = dedup.get(&key)
        && now - last < window_secs
    {
        return false;
    }
    dedup.insert(key, now);
    true
}

/// Port of `store.rs::mark_incident_notified` as `push.rs::incident_alert`
/// uses it: a dedicated per-incident clock (not the generic (org,run,reason)
/// map above), so a spammy incident's dedup never fights a spammy run's.
fn mark_incident_notified(
    tracker: &mut HashMap<String, i64>,
    incident_id: &str,
    now_millis: i64,
    window_ms: i64,
) -> bool {
    match tracker.get(incident_id) {
        Some(&last) if now_millis - last < window_ms => false,
        _ => {
            tracker.insert(incident_id.to_string(), now_millis);
            true
        }
    }
}

/// Sum of remaining headroom (`budget - spent`, floored at 0) across
/// currently-alerting runs -- the "headroom" aggregate.
fn headroom_from_alerts(alerts: &[Alert]) -> i64 {
    alerts
        .iter()
        .map(|a| (a.budget_micros - a.spent_microusd).max(0))
        .sum()
}

fn push_burn_sample(samples: &mut VecDeque<(i64, i64)>, now: i64, spend_microusd: i64) {
    // Cumulative spend can only go UP while the Cloud keeps running. If it
    // comes back lower, the Cloud restarted (its store is in-memory) and the
    // counter began again from zero, so every sample before this point belongs
    // to a different epoch and describes spending that is no longer being
    // counted. Keeping them produced a NEGATIVE burn rate on the phone and the
    // watch, which is not a smaller number, it is a nonsense one.
    //
    // Clamping the rate at zero would have hidden it; dropping the stale epoch
    // is the honest fix, and it means the rate simply rebuilds from the restart
    // rather than lying in either direction.
    if samples
        .back()
        .is_some_and(|&(_, last)| spend_microusd < last)
    {
        eprintln!(
            "genaryx-relay: burn: spend went backwards ({} -> {}), the Cloud restarted;              discarding {} sample(s) from the previous epoch",
            samples.back().map(|s| s.1).unwrap_or(0),
            spend_microusd,
            samples.len()
        );
        samples.clear();
    }
    samples.push_back((now, spend_microusd));
    let cutoff = now - BURN_WINDOW_SECS;
    while samples.len() > 1 && samples.front().is_some_and(|&(t, _)| t < cutoff) {
        samples.pop_front();
    }
    while samples.len() > MAX_BURN_SAMPLES {
        samples.pop_front();
    }
}

/// Microusd-per-minute burn rate from the oldest to the newest kept sample.
/// `0` with fewer than two samples, or if they land on the same second.
fn burn_rate_per_min(samples: &VecDeque<(i64, i64)>) -> i64 {
    let (Some(&(t0, s0)), Some(&(t1, s1))) = (samples.front(), samples.back()) else {
        return 0;
    };
    if t1 <= t0 {
        return 0;
    }
    let elapsed_secs = (t1 - t0) as f64;
    let delta = (s1 - s0) as f64;
    ((delta / elapsed_secs) * 60.0) as i64
}

// ---- the incoming stream shape (mirrors store.rs::StreamEvent's JSON) ------

/// Mirrors `tokenfuse-cloud::store::StreamEvent`'s wire shape exactly
/// (`#[serde(tag = "type", rename_all = "snake_case")]`, confirmed against
/// `store.rs:390-412`): `{"type":"run_update","run":{...}}`,
/// `{"type":"kill","run":"..."}`, `{"type":"budget","run":"...","budget_micros":...}`,
/// or an `Incident`'s own fields flattened in with `"type":"incident"`.
/// Reuses `genaryx_connectors`' own `RunAgg`/`Incident` DTOs (already proven
/// against the identical shapes for `/v1/runs`/`/v1/incidents`) rather than
/// re-modeling them.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RelayStreamEvent {
    RunUpdate { run: RunAgg },
    Kill { run: String },
    Budget { run: String, budget_micros: i64 },
    Incident(Incident),
}

// ---- engine ------------------------------------------------------------------

#[derive(Default)]
struct EngineState {
    aggregates: Aggregates,
    items: HashMap<String, ExceptionItem>,
    /// Already-contained incidents, rolled up by kind. Rebuilt wholesale by
    /// each reconcile: unlike `items` there is no per-entry lifecycle to
    /// preserve, a digest row is just a count of what the Cloud currently
    /// reports open.
    digest: Vec<DigestRow>,
    /// Live-updated from `Budget` stream events, seeded/corrected from
    /// `/v1/alerts` at each reconcile: the only way this relay learns a
    /// run's central-budget override without a `/v1/budgets` read (not one
    /// of `CloudClient`'s seven read methods).
    budgets: HashMap<String, i64>,
    /// Which agent a run belongs to, learned from the only two sources that
    /// carry it: `run_update` stream events and incidents. `/v1/alerts` does
    /// not, and alerts are what most of the queue is built from, so without
    /// this an over-cap run with no open incident reaches the phone with no
    /// agent on it and joins nothing in the agents list. Keyed by run like
    /// `budgets` above and kept the same way.
    run_agents: HashMap<String, String>,
    burn_samples: VecDeque<(i64, i64)>,
    push_dedup: HashMap<(String, String, String), i64>,
    incident_last_notified: HashMap<String, i64>,
    /// FinOps totals from `/v1/savings`, replaced wholesale each reconcile.
    /// See [`MoneySnapshot::savings`].
    savings: SavingsRollup,
    /// Per-agent cumulative figures + burn history from `/v1/agents`, keyed
    /// by `agent_id` (`""` is the unattributed bucket, same convention Cloud
    /// itself uses). Rebuilt by [`rebuild_agents`] each reconcile: an agent
    /// Cloud stops reporting is dropped rather than left to linger forever,
    /// the same rule `items` already follows for alerts.
    agents: HashMap<String, AgentState>,
    /// Fleet-level burn-rate HISTORY, one point per reconcile, for
    /// [`MoneySnapshot::burn_series`]'s sparkline. Distinct from
    /// `burn_samples` above: `burn_samples` holds raw cumulative-spend
    /// readings used to COMPUTE a rate; this holds the already-computed
    /// rates themselves, over time.
    burn_series: VecDeque<BurnPoint>,
}

/// Per-agent bookkeeping behind `EngineState::agents`: the cumulative
/// figures `/v1/agents` reports for one agent, plus that agent's OWN
/// burn-sample history. Kept separate from the fleet's `burn_samples`: an
/// agent's burn rate is that agent's own spend over time, not a slice of the
/// fleet total, so it needs its own window rather than sharing one.
#[derive(Default)]
struct AgentState {
    spent_microusd: i64,
    calls: u64,
    runs: u64,
    last_seen_unix: i64,
    burn_samples: VecDeque<(i64, i64)>,
}

/// Rebuild the per-agent map from a fresh `/v1/agents` read: each reported
/// agent's cumulative figures are replaced and a new burn sample pushed
/// (`push_burn_sample` reused verbatim, so the exact same Cloud-restarted
/// handling applies per agent as already applies fleet-wide); an agent this
/// read no longer reports is simply not carried into the result, the same
/// wholesale-replace rule `items` already follows for alerts each reconcile.
///
/// A free function over plain data (not a method on `EngineState`) so it can
/// be driven directly by a test with no live Cloud and no engine at all, the
/// same reason `alert_item`/`incident_item`/`merge_incident` are free
/// functions rather than being inlined into `reconcile`.
/// Give every queue item the agent it belongs to, then forget the runs that
/// no longer matter.
///
/// `/v1/alerts` states no agent, and alerts are what most of the queue is
/// rebuilt from, so without this an over-cap run with no open incident reaches
/// the phone anonymous and joins nothing in the agents list: the money axis
/// would be populated and the behaviour axis empty for almost every agent.
/// Only items that state no agent of their own are touched, so an incident's
/// own attribution always wins over a remembered one.
///
/// The map is then pruned, because every run the stream mentions teaches it
/// something and it would otherwise grow with the fleet forever. A run worth
/// remembering is one the queue is showing or one carrying a budget, which is
/// the same pair of sets the rest of this engine already bounds itself by.
///
/// Free function over plain data for the same reason as [`rebuild_agents`]:
/// a test can drive it with no live Cloud and no engine at all.
fn attach_known_agents(
    items: &mut HashMap<String, ExceptionItem>,
    run_agents: &mut HashMap<String, String>,
    budgets: &HashMap<String, i64>,
) {
    for item in items.values_mut() {
        if item.agent_id.is_none()
            && let Some(run) = item.run_id.as_ref()
        {
            item.agent_id = run_agents.get(run).cloned();
        }
    }
    run_agents.retain(|run, _| {
        budgets.contains_key(run)
            || items
                .values()
                .any(|i| i.run_id.as_deref() == Some(run.as_str()))
    });
}

fn rebuild_agents(
    existing: &mut HashMap<String, AgentState>,
    agents: &[AgentAgg],
    now: i64,
) -> HashMap<String, AgentState> {
    let mut next = HashMap::with_capacity(agents.len());
    for a in agents {
        let mut agent_state = existing.remove(&a.agent_id).unwrap_or_default();
        agent_state.spent_microusd = a.spent_microusd;
        agent_state.calls = a.calls;
        agent_state.runs = a.runs;
        agent_state.last_seen_unix = a.last_seen_millis / 1000;
        push_burn_sample(&mut agent_state.burn_samples, now, a.spent_microusd);
        next.insert(a.agent_id.clone(), agent_state);
    }
    next
}

/// Append one fleet-level burn point and drop whatever falls outside the
/// same window/count `push_burn_sample` enforces on the underlying spend
/// samples -- a separate, smaller cap here, not a shared one, because a
/// `BurnPoint` is already a computed rate rather than a cumulative counter,
/// so there is no "Cloud restarted" case to detect the way `push_burn_sample`
/// has to.
fn push_burn_point(series: &mut VecDeque<BurnPoint>, point: BurnPoint) {
    series.push_back(point);
    let cutoff = point.t_unix - BURN_WINDOW_SECS;
    while series.len() > 1 && series.front().is_some_and(|p| p.t_unix < cutoff) {
        series.pop_front();
    }
    while series.len() > MAX_BURN_SAMPLES {
        series.pop_front();
    }
}

/// The phone-facing exception state, fed by `CloudSse` + periodic
/// `CloudClient` reads. See the module docs for the full design.
pub struct ExceptionEngine {
    org: String,
    alert_pct: f64,
    dedup_secs: i64,
    state: Mutex<EngineState>,
}

impl ExceptionEngine {
    pub fn new(org: impl Into<String>, alert_pct: f64, dedup_secs: i64) -> Self {
        Self {
            org: org.into(),
            alert_pct,
            dedup_secs,
            state: Mutex::new(EngineState::default()),
        }
    }

    /// Every tracked item, unfiltered, for tests that are about the engine's
    /// bookkeeping (classification, dedup) rather than about what a pager
    /// chooses to display. Production code goes through `snapshot`.
    #[cfg(test)]
    fn tracked_items(&self) -> Vec<ExceptionItem> {
        let st = self.state.lock().expect("exception engine mutex poisoned");
        st.items.values().cloned().collect()
    }

    /// The current bounded, pre-computed snapshot `GET /relay/v1/exceptions`
    /// serves. HARD items sort first, then most-recently-seen first.
    pub fn snapshot(&self) -> ExceptionSnapshot {
        let st = self.state.lock().expect("exception engine mutex poisoned");
        // The pager shows ONLY what the operator can still act on, and only
        // what has actually crossed a line. Everything filtered here is still
        // fully visible in Genaryx on the desktop, which reads the Cloud
        // directly and never sees this queue: the reduction is the wrist's and
        // the pocket's, not the fleet record's.
        let mut queue: Vec<ExceptionItem> = st
            .items
            .values()
            .filter(|i| !i.killed && shows_on_a_pager(i.class))
            .cloned()
            .collect();
        queue.sort_by(queue_order);
        // Report what we cut rather than quietly serving a shorter list. A
        // governance surface that silently truncates is telling the operator
        // "this is everything" when it is not.
        let queue_truncated = queue.len().saturating_sub(MAX_QUEUE_LEN);
        queue.truncate(MAX_QUEUE_LEN);
        ExceptionSnapshot {
            aggregates: st.aggregates,
            queue,
            digest: st.digest.clone(),
            queue_truncated,
        }
    }

    /// The current bounded, pre-computed snapshot `GET /relay/v1/money`
    /// serves: what a paired phone needs to see the money and the agents
    /// without ever browsing the fleet (see the module docs' proxy-allowlist
    /// rule this stays inside of). Agents sort by spend descending, tie-broken
    /// by `agent_id` ascending so two agents with identical spend still come
    /// back in the same order on every poll, rather than however `HashMap`
    /// iteration happened to land this time.
    pub fn money_snapshot(&self) -> MoneySnapshot {
        let st = self.state.lock().expect("exception engine mutex poisoned");

        let mut agents: Vec<AgentRollup> = st
            .agents
            .iter()
            .map(|(agent_id, agent)| {
                let (open_exceptions, worst_severity) =
                    agent_exception_summary(&st.items, agent_id);
                AgentRollup {
                    agent_id: agent_id.clone(),
                    spent_microusd: agent.spent_microusd,
                    calls: agent.calls,
                    runs: agent.runs,
                    burn_rate_microusd_per_min: burn_rate_per_min(&agent.burn_samples),
                    last_seen_unix: agent.last_seen_unix,
                    open_exceptions,
                    worst_severity,
                }
            })
            .collect();
        agents.sort_by(|a, b| {
            b.spent_microusd
                .cmp(&a.spent_microusd)
                .then(a.agent_id.cmp(&b.agent_id))
        });

        // Same "never silent" rule `queue_truncated` already applies to the
        // exception queue: a phone shown only the top MAX_AGENTS_ON_THE_WIRE
        // must still be told the rest of the fleet exists, and how much it
        // spent, not just that some was cut.
        let agents_truncated = agents.len().saturating_sub(MAX_AGENTS_ON_THE_WIRE);
        let others_spent_microusd: i64 = agents
            .iter()
            .skip(MAX_AGENTS_ON_THE_WIRE)
            .map(|a| a.spent_microusd)
            .sum();
        agents.truncate(MAX_AGENTS_ON_THE_WIRE);

        MoneySnapshot {
            aggregates: st.aggregates,
            savings: st.savings,
            agents,
            agents_truncated,
            others_spent_microusd,
            burn_series: st.burn_series.iter().copied().collect(),
        }
    }

    /// C3 (docs/PHASE6-C3.md): attach a Felyx annotation to a queued item, if it
    /// is still present. The triage stage's spawned, budgeted task calls this
    /// AFTER the deterministic HARD push has already gone out, so it only ever
    /// ENRICHES what the phone's next poll shows and can never gate the push.
    /// Returns whether an item was found (it may have been reconciled away
    /// between the push and the annotation completing).
    pub fn annotate_item(&self, key: &str, annotation: genaryx_copilot::CopilotAnnotation) -> bool {
        let mut st = self.state.lock().expect("exception engine mutex poisoned");
        if let Some(item) = st.items.get_mut(key) {
            item.copilot = Some(annotation);
            true
        } else {
            false
        }
    }

    /// Test-only: seed a queue item directly (the triage/annotation tests need a
    /// known item without replaying a full SSE record).
    #[cfg(test)]
    pub(crate) fn seed_item_for_test(&self, item: ExceptionItem) {
        let mut st = self.state.lock().expect("exception engine mutex poisoned");
        st.items.insert(item.key.clone(), item);
    }

    /// Test-only: seed one agent's bookkeeping directly (the money-snapshot
    /// tests need known agent figures and burn history without a live
    /// Cloud's `/v1/agents` to reconcile against, the same reason
    /// `seed_item_for_test` exists for the exception queue). Private rather
    /// than `pub(crate)` like its sibling: unlike `seed_item_for_test`
    /// (called from `triage.rs`'s tests too), only this module's own tests
    /// use this one, and `AgentState` itself is private to this module.
    #[cfg(test)]
    fn seed_agent_for_test(&self, agent_id: &str, agent: AgentState) {
        let mut st = self.state.lock().expect("exception engine mutex poisoned");
        st.agents.insert(agent_id.to_string(), agent);
    }

    /// Full resync against the Cloud's own authoritative reads: `/v1/summary`
    /// (aggregate spend), `/v1/alerts` (near/over-cap runs + their budgets),
    /// `/v1/incidents` (open, unacknowledged incidents), `/v1/agents`
    /// (per-agent spend rollup) and `/v1/savings` (FinOps totals) -- the last
    /// two feed `money_snapshot` exactly the way the first three feed
    /// `snapshot`. Replaces the queue wholesale but preserves each surviving
    /// item's `first_seen_unix` (the one thing Cloud doesn't hand back for
    /// alerts) rather than resetting it every sweep.
    pub async fn reconcile(&self, cloud: &CloudClient) -> Result<(), ConnectorError> {
        let summary = cloud.summary().await?;
        let alerts = cloud.alerts().await?;
        let incidents = cloud.incidents().await?;
        let agents = cloud.agents().await?;
        let savings = cloud.savings().await?;
        let now = now_unix();

        let mut st = self.state.lock().expect("exception engine mutex poisoned");
        let mut items = HashMap::with_capacity(alerts.len() + incidents.len());

        for a in &alerts {
            let key = format!("run:{}", a.run_id);
            let first_seen = st.items.get(&key).map_or(now, |old| old.first_seen_unix);
            items.insert(key.clone(), alert_item(&key, a, first_seen, now));
            st.budgets.insert(a.run_id.clone(), a.budget_micros);
        }

        // Incidents fall into three cases, and getting them into ONE list keyed
        // by run is what stops the same run appearing twice with different
        // numbers (measured: the protagonist run showed once with its real
        // $6.91/$5.57 and again with $0.00/$0.00, because alerts keyed on
        // `run:` and incidents on `incident:`).
        let mut digest: HashMap<String, DigestRow> = HashMap::new();
        for inc in incidents.iter().filter(|i| !i.acknowledged) {
            // An incident is one of only two places a run's agent is ever
            // stated, so learn it here whether or not this incident earns a
            // row of its own.
            if let (Some(run), Some(agent)) = (inc.run_id.as_ref(), inc.agent_id.as_ref()) {
                st.run_agents.insert(run.clone(), agent.clone());
            }
            match inc.run_id.as_ref().map(|r| format!("run:{r}")) {
                // 1. About a run we are already showing: MERGE, never add a
                //    row. The alert owns the money, the incident owns the kind.
                Some(key) if items.contains_key(&key) => {
                    if let Some(item) = items.get_mut(&key) {
                        merge_incident(item, inc);
                    }
                }
                // 2. Already contained: the breaker tripped and the spending
                //    stopped, so this is a report, not a task. Counted.
                _ if incident_is_already_contained(&inc.kind) => {
                    fold_into_digest(&mut digest, inc);
                }
                // 3. Still happening, and not attached to a run we are already
                //    showing: it earns its own row.
                _ => {
                    let key = format!("incident:{}", inc.id);
                    items.insert(key.clone(), incident_item(&key, inc));
                }
            }
        }

        st.items = items;
        // Alerts carry no agent, so most of the queue arrives anonymous and
        // would join nothing in the agents list. Anything already known about
        // the run, from a stream update or an incident, is filled in here.
        {
            let EngineState {
                items,
                run_agents,
                budgets,
                ..
            } = &mut *st;
            attach_known_agents(items, run_agents, budgets);
        }
        let mut digest: Vec<DigestRow> = digest.into_values().collect();
        digest.sort_by(|a, b| b.count.cmp(&a.count).then(a.kind.cmp(&b.kind)));
        st.digest = digest;

        let next_agents = rebuild_agents(&mut st.agents, &agents, now);
        st.agents = next_agents;
        st.savings = SavingsRollup {
            blocked_spend_microusd: savings.blocked_spend_microusd,
            cache_saved_microusd: savings.cache_saved_microusd,
            router_saved_microusd: savings.router_saved_microusd,
            budget_breaks: savings.budget_breaks,
            total_saved_microusd: savings.total_saved_microusd,
        };

        push_burn_sample(&mut st.burn_samples, now, summary.spent_microusd);
        let fleet_burn_rate = burn_rate_per_min(&st.burn_samples);
        st.aggregates = Aggregates {
            spend_microusd: summary.spent_microusd,
            headroom_microusd: headroom_from_alerts(&alerts),
            burn_rate_microusd_per_min: fleet_burn_rate,
            updated_at_unix: now,
        };
        // One fleet-level burn point per reconcile, from the rate just
        // computed above -- see `EngineState::burn_series`'s doc for how this
        // differs from `burn_samples`.
        push_burn_point(
            &mut st.burn_series,
            BurnPoint {
                t_unix: now,
                microusd_per_min: fleet_burn_rate,
            },
        );
        Ok(())
    }

    /// Apply one decoded `/v1/stream` record (a [`RawRecord::raw`] JSON
    /// string). Returns a push intent iff this update should notify per the
    /// ported dedup rules; a record this engine doesn't recognize (schema
    /// drift, or a future event kind) is ignored rather than failing the
    /// whole poll loop (fail-closed on TRUST, not on best-effort parsing --
    /// `reconcile`'s periodic sweep remains the correctness backstop).
    pub fn handle_raw_record(&self, raw: &RawRecord, now: i64) -> Option<PushIntent> {
        let event: RelayStreamEvent = serde_json::from_str(&raw.raw).ok()?;
        let mut st = self.state.lock().expect("exception engine mutex poisoned");
        match event {
            RelayStreamEvent::RunUpdate { run } => self.handle_run_update(&mut st, run, now),
            RelayStreamEvent::Kill { run } => self.handle_kill(&mut st, run, now),
            RelayStreamEvent::Budget { run, budget_micros } => {
                st.budgets.insert(run, budget_micros);
                None
            }
            RelayStreamEvent::Incident(inc) => self.handle_incident(&mut st, inc, now),
        }
    }

    fn handle_run_update(&self, st: &mut EngineState, run: RunAgg, now: i64) -> Option<PushIntent> {
        // Learned before any of the gates below, because a run that is calm now
        // is exactly the one whose agent we will want if it goes over its cap
        // later and arrives as an alert, which carries no agent at all.
        st.run_agents
            .insert(run.run_id.clone(), run.agent_id.clone());
        let budget = *st.budgets.get(&run.run_id)?;
        if budget <= 0 {
            return None;
        }
        let fraction = run.spent_microusd as f64 / budget as f64;
        if fraction < self.alert_pct {
            return None;
        }
        let key = format!("run:{}", run.run_id);
        let class = classify_fraction(fraction);
        let first_seen = st.items.get(&key).map_or(now, |old| old.first_seen_unix);
        let was_killed = st.items.get(&key).is_some_and(|old| old.killed);
        st.items.insert(
            key.clone(),
            ExceptionItem {
                key,
                run_id: Some(run.run_id.clone()),
                incident_id: None,
                // `RunAgg::agent_id` is a plain String ("" = unattributed,
                // but still KNOWN), unlike the total absence of agent info
                // `/v1/alerts` carries -- keep it as-is, never collapsed to
                // `None`, so it still joins the unattributed bucket correctly.
                agent_id: Some(run.agent_id.clone()),
                kind: "budget".to_string(),
                class,
                severity: None,
                source: None,
                summary: None,
                headline: format!("Run {} at {:.0}% of budget", run.run_id, fraction * 100.0),
                spent_microusd: run.spent_microusd,
                budget_micros: Some(budget),
                fraction: Some(fraction),
                first_seen_unix: first_seen,
                last_seen_unix: now,
                acknowledged: false,
                killed: was_killed || run.killed,
                copilot: None,
            },
        );
        if !should_notify(
            &mut st.push_dedup,
            &self.org,
            &run.run_id,
            "budget",
            now,
            self.dedup_secs,
        ) {
            return None;
        }
        Some(PushIntent {
            title: "Budget alert".to_string(),
            body: format!("Run {} at {:.0}% of budget", run.run_id, fraction * 100.0),
            run_id: Some(run.run_id),
            incident_id: None,
            kind: "budget".to_string(),
            hard: is_hard(class),
        })
    }

    fn handle_kill(&self, st: &mut EngineState, run: String, now: i64) -> Option<PushIntent> {
        let key = format!("run:{run}");
        match st.items.get_mut(&key) {
            Some(item) => {
                item.killed = true;
                item.last_seen_unix = now;
            }
            None => {
                // A kill with no prior tracked alert (e.g. a desktop-issued
                // kill on a run that never crossed alert_pct) is still worth
                // surfacing so the phone's queue reflects it.
                st.items.insert(
                    key.clone(),
                    ExceptionItem {
                        key,
                        run_id: Some(run.clone()),
                        incident_id: None,
                        agent_id: None, // a bare kill event carries no agent info
                        kind: "kill".to_string(),
                        class: ExceptionClass::OverCap,
                        severity: None,
                        source: None,
                        summary: None,
                        headline: format!("Agent run {run} was killed"),
                        spent_microusd: 0,
                        budget_micros: None,
                        fraction: None,
                        first_seen_unix: now,
                        last_seen_unix: now,
                        acknowledged: false,
                        killed: true,
                        copilot: None,
                    },
                );
            }
        }
        if !should_notify(
            &mut st.push_dedup,
            &self.org,
            &run,
            "kill",
            now,
            self.dedup_secs,
        ) {
            return None;
        }
        Some(PushIntent {
            title: "Run killed".to_string(),
            body: format!("Agent run {run} was killed"),
            run_id: Some(run),
            incident_id: None,
            kind: "kill".to_string(),
            hard: true,
        })
    }

    fn handle_incident(&self, st: &mut EngineState, inc: Incident, now: i64) -> Option<PushIntent> {
        let key = format!("incident:{}", inc.id);
        if inc.acknowledged {
            st.items.remove(&key);
            return None;
        }
        let class = classify_incident(&inc.kind, inc.severity);
        // The subject is the run when there is one, otherwise the AGENT. Never the
        // incident id: for a runaway that id is literally
        // `fanout_explosion:agent://meridian.example/kyc-aml/sanctions-screener`,
        // which produced the headline "Agent/run fanout_explosion:agent://... running
        // hot - fanout_explosion", naming the kind twice around a raw URI.
        st.items.insert(key.clone(), incident_item(&key, &inc));

        let now_ms = now * 1000;
        let window_ms = self.dedup_secs * 1000;
        if !mark_incident_notified(&mut st.incident_last_notified, &inc.id, now_ms, window_ms) {
            return None;
        }
        Some(PushIntent {
            title: "Agent running hot".to_string(),
            body: format!(
                "{} on {}. Tap to review and kill.",
                humanise_kind(&inc.kind),
                inc.run_id
                    .clone()
                    .or_else(|| inc.agent_id.as_deref().map(short_agent))
                    .unwrap_or_else(|| "the fleet".to_string())
            ),
            run_id: inc.run_id,
            incident_id: Some(inc.id),
            kind: inc.kind,
            hard: is_hard(class),
        })
    }
}

/// Does this class belong on a wrist or in a pocket at all?
///
/// Only things that have crossed a line: over the cap, a detection of something
/// still running away, or past the 80% alert threshold. Anything below that is
/// ordinary operation, and the operator was explicit that it does not interest
/// them on a pager. `PendingApproval` is governance queueing, not a burning
/// budget, so it is desktop work.
///
/// This is a display filter, never a data filter: Genaryx reads the Cloud
/// directly and still shows every run, every incident and every kill.
fn shows_on_a_pager(class: ExceptionClass) -> bool {
    matches!(
        class,
        ExceptionClass::OverCap | ExceptionClass::Runaway | ExceptionClass::NearCap
    )
}

/// The one definition of queue order, owned by the SERVER so the phone and the
/// watch cannot disagree. They used to: this sorted hard-class-first here while
/// the watch re-sorted by its own rule, so one fleet read two different ways on
/// two surfaces of the same product.
///
/// 1. Killed items are filtered out upstream and never reach here; the clause
///    is kept as a backstop so a future caller that skips the filter still
///    cannot put history above work.
/// 2. Then urgency class, most urgent first: over the limit, then a detection
///    of something still running, then approaching the limit.
/// 3. Then how far past the line, worst first.
/// 4. Then most recently seen.
fn queue_order(a: &ExceptionItem, b: &ExceptionItem) -> std::cmp::Ordering {
    a.killed
        .cmp(&b.killed)
        .then(class_rank(a.class).cmp(&class_rank(b.class)))
        .then(
            b.fraction
                .unwrap_or(-1.0)
                .partial_cmp(&a.fraction.unwrap_or(-1.0))
                .unwrap_or(std::cmp::Ordering::Equal),
        )
        .then(b.last_seen_unix.cmp(&a.last_seen_unix))
}

/// Urgency order for the queue, lowest first. Explicit rather than derived from
/// [`is_hard`] because the operator's rule is about the LIMIT, not about the
/// hard/soft push distinction: something over its cap outranks something merely
/// detected, which outranks something approaching its cap.
fn class_rank(class: ExceptionClass) -> u8 {
    match class {
        ExceptionClass::OverCap => 0,
        ExceptionClass::Runaway => 1,
        ExceptionClass::NearCap => 2,
        ExceptionClass::AtRisk => 3,
        ExceptionClass::PendingApproval => 4,
    }
}

fn alert_item(key: &str, a: &Alert, first_seen_unix: i64, now: i64) -> ExceptionItem {
    let class = classify_fraction(a.fraction);
    ExceptionItem {
        key: key.to_string(),
        run_id: Some(a.run_id.clone()),
        incident_id: None,
        agent_id: None, // `Alert` (`/v1/alerts`) carries no agent info at all
        kind: "budget".to_string(),
        class,
        severity: None,
        source: None,
        summary: None,
        // Deliberately does NOT name the run: every row already renders
        // `run_id` on its own line, and repeating it here cost three wrapped
        // lines on the phone and left nothing but "reconcil..." on the wrist.
        headline: format!("At {:.0}% of its budget", a.fraction * 100.0),
        spent_microusd: a.spent_microusd,
        budget_micros: Some(a.budget_micros),
        fraction: Some(a.fraction),
        first_seen_unix,
        last_seen_unix: now,
        acknowledged: false,
        killed: a.killed,
        copilot: None,
    }
}

/// Is this incident kind a report that the guardrail ALREADY stopped the
/// spending, rather than a detection of something still going on?
///
/// `budget_exhausted` is the breaker tripping: by the time we see it, the call
/// was refused and nothing more is being spent on that run. There is no action
/// left to take, so 150 of them belong on one counted line.
///
/// The others are live signals. `sustained_loop` means it is still looping,
/// `spend_spike` that it is still spiking, `fanout_explosion` that the fan-out
/// is still widening. Those keep their own row, because the operator may still
/// want to reach for the kill.
///
/// Unknown kinds are treated as NOT contained, deliberately: a kind this build
/// has never heard of gets a visible row rather than being quietly folded into
/// a number.
fn incident_is_already_contained(kind: &str) -> bool {
    matches!(kind, "budget_exhausted")
}

/// Fold an incident into its kind's digest row, keeping the exact count, the
/// highest severity seen and the most recent occurrence.
fn fold_into_digest(digest: &mut HashMap<String, DigestRow>, inc: &Incident) {
    let last_seen = inc.last_seen_millis / 1000;
    let sev = severity_str(inc.severity).to_string();
    let group = format!("{}|{}", inc.kind, inc.agent_id.as_deref().unwrap_or(""));
    digest
        .entry(group)
        .and_modify(|row| {
            row.count += 1;
            row.last_seen_unix = row.last_seen_unix.max(last_seen);
            if severity_rank(&sev) > row.severity.as_deref().map_or(0, severity_rank) {
                row.severity = Some(sev.clone());
            }
        })
        .or_insert_with(|| DigestRow {
            kind: inc.kind.clone(),
            agent_id: inc.agent_id.clone(),
            count: 1,
            severity: Some(sev),
            last_seen_unix: last_seen,
        });
}

/// Ordering over the severity spellings, so "highest seen" is well defined.
fn severity_rank(s: &str) -> u8 {
    match s {
        "critical" => 4,
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

/// `open_exceptions`/`worst_severity` for one agent, joined against the SAME
/// pager-visible slice `snapshot()` serves as `queue` (`!killed &&
/// shows_on_a_pager`, not the raw tracked-items map) -- so a phone drilling
/// from the money view into one agent sees a count consistent with what the
/// pager itself would show for that agent, not a second, differently-filtered
/// number. Ranks severity with `severity_rank`, the one ordering this file
/// already defines for "highest seen" (`fold_into_digest` uses the same
/// function), rather than inventing a second one.
fn agent_exception_summary(
    items: &HashMap<String, ExceptionItem>,
    agent_id: &str,
) -> (usize, Option<String>) {
    let mut count = 0usize;
    let mut worst: Option<String> = None;
    for item in items.values() {
        if item.killed || !shows_on_a_pager(item.class) {
            continue;
        }
        if item.agent_id.as_deref() != Some(agent_id) {
            continue;
        }
        count += 1;
        if let Some(sev) = &item.severity
            && severity_rank(sev) > worst.as_deref().map_or(0, severity_rank)
        {
            worst = Some(sev.clone());
        }
    }
    (count, worst)
}

/// Fold an incident INTO the run item an alert already produced. The alert is
/// authoritative for money (it is the only source with spend and budget); the
/// incident contributes what the alert cannot know: that there is an open
/// incident, its kind, its severity, and how recently it fired.
///
/// The class is widened, never narrowed: an incident that classifies harder
/// than the budget fraction did wins, so a run that is only at 84% of budget
/// but is in a fan-out explosion does not read as a mild "near cap".
fn merge_incident(item: &mut ExceptionItem, inc: &Incident) {
    item.incident_id = Some(inc.id.clone());
    item.agent_id = inc.agent_id.clone();
    item.kind = inc.kind.clone();
    item.severity = Some(severity_str(inc.severity).to_string());
    // A merged row takes the incident's provenance too. Without this, an
    // over-cap run that an external detector ALSO flagged would reach the
    // phone looking like something this plane measured end to end.
    item.source = inc.source.clone();
    item.summary = inc.summary.clone();
    item.last_seen_unix = item.last_seen_unix.max(inc.last_seen_millis / 1000);
    item.first_seen_unix = item.first_seen_unix.min(inc.first_seen_millis / 1000);
    let incident_class = classify_incident(&inc.kind, inc.severity);
    if is_hard(incident_class) && !is_hard(item.class) {
        item.class = incident_class;
    }
    if let Some(fraction) = item.fraction {
        item.headline = format!(
            "At {:.0}% of its budget - {}",
            fraction * 100.0,
            humanise_kind(&inc.kind).to_lowercase()
        );
    }
}

/// The readable tail of an agent URI: `agent://meridian.example/kyc-aml/
/// sanctions-screener` becomes `sanctions-screener`. What a person calls it.
fn short_agent(agent_id: &str) -> String {
    agent_id.rsplit('/').next().unwrap_or(agent_id).to_string()
}

/// `budget_exhausted` becomes `Budget exhausted`. The wire keeps snake_case,
/// which is right for a protocol and wrong for a sentence a human reads on a
/// watch face.
fn humanise_kind(kind: &str) -> String {
    match kind {
        "budget_exhausted" => "Budget exhausted".to_string(),
        "sustained_loop" => "Sustained loop".to_string(),
        "spend_spike" => "Spend spike".to_string(),
        "fanout_explosion" => "Fan-out explosion".to_string(),
        other => {
            let mut words = other.split('_');
            match words.next() {
                Some(first) if !first.is_empty() => {
                    let mut s = first[..1].to_uppercase() + &first[1..];
                    for w in words {
                        s.push(' ');
                        s.push_str(w);
                    }
                    s
                }
                _ => other.to_string(),
            }
        }
    }
}

fn incident_item(key: &str, inc: &Incident) -> ExceptionItem {
    let class = classify_incident(&inc.kind, inc.severity);
    // The subject is the run when there is one, otherwise the AGENT. Never the
    // incident id: for a runaway that id is literally
    // `fanout_explosion:agent://meridian.example/kyc-aml/sanctions-screener`,
    // which produced the headline "Agent/run fanout_explosion:agent://...
    // running hot - fanout_explosion", naming the kind twice around a raw URI.
    let subject = inc
        .run_id
        .clone()
        .or_else(|| inc.agent_id.as_deref().map(short_agent))
        .unwrap_or_else(|| "the fleet".to_string());
    ExceptionItem {
        key: key.to_string(),
        run_id: inc.run_id.clone(),
        incident_id: Some(inc.id.clone()),
        agent_id: inc.agent_id.clone(),
        kind: inc.kind.clone(),
        class,
        severity: Some(severity_str(inc.severity).to_string()),
        source: inc.source.clone(),
        summary: inc.summary.clone(),
        headline: format!("{} on {subject}", humanise_kind(&inc.kind)),
        spent_microusd: 0,
        budget_micros: None,
        fraction: None,
        first_seen_unix: inc.first_seen_millis / 1000,
        last_seen_unix: inc.last_seen_millis / 1000,
        acknowledged: inc.acknowledged,
        killed: false,
        copilot: None,
    }
}

// ---- GET /relay/v1/exceptions (public, authenticated) -----------------------

/// `GET /relay/v1/exceptions`: the phone's authoritative queue, bounded and
/// pre-computed (D12.2b step 5: "the relay serves a bounded, pre-computed
/// queue; empty queue = calm = normal"). Authenticated against the
/// registry's own stored device token (constant-time), since -- unlike the
/// proxied `/v1/summary` read -- this data never round-trips the Cloud per
/// request, so the relay itself is the only thing that CAN authenticate it.
pub async fn exceptions_handler(
    axum::extract::State(state): axum::extract::State<crate::PublicState>,
    headers: axum::http::HeaderMap,
) -> Result<axum::Json<ExceptionSnapshot>, crate::proxy::ProxyError> {
    let token =
        crate::proxy::bearer_token(&headers).ok_or(crate::proxy::ProxyError::Unauthorized)?;
    let device = state
        .registry
        .verify_bearer(token)
        .map_err(|e| crate::proxy::ProxyError::Internal(e.to_string()))?
        .ok_or(crate::proxy::ProxyError::Unauthorized)?;
    if let Err(e) = state
        .registry
        .touch_last_seen(&device.device_id, now_unix())
    {
        eprintln!("genaryx-relay: exceptions: touch_last_seen failed (non-fatal): {e}");
    }
    Ok(axum::Json(state.engine.snapshot()))
}

// ---- GET /relay/v1/money (public, authenticated) ----------------------------

/// `GET /relay/v1/money`: the phone's money + agents view, bounded and
/// pre-computed for the same reason [`exceptions_handler`] is (see the module
/// docs, and `proxy.rs`'s own module docs for the read-proxy side of the
/// identical rule): a second allowlisted read surface, never a doorway to
/// `/v1/agents` or `/v1/runs`. Same auth shape as `exceptions_handler`: bearer
/// against the registry's own stored device token, since this data never
/// round-trips the Cloud per request either.
pub async fn money_handler(
    axum::extract::State(state): axum::extract::State<crate::PublicState>,
    headers: axum::http::HeaderMap,
) -> Result<axum::Json<MoneySnapshot>, crate::proxy::ProxyError> {
    let token =
        crate::proxy::bearer_token(&headers).ok_or(crate::proxy::ProxyError::Unauthorized)?;
    let device = state
        .registry
        .verify_bearer(token)
        .map_err(|e| crate::proxy::ProxyError::Internal(e.to_string()))?
        .ok_or(crate::proxy::ProxyError::Unauthorized)?;
    if let Err(e) = state
        .registry
        .touch_last_seen(&device.device_id, now_unix())
    {
        eprintln!("genaryx-relay: money: touch_last_seen failed (non-fatal): {e}");
    }
    Ok(axum::Json(state.engine.money_snapshot()))
}

// ---- background drivers (wired from main.rs) --------------------------------

/// Poll `sse` on a short tick and apply every decoded record to `engine`,
/// forwarding any resulting [`PushIntent`] to `push` (attaching the current
/// device's APNs token from `registry`, if any -- the engine itself never
/// sees the registry). Runs until the process exits; `CloudSse` was
/// constructed with `max_attempts: None` (see `main.rs`), so in normal
/// operation `poll()` only ever returns `Ok`, `Err` means the background
/// loop was told to stop.
pub async fn run_event_loop(
    triage: std::sync::Arc<crate::triage::Triage>,
    mut sse: genaryx_connectors::CloudSse,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
    let mut last_soft_flush = now_unix();
    loop {
        interval.tick().await;
        match sse.poll() {
            Ok(records) => {
                for record in &records {
                    let now = now_unix();
                    if let Some(intent) = triage.engine().handle_raw_record(record, now) {
                        // C3: the triage stage decides HARD (push now, annotate
                        // best-effort) vs SOFT (hold for the digest). In W1 this
                        // was a direct dispatch_push; the deterministic floor is
                        // preserved inside Triage::on_intent.
                        triage.on_intent(intent);
                    }
                }
            }
            Err(e) => {
                eprintln!("genaryx-relay: exceptions: sse stream ended: {e}");
                return;
            }
        }
        // C3: flush the soft-event digest on its own cadence (batch / hold).
        let now = now_unix();
        if now - last_soft_flush >= triage.soft_flush_secs() {
            triage.flush_soft();
            last_soft_flush = now;
        }
    }
}

pub(crate) fn dispatch_push(
    registry: &crate::registry::Registry,
    push: &dyn crate::push::ApnsSender,
    intent: PushIntent,
) {
    // Fan out to every paired surface that has registered for push. Phone and
    // watch each hold their own APNs token, so an exception that matters
    // reaches both; a device that has not registered is skipped rather than
    // holding the others back.
    let tokens: Vec<String> = match registry.devices() {
        Ok(devices) => devices.into_iter().filter_map(|d| d.apns_token).collect(),
        Err(e) => {
            eprintln!("genaryx-relay: exceptions: registry read failed, dropping push: {e}");
            return;
        }
    };
    if tokens.is_empty() {
        // Sim phase: no APNs registration path is wired (docs/PHASE5.md
        // defers real APNs to R1); the devices poll `/relay/v1/exceptions`
        // instead, so a push is a nice-to-have wake, never the source of
        // truth. Still log the would-be push for visibility.
        eprintln!(
            "genaryx-relay: would push (no APNs token on file): {} - {}",
            intent.title, intent.body
        );
        return;
    }
    for apns_token in tokens {
        push.send(crate::push::Notification {
            apns_token,
            title: intent.title.clone(),
            body: intent.body.clone(),
            run_id: intent.run_id.clone(),
            incident_id: intent.incident_id.clone(),
            kind: intent.kind.clone(),
        });
    }
}

/// Periodic belt-and-braces reconcile sweep (D12.6 R6). Runs forever;
/// a single failed reconcile just logs and waits for the next tick (the
/// previous snapshot stays live -- fail-open on a transient read error here
/// would be worse than serving slightly-stale data the phone can still act
/// on, matching the Cloud's own gateway "estimate-then-settle" honesty).
pub async fn run_reconcile_sweep(
    engine: std::sync::Arc<ExceptionEngine>,
    cloud: CloudClient,
    interval: std::time::Duration,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.tick().await; // first tick fires immediately; the caller already reconciled once at startup
    loop {
        ticker.tick().await;
        if let Err(e) = engine.reconcile(&cloud).await {
            eprintln!("genaryx-relay: exceptions: periodic reconcile failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- pure classification -------------------------------------------

    #[test]
    fn classify_fraction_boundary() {
        assert_eq!(classify_fraction(0.79), ExceptionClass::NearCap);
        assert_eq!(classify_fraction(0.8), ExceptionClass::NearCap);
        assert_eq!(classify_fraction(0.999), ExceptionClass::NearCap);
        assert_eq!(classify_fraction(1.0), ExceptionClass::OverCap);
        assert_eq!(classify_fraction(1.5), ExceptionClass::OverCap);
    }

    #[test]
    fn classify_incident_kinds() {
        assert_eq!(
            classify_incident("sustained_loop", Severity::Low),
            ExceptionClass::Runaway
        );
        assert_eq!(
            classify_incident("fanout_explosion", Severity::Medium),
            ExceptionClass::Runaway
        );
        assert_eq!(
            classify_incident("budget_exhausted", Severity::Low),
            ExceptionClass::OverCap
        );
        assert_eq!(
            classify_incident("spend_spike", Severity::Medium),
            ExceptionClass::AtRisk
        );
        assert_eq!(
            classify_incident("spend_spike", Severity::High),
            ExceptionClass::Runaway
        );
        assert_eq!(
            classify_incident("something_new", Severity::Critical),
            ExceptionClass::Runaway
        );
    }

    #[test]
    fn is_hard_matches_d12_2b_taxonomy() {
        assert!(is_hard(ExceptionClass::OverCap));
        assert!(is_hard(ExceptionClass::Runaway));
        assert!(!is_hard(ExceptionClass::NearCap));
        assert!(!is_hard(ExceptionClass::AtRisk));
        assert!(!is_hard(ExceptionClass::PendingApproval));
    }

    // ---- dedup, ported from push.rs -------------------------------------

    #[test]
    fn should_notify_dedupes_within_window_then_allows_again() {
        let mut dedup = HashMap::new();
        assert!(should_notify(&mut dedup, "acme", "r1", "kill", 1000, 600));
        assert!(
            !should_notify(&mut dedup, "acme", "r1", "kill", 1300, 600),
            "still within 600s"
        );
        assert!(
            should_notify(&mut dedup, "acme", "r1", "kill", 1601, 600),
            "window elapsed"
        );
    }

    #[test]
    fn should_notify_keys_are_independent() {
        let mut dedup = HashMap::new();
        assert!(should_notify(&mut dedup, "acme", "r1", "kill", 1000, 600));
        assert!(
            should_notify(&mut dedup, "acme", "r2", "kill", 1000, 600),
            "different run"
        );
        assert!(
            should_notify(&mut dedup, "acme", "r1", "budget", 1000, 600),
            "different reason"
        );
        assert!(
            should_notify(&mut dedup, "other", "r1", "kill", 1000, 600),
            "different org"
        );
    }

    #[test]
    fn mark_incident_notified_dedupes_independently_of_should_notify() {
        let mut tracker = HashMap::new();
        assert!(mark_incident_notified(
            &mut tracker,
            "inc-1",
            1_000_000,
            600_000
        ));
        assert!(!mark_incident_notified(
            &mut tracker,
            "inc-1",
            1_300_000,
            600_000
        ));
        assert!(mark_incident_notified(
            &mut tracker,
            "inc-1",
            1_700_000,
            600_000
        ));
    }

    // ---- queue shape: action versus report --------------------------------
    //
    // Measured against a real 9,288-run fleet, the queue came back with 189
    // rows (9 alerts + 180 incidents, ~150 of them one budget_exhausted per
    // shard of a single batch), and the protagonist run appeared TWICE, once
    // with its real money and once with zeros. These lock in the fix.

    fn alert(run: &str, spent: i64, budget: i64) -> Alert {
        Alert {
            run_id: run.into(),
            spent_microusd: spent,
            budget_micros: budget,
            fraction: spent as f64 / budget as f64,
            killed: false,
        }
    }

    fn incident(id: &str, kind: &str, run: Option<&str>, agent: Option<&str>) -> Incident {
        Incident {
            id: id.into(),
            org: "default".into(),
            run_id: run.map(str::to_string),
            agent_id: agent.map(str::to_string),
            kind: kind.into(),
            severity: Severity::High,
            source: None,
            summary: None,
            first_seen_millis: 1_000_000,
            last_seen_millis: 2_000_000,
            occurrences: 7,
            acknowledged: false,
            last_notified_millis: 0,
        }
    }

    /// Drive the same classification `reconcile` does, without a live Cloud.
    fn shape(alerts: &[Alert], incidents: &[Incident]) -> (Vec<ExceptionItem>, Vec<DigestRow>) {
        let mut items: HashMap<String, ExceptionItem> = HashMap::new();
        for a in alerts {
            let key = format!("run:{}", a.run_id);
            items.insert(key.clone(), alert_item(&key, a, 0, 0));
        }
        let mut digest: HashMap<String, DigestRow> = HashMap::new();
        for inc in incidents.iter().filter(|i| !i.acknowledged) {
            match inc.run_id.as_ref().map(|r| format!("run:{r}")) {
                Some(key) if items.contains_key(&key) => {
                    if let Some(item) = items.get_mut(&key) {
                        merge_incident(item, inc);
                    }
                }
                _ if incident_is_already_contained(&inc.kind) => fold_into_digest(&mut digest, inc),
                _ => {
                    let key = format!("incident:{}", inc.id);
                    items.insert(key.clone(), incident_item(&key, inc));
                }
            }
        }
        (
            items.into_values().collect(),
            digest.into_values().collect(),
        )
    }

    #[test]
    fn an_incident_about_an_alerted_run_merges_instead_of_adding_a_second_row() {
        let alerts = vec![alert(
            "reconciliation-batch-eod-002-LIVE",
            6_910_000,
            5_570_000,
        )];
        let incidents = vec![incident(
            "inc-1",
            "budget_exhausted",
            Some("reconciliation-batch-eod-002-LIVE"),
            Some("agent://meridian.example/treasury/reconciliation-batch"),
        )];
        let (queue, digest) = shape(&alerts, &incidents);

        assert_eq!(queue.len(), 1, "one run must produce exactly one row");
        assert!(digest.is_empty(), "it merged, so nothing to digest");
        let item = &queue[0];
        // The alert keeps the money: this is the half that was showing $0.00.
        assert_eq!(item.spent_microusd, 6_910_000);
        assert_eq!(item.budget_micros, Some(5_570_000));
        // The incident keeps its identity and kind: this is the half that
        // would otherwise be lost by naive dedup.
        assert_eq!(item.incident_id.as_deref(), Some("inc-1"));
        assert_eq!(item.kind, "budget_exhausted");
        assert_eq!(item.severity.as_deref(), Some("high"));
    }

    #[test]
    fn a_borrowed_detection_reaches_the_phone_saying_who_made_it() {
        // The operator about to kill a run is entitled to know whether this
        // plane measured the thing or was told it. That answer only survives
        // if BOTH paths carry it: a finding that earns its own row, and a
        // finding about a run that already has one and therefore merges.
        let mut standalone = incident(
            "inc-idryx-1",
            "agent_shadow_tool",
            None,
            Some("agent://meridian.example/support/support-tier2-bot"),
        );
        standalone.source = Some("idryx".into());
        standalone.summary =
            Some("agent uses tool(s) from an unsanctioned MCP server: notes_export".into());

        let mut merged = incident(
            "inc-idryx-2",
            "runaway_agent",
            Some("reconciliation-batch-eod-002-LIVE"),
            Some("agent://meridian.example/treasury/reconciliation-batch"),
        );
        merged.source = Some("idryx".into());
        merged.summary = Some("agent spend incident: budget_exhausted=40".into());

        let alerts = vec![alert(
            "reconciliation-batch-eod-002-LIVE",
            6_910_000,
            5_570_000,
        )];
        let (queue, _) = shape(&alerts, &[standalone, merged]);

        for item in &queue {
            assert_eq!(
                item.source.as_deref(),
                Some("idryx"),
                "provenance must survive both the standalone and the merged path"
            );
            assert!(
                item.summary.as_deref().is_some_and(|s| !s.is_empty()),
                "for a borrowed finding the detector's sentence is usually the \
                 only readable explanation there is"
            );
        }
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn an_incident_this_plane_measured_itself_names_no_source() {
        // If `source` were set on our own detections it would stop meaning
        // anything, and the one row that was borrowed would read like the rest.
        let incidents = vec![incident(
            "inc-1",
            "fanout_explosion",
            None,
            Some("agent://meridian.example/support/support-tier2-bot"),
        )];
        let (queue, _) = shape(&[], &incidents);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].source, None);
        assert_eq!(queue[0].summary, None);
    }

    #[test]
    fn a_shard_storm_becomes_one_counted_line_not_a_hundred_and_fifty_rows() {
        let agent = "agent://meridian.example/treasury/reconciliation-batch";
        let incidents: Vec<Incident> = (0..150)
            .map(|i| {
                incident(
                    &format!("inc-{i}"),
                    "budget_exhausted",
                    Some(&format!("reconciliation-batch-eod-002-s{i:03}")),
                    Some(agent),
                )
            })
            .collect();
        let (queue, digest) = shape(&[], &incidents);

        assert!(queue.is_empty(), "already-contained events are not tasks");
        assert_eq!(digest.len(), 1, "one line, not 150");
        assert_eq!(digest[0].count, 150, "the count is exact, never rounded");
        assert_eq!(digest[0].kind, "budget_exhausted");
        assert_eq!(
            digest[0].agent_id.as_deref(),
            Some(agent),
            "naming the agent is what makes the line a sentence"
        );
    }

    #[test]
    fn live_signals_keep_their_own_row_even_without_an_alert() {
        // budget_exhausted means the breaker already fired, so it digests.
        // These three mean it is STILL happening, so they stay actionable.
        for kind in ["sustained_loop", "spend_spike", "fanout_explosion"] {
            let (queue, digest) = shape(&[], &[incident("i", kind, Some("r1"), None)]);
            assert_eq!(queue.len(), 1, "{kind} must stay visible");
            assert!(digest.is_empty(), "{kind} must not be digested");
        }
        let (queue, digest) = shape(&[], &[incident("i", "budget_exhausted", Some("r1"), None)]);
        assert!(queue.is_empty());
        assert_eq!(digest.len(), 1);
    }

    #[test]
    fn an_unknown_incident_kind_is_shown_not_quietly_counted() {
        // Fail toward visibility: a kind this build has never heard of must
        // not disappear into a number.
        let (queue, digest) = shape(&[], &[incident("i", "some_future_kind", Some("r1"), None)]);
        assert_eq!(queue.len(), 1);
        assert!(digest.is_empty());
    }

    #[test]
    fn merging_widens_the_class_and_never_narrows_it() {
        // A run only 84% through its budget, but in a fan-out explosion, must
        // not read as a mild "near cap".
        let alerts = vec![alert("r1", 840, 1000)];
        let (queue, _) = shape(
            &alerts,
            &[incident("i", "fanout_explosion", Some("r1"), None)],
        );
        assert_eq!(queue.len(), 1);
        assert!(
            is_hard(queue[0].class),
            "an incident that classifies harder than the fraction must win"
        );
    }

    #[test]
    fn the_same_agent_with_two_kinds_gets_two_digest_lines() {
        let agent = "agent://meridian.example/support/support-tier1-bot";
        let (_, mut digest) = shape(
            &[],
            &[
                incident("a", "budget_exhausted", Some("r1"), Some(agent)),
                incident("b", "budget_exhausted", Some("r2"), Some(agent)),
                incident("c", "budget_exhausted", Some("r3"), Some("agent://other")),
            ],
        );
        digest.sort_by_key(|d| std::cmp::Reverse(d.count));
        assert_eq!(digest.len(), 2, "grouped by (kind, agent), not kind alone");
        assert_eq!(digest[0].count, 2);
        assert_eq!(digest[1].count, 1);
    }

    // ---- aggregates -------------------------------------------------------

    #[test]
    fn headroom_sums_positive_remaining_and_floors_negative_at_zero() {
        let alerts = vec![
            Alert {
                run_id: "r1".into(),
                spent_microusd: 800,
                budget_micros: 1000,
                fraction: 0.8,
                killed: false,
            },
            Alert {
                run_id: "r2".into(),
                spent_microusd: 1200,
                budget_micros: 1000,
                fraction: 1.2,
                killed: false,
            },
        ];
        assert_eq!(headroom_from_alerts(&alerts), 200); // r1's 200 headroom; r2 over budget contributes 0
    }

    #[test]
    fn burn_rate_from_two_samples() {
        let mut samples = VecDeque::new();
        push_burn_sample(&mut samples, 1000, 10_000);
        push_burn_sample(&mut samples, 1060, 22_000); // +12,000 over 60s = 12,000/min
        assert_eq!(burn_rate_per_min(&samples), 12_000);
    }

    #[test]
    fn burn_rate_is_zero_with_one_sample_or_no_elapsed_time() {
        let mut samples = VecDeque::new();
        assert_eq!(burn_rate_per_min(&samples), 0);
        push_burn_sample(&mut samples, 1000, 500);
        assert_eq!(burn_rate_per_min(&samples), 0);
    }

    #[test]
    fn spend_going_backwards_discards_the_previous_epoch() {
        // The Cloud keeps its store in memory, so a restart takes cumulative
        // spend back to zero. Before this, the stale high samples stayed in the
        // window and the phone and watch showed a NEGATIVE burn rate.
        let mut samples = VecDeque::new();
        push_burn_sample(&mut samples, 100, 4_700_000_000);
        push_burn_sample(&mut samples, 160, 4_760_000_000);
        assert!(
            burn_rate_per_min(&samples) > 0,
            "rising spend, positive rate"
        );

        push_burn_sample(&mut samples, 220, 4_255_000_000); // the Cloud restarted
        assert_eq!(samples.len(), 1, "the previous epoch is discarded whole");
        assert_eq!(
            burn_rate_per_min(&samples),
            0,
            "not negative, just unknown yet"
        );

        push_burn_sample(&mut samples, 280, 4_300_000_000);
        assert!(
            burn_rate_per_min(&samples) > 0,
            "and it rebuilds from the restart rather than lying in either direction"
        );
    }

    #[test]
    fn burn_samples_drop_outside_the_window_but_keep_at_least_one() {
        let mut samples = VecDeque::new();
        push_burn_sample(&mut samples, 0, 0);
        push_burn_sample(&mut samples, 5000, 5000); // far beyond BURN_WINDOW_SECS later
        assert_eq!(
            samples.len(),
            1,
            "the stale sample is evicted, one sample remains"
        );
    }

    // ---- engine integration ------------------------------------------------

    fn engine() -> ExceptionEngine {
        ExceptionEngine::new("acme", 0.8, 600)
    }

    fn record(json: &str) -> RawRecord {
        RawRecord {
            raw: json.to_string(),
            file: None,
            offset: None,
        }
    }

    #[test]
    fn budget_then_run_update_crossing_threshold_produces_one_item_and_one_push() {
        let eng = engine();
        assert!(
            eng.handle_raw_record(
                &record(r#"{"type":"budget","run":"r1","budget_micros":1000}"#),
                1000
            )
            .is_none()
        );

        // Below alert_pct: tracked budget, but no item/push yet.
        let below = eng.handle_raw_record(
            &record(r#"{"type":"run_update","run":{"run_id":"r1","model":"gpt","agent_id":"","spent_microusd":500,"calls":1,"cache_hits":0,"steps":1,"last_seen_millis":1,"killed":false}}"#),
            1001,
        );
        assert!(below.is_none());
        assert!(eng.snapshot().queue.is_empty());

        // Crosses 80%: one item, one push.
        let over = eng.handle_raw_record(
            &record(r#"{"type":"run_update","run":{"run_id":"r1","model":"gpt","agent_id":"","spent_microusd":900,"calls":2,"cache_hits":0,"steps":2,"last_seen_millis":2,"killed":false}}"#),
            1002,
        );
        let intent = over.expect("should push on first crossing");
        assert_eq!(intent.kind, "budget");
        assert!(!intent.hard, "near-cap is SOFT");
        let snap = eng.snapshot();
        assert_eq!(snap.queue.len(), 1);
        assert_eq!(snap.queue[0].class, ExceptionClass::NearCap);
        assert_eq!(snap.queue[0].spent_microusd, 900);

        // A second crossing update within the dedup window updates state but
        // does not push again.
        let again = eng.handle_raw_record(
            &record(r#"{"type":"run_update","run":{"run_id":"r1","model":"gpt","agent_id":"","spent_microusd":950,"calls":3,"cache_hits":0,"steps":3,"last_seen_millis":3,"killed":false}}"#),
            1003,
        );
        assert!(again.is_none(), "deduped within the window");
        assert_eq!(
            eng.snapshot().queue[0].spent_microusd,
            950,
            "state still updates"
        );
    }

    #[test]
    fn run_update_with_unknown_budget_is_ignored() {
        let eng = engine();
        let out = eng.handle_raw_record(
            &record(r#"{"type":"run_update","run":{"run_id":"unknown","model":"gpt","agent_id":"","spent_microusd":9999,"calls":1,"cache_hits":0,"steps":1,"last_seen_millis":1,"killed":false}}"#),
            1000,
        );
        assert!(out.is_none());
        assert!(eng.snapshot().queue.is_empty());
    }

    #[test]
    fn kill_event_is_hard_and_always_over_cap_class() {
        let eng = engine();
        let intent = eng
            .handle_raw_record(&record(r#"{"type":"kill","run":"r9"}"#), 1000)
            .expect("kill always pushes on first occurrence");
        assert!(intent.hard);
        assert_eq!(intent.kind, "kill");
        // Checked against the engine's own items: a killed run is deliberately
        // absent from `snapshot()`, which is the pager's view.
        let items = eng.tracked_items();
        assert_eq!(items.len(), 1);
        assert!(items[0].killed);
        assert!(is_hard(items[0].class));
        assert!(
            eng.snapshot().queue.is_empty(),
            "and it does not reach the pager"
        );
    }

    #[test]
    fn kill_on_an_already_tracked_run_updates_the_same_item_not_a_duplicate() {
        let eng = engine();
        eng.handle_raw_record(
            &record(r#"{"type":"budget","run":"r1","budget_micros":1000}"#),
            1000,
        );
        eng.handle_raw_record(
            &record(r#"{"type":"run_update","run":{"run_id":"r1","model":"gpt","agent_id":"","spent_microusd":900,"calls":1,"cache_hits":0,"steps":1,"last_seen_millis":1,"killed":false}}"#),
            1001,
        );
        eng.handle_raw_record(&record(r#"{"type":"kill","run":"r1"}"#), 1002);
        let items = eng.tracked_items();
        assert_eq!(items.len(), 1, "same run key, no duplicate row");
        assert!(items[0].killed);
    }

    #[test]
    fn incident_event_creates_item_and_acked_incident_clears_it() {
        let eng = engine();
        let intent = eng
            .handle_raw_record(
                &record(r#"{"type":"incident","id":"sustained_loop:r1","org":"acme","run_id":"r1","agent_id":null,"kind":"sustained_loop","severity":"high","first_seen_millis":1000,"last_seen_millis":2000,"occurrences":3,"acknowledged":false,"last_notified_millis":0}"#),
                1000,
            )
            .expect("first trip pushes");
        assert!(intent.hard, "sustained_loop is runaway, HARD");
        assert_eq!(eng.snapshot().queue.len(), 1);

        // Re-trip within the dedup window: item stays, no second push.
        let again = eng.handle_raw_record(
            &record(r#"{"type":"incident","id":"sustained_loop:r1","org":"acme","run_id":"r1","agent_id":null,"kind":"sustained_loop","severity":"high","first_seen_millis":1000,"last_seen_millis":2500,"occurrences":4,"acknowledged":false,"last_notified_millis":0}"#),
            1100,
        );
        assert!(again.is_none());
        assert_eq!(eng.snapshot().queue.len(), 1);

        // An acknowledged copy (e.g. reconcile picking up an ack) clears it.
        let acked = eng.handle_raw_record(
            &record(r#"{"type":"incident","id":"sustained_loop:r1","org":"acme","run_id":"r1","agent_id":null,"kind":"sustained_loop","severity":"high","first_seen_millis":1000,"last_seen_millis":2500,"occurrences":4,"acknowledged":true,"last_notified_millis":0}"#),
            1200,
        );
        assert!(acked.is_none());
        assert!(eng.snapshot().queue.is_empty());
    }

    #[test]
    fn unrecognized_or_malformed_record_is_ignored_not_a_panic() {
        let eng = engine();
        assert!(eng.handle_raw_record(&record("not json"), 1000).is_none());
        assert!(
            eng.handle_raw_record(&record(r#"{"type":"future_event"}"#), 1000)
                .is_none()
        );
        assert!(eng.snapshot().queue.is_empty());
    }

    #[test]
    fn a_killed_run_disappears_from_the_pager_entirely() {
        // The operator's rule: a killed run is done with, and the pager must
        // not spend a row on it. It stays fully visible in Genaryx, which reads
        // the Cloud directly and never sees this queue.
        let eng = engine();
        eng.handle_raw_record(&record(r#"{"type":"kill","run":"already-dead"}"#), 1000);
        eng.handle_raw_record(
            &record(r#"{"type":"incident","id":"loop:r2","org":"acme","run_id":"r2","agent_id":null,"kind":"sustained_loop","severity":"high","first_seen_millis":1000,"last_seen_millis":1000,"occurrences":1,"acknowledged":false,"last_notified_millis":0}"#),
            1001,
        );
        let snap = eng.snapshot();
        assert!(
            snap.queue.iter().all(|i| !i.killed),
            "no killed run may appear on the pager"
        );
        assert_eq!(snap.queue.len(), 1, "only the live detection remains");
        assert_eq!(snap.queue[0].run_id.as_deref(), Some("r2"));
    }

    #[test]
    fn anything_below_the_alert_threshold_is_not_a_pager_item() {
        assert!(shows_on_a_pager(ExceptionClass::OverCap));
        assert!(shows_on_a_pager(ExceptionClass::Runaway));
        assert!(shows_on_a_pager(ExceptionClass::NearCap));
        assert!(
            !shows_on_a_pager(ExceptionClass::AtRisk),
            "below 80% is ordinary operation, not something to wake someone for"
        );
        assert!(
            !shows_on_a_pager(ExceptionClass::PendingApproval),
            "governance queueing is desktop work"
        );
    }

    #[test]
    fn the_queue_is_ordered_by_how_far_past_the_limit() {
        // Over the cap outranks a running detection, which outranks merely
        // approaching the cap. Both clients render this order as given, so it
        // is defined once and cannot drift between the two surfaces.
        assert!(class_rank(ExceptionClass::OverCap) < class_rank(ExceptionClass::Runaway));
        assert!(class_rank(ExceptionClass::Runaway) < class_rank(ExceptionClass::NearCap));
        assert!(class_rank(ExceptionClass::NearCap) < class_rank(ExceptionClass::AtRisk));

        fn item(
            run: &str,
            class: ExceptionClass,
            fraction: Option<f64>,
            killed: bool,
        ) -> ExceptionItem {
            ExceptionItem {
                key: format!("run:{run}"),
                run_id: Some(run.into()),
                incident_id: None,
                agent_id: None,
                kind: "budget".into(),
                class,
                severity: None,
                source: None,
                summary: None,
                headline: String::new(),
                spent_microusd: 0,
                budget_micros: None,
                fraction,
                first_seen_unix: 0,
                last_seen_unix: 0,
                acknowledged: false,
                killed,
                copilot: None,
            }
        }
        let mut q = [
            item("near", ExceptionClass::NearCap, Some(0.85), false),
            item("worst", ExceptionClass::OverCap, Some(1.40), false),
            item("runaway", ExceptionClass::Runaway, None, false),
            item("over", ExceptionClass::OverCap, Some(1.10), false),
        ];
        q.sort_by(queue_order);
        let ids: Vec<&str> = q.iter().filter_map(|i| i.run_id.as_deref()).collect();
        assert_eq!(
            ids,
            vec!["worst", "over", "runaway", "near"],
            "over cap worst-first, then still-running, then near cap"
        );
    }

    #[tokio::test]
    async fn reconcile_against_a_live_cloud_is_skipped_without_one() {
        // Mirrors the connectors crate's own live-test convention (skip
        // gracefully with an eprintln, never fail CI on a missing local
        // Cloud). `CloudClient::new` never touches the network, only
        // `.summary()` etc. would; a port nothing listens on proves the
        // fail-closed path without needing an actual server.
        let cloud = CloudClient::new("http://127.0.0.1:1", "key:acme:viewer").unwrap();
        let eng = engine();
        match eng.reconcile(&cloud).await {
            Ok(()) => panic!("unexpected success against a closed port"),
            Err(_) => eprintln!(
                "SKIP: reconcile_against_a_live_cloud_is_skipped_without_one (no live Cloud)"
            ),
        }
    }

    // ---- money snapshot (GET /relay/v1/money) ------------------------------

    fn agent_state(spent_microusd: i64) -> AgentState {
        AgentState {
            spent_microusd,
            ..Default::default()
        }
    }

    fn agent_agg(
        agent_id: &str,
        spent_microusd: i64,
        calls: u64,
        runs: u64,
        last_seen_millis: i64,
    ) -> AgentAgg {
        AgentAgg {
            agent_id: agent_id.to_string(),
            spent_microusd,
            calls,
            runs,
            last_seen_millis,
        }
    }

    /// A minimal pager-visible item attributed to `agent_id`, for the
    /// money-snapshot join tests. `class`/`severity` are the only things
    /// those tests vary; everything else is a zeroed placeholder.
    fn exception_item_for_agent(
        key: &str,
        agent_id: &str,
        class: ExceptionClass,
        severity: Option<&str>,
    ) -> ExceptionItem {
        ExceptionItem {
            key: key.to_string(),
            run_id: None,
            incident_id: None,
            agent_id: Some(agent_id.to_string()),
            kind: "sustained_loop".to_string(),
            class,
            severity: severity.map(str::to_string),
            source: None,
            summary: None,
            headline: String::new(),
            spent_microusd: 0,
            budget_micros: None,
            fraction: None,
            first_seen_unix: 0,
            last_seen_unix: 0,
            acknowledged: false,
            killed: false,
            copilot: None,
        }
    }

    fn find_agent<'a>(snap: &'a MoneySnapshot, agent_id: &str) -> &'a AgentRollup {
        snap.agents
            .iter()
            .find(|a| a.agent_id == agent_id)
            .unwrap_or_else(|| panic!("agent {agent_id} missing from the snapshot"))
    }

    #[test]
    fn agents_are_sorted_by_spend_descending_and_capped_at_the_constant() {
        let eng = engine();
        // A few more than the cap, so the cap is genuinely exercised rather
        // than happening to equal the input size.
        let total = MAX_AGENTS_ON_THE_WIRE + 3;
        for i in 0..total {
            eng.seed_agent_for_test(&format!("agent-{i:04}"), agent_state(i as i64));
        }

        let snap = eng.money_snapshot();
        assert_eq!(
            snap.agents.len(),
            MAX_AGENTS_ON_THE_WIRE,
            "capped at the constant"
        );
        for pair in snap.agents.windows(2) {
            assert!(
                pair[0].spent_microusd >= pair[1].spent_microusd,
                "must sort highest spend first"
            );
        }
        assert_eq!(
            snap.agents[0].spent_microusd,
            (total - 1) as i64,
            "the single highest spender leads"
        );
    }

    #[test]
    fn truncated_agents_are_accounted_for_and_the_total_still_adds_up() {
        let eng = engine();
        let extra = 7;
        let total_agents = MAX_AGENTS_ON_THE_WIRE + extra;
        let mut total_spend: i64 = 0;
        for i in 0..total_agents {
            let spend = (i + 1) as i64; // 1..=total_agents, all distinct and nonzero
            total_spend += spend;
            eng.seed_agent_for_test(&format!("agent-{i:04}"), agent_state(spend));
        }

        let snap = eng.money_snapshot();
        assert_eq!(snap.agents.len(), MAX_AGENTS_ON_THE_WIRE);
        assert_eq!(snap.agents_truncated, extra, "exactly the ones cut");
        let shown_spend: i64 = snap.agents.iter().map(|a| a.spent_microusd).sum();
        assert_eq!(
            shown_spend + snap.others_spent_microusd,
            total_spend,
            "shown spend plus others must equal the fleet total"
        );
    }

    #[test]
    fn a_per_agent_burn_rate_is_computed_from_two_reconciles() {
        let mut agents_map: HashMap<String, AgentState> = HashMap::new();
        agents_map = rebuild_agents(
            &mut agents_map,
            &[agent_agg("a1", 10_000, 1, 1, 1_000_000)],
            1000,
        );
        agents_map = rebuild_agents(
            &mut agents_map,
            &[agent_agg("a1", 22_000, 2, 1, 1_060_000)],
            1060,
        );

        // +12,000 over 60s = 12,000/min: the exact fleet-level case
        // `burn_rate_from_two_samples` already proves, now proven per agent.
        let rate = burn_rate_per_min(&agents_map["a1"].burn_samples);
        assert_eq!(rate, 12_000);
    }

    #[test]
    fn a_per_agent_counter_going_backwards_does_not_produce_a_negative_rate() {
        let mut agents_map: HashMap<String, AgentState> = HashMap::new();
        agents_map = rebuild_agents(
            &mut agents_map,
            &[agent_agg("a1", 4_700_000_000, 1, 1, 100_000)],
            100,
        );
        agents_map = rebuild_agents(
            &mut agents_map,
            &[agent_agg("a1", 4_760_000_000, 2, 1, 160_000)],
            160,
        );
        assert!(
            burn_rate_per_min(&agents_map["a1"].burn_samples) > 0,
            "rising spend, positive rate"
        );

        // The Cloud restarted: agent a1's cumulative spend resets lower.
        agents_map = rebuild_agents(
            &mut agents_map,
            &[agent_agg("a1", 4_255_000_000, 3, 1, 220_000)],
            220,
        );
        assert_eq!(
            burn_rate_per_min(&agents_map["a1"].burn_samples),
            0,
            "not negative, the previous epoch was discarded"
        );
    }

    #[test]
    fn a_run_update_teaches_the_agent_even_when_the_run_earns_no_row() {
        // The gates in `handle_run_update` return early for a run with no
        // budget, but the agent it named is exactly what an alert for that same
        // run will be missing later, so it has to be learned before them.
        let eng = engine();
        assert!(
            eng.handle_raw_record(
                &record(r#"{"type":"run_update","run":{"run_id":"r9","model":"gpt","agent_id":"agent-late","spent_microusd":10,"calls":1,"cache_hits":0,"steps":1,"last_seen_millis":1,"killed":false}}"#),
                1000
            )
            .is_none(),
            "no budget, so no queue row and no push"
        );
        assert!(eng.snapshot().queue.is_empty());

        let st = eng.state.lock().unwrap();
        assert_eq!(
            st.run_agents.get("r9").map(String::as_str),
            Some("agent-late")
        );
    }

    #[test]
    fn an_alert_only_item_is_given_the_agent_learned_elsewhere() {
        // An alert states no agent at all, so on its own it would reach the
        // phone anonymous and join nothing in the agents list.
        let mut items = HashMap::new();
        let mut anonymous =
            exception_item_for_agent("run:r1", "", ExceptionClass::OverCap, Some("high"));
        anonymous.run_id = Some("r1".to_string());
        anonymous.agent_id = None;
        items.insert("run:r1".to_string(), anonymous);

        let mut already_known = exception_item_for_agent(
            "run:r2",
            "agent-own",
            ExceptionClass::Runaway,
            Some("critical"),
        );
        already_known.run_id = Some("r2".to_string());
        items.insert("run:r2".to_string(), already_known);

        let mut run_agents = HashMap::from([
            ("r1".to_string(), "agent-x".to_string()),
            ("r2".to_string(), "agent-should-not-win".to_string()),
            ("r-gone".to_string(), "agent-stale".to_string()),
        ]);
        let budgets = HashMap::new();

        attach_known_agents(&mut items, &mut run_agents, &budgets);

        assert_eq!(
            items["run:r1"].agent_id.as_deref(),
            Some("agent-x"),
            "the anonymous alert item gets the agent learned from the stream"
        );
        assert_eq!(
            items["run:r2"].agent_id.as_deref(),
            Some("agent-own"),
            "an item that states its own agent is never overwritten"
        );
        assert!(
            !run_agents.contains_key("r-gone"),
            "a run neither queued nor budgeted is forgotten rather than kept forever"
        );
        assert!(
            run_agents.contains_key("r1"),
            "a queued run is still worth remembering"
        );
    }

    #[test]
    fn open_exceptions_and_worst_severity_are_joined_by_agent() {
        let eng = engine();
        eng.seed_agent_for_test("agent-a", agent_state(1_000));
        eng.seed_agent_for_test("agent-b", agent_state(500));

        eng.seed_item_for_test(exception_item_for_agent(
            "run:a1",
            "agent-a",
            ExceptionClass::NearCap,
            Some("medium"),
        ));
        eng.seed_item_for_test(exception_item_for_agent(
            "run:a2",
            "agent-a",
            ExceptionClass::Runaway,
            Some("critical"),
        ));
        // A killed item for the same agent must not count as open.
        let mut killed_item =
            exception_item_for_agent("run:a3", "agent-a", ExceptionClass::OverCap, Some("high"));
        killed_item.killed = true;
        eng.seed_item_for_test(killed_item);

        let snap = eng.money_snapshot();
        let a = find_agent(&snap, "agent-a");
        assert_eq!(a.open_exceptions, 2, "the killed item must not count");
        assert_eq!(a.worst_severity.as_deref(), Some("critical"));

        let b = find_agent(&snap, "agent-b");
        assert_eq!(b.open_exceptions, 0);
        assert_eq!(b.worst_severity, None);
    }

    #[test]
    fn an_agent_cloud_stops_reporting_disappears_from_the_next_rebuild() {
        let mut existing: HashMap<String, AgentState> = HashMap::new();
        existing.insert("agent-a".to_string(), agent_state(100));
        existing.insert("agent-b".to_string(), agent_state(200));

        // Cloud's next `/v1/agents` read no longer mentions agent-b.
        let next = rebuild_agents(&mut existing, &[agent_agg("agent-a", 150, 3, 1, 5000)], 5);

        assert_eq!(next.len(), 1, "agent-b is gone, Cloud no longer reports it");
        assert!(next.contains_key("agent-a"));
        assert!(!next.contains_key("agent-b"));
    }
}
