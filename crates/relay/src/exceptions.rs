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

use genaryx_connectors::{Alert, CloudClient, ConnectorError, Incident, RunAgg, Severity};
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
    /// `"budget"` | `"kill"` | the incident kind (`budget_exhausted` |
    /// `sustained_loop` | `spend_spike` | `fanout_explosion`).
    pub kind: String,
    pub class: ExceptionClass,
    pub severity: Option<String>,
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
    pub queue: Vec<ExceptionItem>,
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
    /// Live-updated from `Budget` stream events, seeded/corrected from
    /// `/v1/alerts` at each reconcile: the only way this relay learns a
    /// run's central-budget override without a `/v1/budgets` read (not one
    /// of `CloudClient`'s seven read methods).
    budgets: HashMap<String, i64>,
    burn_samples: VecDeque<(i64, i64)>,
    push_dedup: HashMap<(String, String, String), i64>,
    incident_last_notified: HashMap<String, i64>,
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

    /// The current bounded, pre-computed snapshot `GET /relay/v1/exceptions`
    /// serves. HARD items sort first, then most-recently-seen first.
    pub fn snapshot(&self) -> ExceptionSnapshot {
        let st = self.state.lock().expect("exception engine mutex poisoned");
        let mut queue: Vec<ExceptionItem> = st.items.values().cloned().collect();
        queue.sort_by(|a, b| {
            is_hard(b.class)
                .cmp(&is_hard(a.class))
                .then(b.last_seen_unix.cmp(&a.last_seen_unix))
        });
        queue.truncate(MAX_QUEUE_LEN);
        ExceptionSnapshot {
            aggregates: st.aggregates,
            queue,
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

    /// Full resync against the Cloud's own authoritative reads: `/v1/summary`
    /// (aggregate spend), `/v1/alerts` (near/over-cap runs + their budgets),
    /// `/v1/incidents` (open, unacknowledged incidents). Replaces the queue
    /// wholesale but preserves each surviving item's `first_seen_unix` (the
    /// one thing Cloud doesn't hand back for alerts) rather than resetting
    /// it every sweep.
    pub async fn reconcile(&self, cloud: &CloudClient) -> Result<(), ConnectorError> {
        let summary = cloud.summary().await?;
        let alerts = cloud.alerts().await?;
        let incidents = cloud.incidents().await?;
        let now = now_unix();

        let mut st = self.state.lock().expect("exception engine mutex poisoned");
        let mut items = HashMap::with_capacity(alerts.len() + incidents.len());

        for a in &alerts {
            let key = format!("run:{}", a.run_id);
            let first_seen = st.items.get(&key).map_or(now, |old| old.first_seen_unix);
            items.insert(key.clone(), alert_item(&key, a, first_seen, now));
            st.budgets.insert(a.run_id.clone(), a.budget_micros);
        }
        for inc in incidents.iter().filter(|i| !i.acknowledged) {
            let key = format!("incident:{}", inc.id);
            items.insert(key.clone(), incident_item(&key, inc));
        }
        st.items = items;
        push_burn_sample(&mut st.burn_samples, now, summary.spent_microusd);
        st.aggregates = Aggregates {
            spend_microusd: summary.spent_microusd,
            headroom_microusd: headroom_from_alerts(&alerts),
            burn_rate_microusd_per_min: burn_rate_per_min(&st.burn_samples),
            updated_at_unix: now,
        };
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
                kind: "budget".to_string(),
                class,
                severity: None,
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
                        kind: "kill".to_string(),
                        class: ExceptionClass::OverCap,
                        severity: None,
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
        let run_label = inc.run_id.clone().unwrap_or_else(|| inc.id.clone());
        st.items.insert(key.clone(), incident_item(&key, &inc));

        let now_ms = now * 1000;
        let window_ms = self.dedup_secs * 1000;
        if !mark_incident_notified(&mut st.incident_last_notified, &inc.id, now_ms, window_ms) {
            return None;
        }
        Some(PushIntent {
            title: "Agent running hot".to_string(),
            body: format!(
                "Agent/run {run_label} running hot - {}. Tap to review and kill.",
                inc.kind
            ),
            run_id: inc.run_id,
            incident_id: Some(inc.id),
            kind: inc.kind,
            hard: is_hard(class),
        })
    }
}

fn alert_item(key: &str, a: &Alert, first_seen_unix: i64, now: i64) -> ExceptionItem {
    let class = classify_fraction(a.fraction);
    ExceptionItem {
        key: key.to_string(),
        run_id: Some(a.run_id.clone()),
        incident_id: None,
        kind: "budget".to_string(),
        class,
        severity: None,
        headline: format!("Run {} at {:.0}% of budget", a.run_id, a.fraction * 100.0),
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

fn incident_item(key: &str, inc: &Incident) -> ExceptionItem {
    let class = classify_incident(&inc.kind, inc.severity);
    let run_label = inc.run_id.clone().unwrap_or_else(|| inc.id.clone());
    ExceptionItem {
        key: key.to_string(),
        run_id: inc.run_id.clone(),
        incident_id: Some(inc.id.clone()),
        kind: inc.kind.clone(),
        class,
        severity: Some(severity_str(inc.severity).to_string()),
        headline: format!("Agent/run {run_label} running hot - {}", inc.kind),
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
    let apns_token = match registry.current_device() {
        Ok(Some(device)) => device.apns_token,
        Ok(None) => None,
        Err(e) => {
            eprintln!("genaryx-relay: exceptions: registry read failed, dropping push: {e}");
            None
        }
    };
    let Some(apns_token) = apns_token else {
        // Sim phase: no APNs registration path is wired (docs/PHASE5.md
        // defers real APNs to R1); the phone polls `/relay/v1/exceptions`
        // instead, so a push is a nice-to-have wake, never the source of
        // truth. Still log the would-be push for visibility.
        eprintln!(
            "genaryx-relay: would push (no APNs token on file): {} - {}",
            intent.title, intent.body
        );
        return;
    };
    push.send(crate::push::Notification {
        apns_token,
        title: intent.title,
        body: intent.body,
        run_id: intent.run_id,
        incident_id: intent.incident_id,
        kind: intent.kind,
    });
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
        let snap = eng.snapshot();
        assert_eq!(snap.queue.len(), 1);
        assert!(snap.queue[0].killed);
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
        let snap = eng.snapshot();
        assert_eq!(snap.queue.len(), 1, "same run key, no duplicate row");
        assert!(snap.queue[0].killed);
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
    fn snapshot_sorts_hard_first_then_most_recent() {
        let eng = engine();
        eng.handle_raw_record(&record(r#"{"type":"kill","run":"soft-first"}"#), 1000); // hard
        eng.handle_raw_record(
            &record(r#"{"type":"incident","id":"spike:r2","org":"acme","run_id":"r2","agent_id":null,"kind":"spend_spike","severity":"low","first_seen_millis":1000,"last_seen_millis":1000,"occurrences":1,"acknowledged":false,"last_notified_millis":0}"#),
            1001,
        ); // soft (at_risk)
        let snap = eng.snapshot();
        assert_eq!(snap.queue.len(), 2);
        assert!(is_hard(snap.queue[0].class), "hard item sorts first");
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
}
