//! Tauri commands for the Overview + Money views: four reads
//! (`money_overview`/`money_runs`/`money_incidents`/`money_savings`) and
//! three signed mutations (`money_kill_run`/`money_set_budget`/
//! `money_ack_incident`), plus `money_status` so the frontend can render a
//! clean "no environment" / "pairing failed" state up front instead of
//! guessing from a read command's error shape.
//!
//! Every DTO here is a UI-facing serde mirror of a `genaryx_connectors` type
//! (same convention `src-tauri/src/events.rs`'s `UiEvent` already uses for
//! `genaryx_core::store::StoredEvent`): the connector's own DTOs only derive
//! `Deserialize` (they exist to parse the Cloud's responses), not
//! `Serialize`, so they cannot cross the Tauri IPC boundary as-is.
//!
//! Fail-closed mutation contract: every mutation calls the connector first,
//! then ALWAYS attempts `genaryx_core::command::record` - even when the
//! Cloud call itself failed or was rejected, since a rejected privileged
//! attempt is itself part of the audit trail (06 §0.4, "the console is
//! itself an agent of the stack"). The two outcomes are reported back
//! separately and honestly: the Cloud's verdict decides `Ok`/`Err` for the
//! frontend, while `MutationOutcome::bus_recorded`/`bus_error` says whether
//! the local journal write succeeded, never conflating the two.
//!
//! Break-glass ceremony (Phase-2 wave 3B): `money_kill_run` and
//! `money_set_budget` are the two genuinely-privileged mutations - they
//! change what the Cloud is doing (stopping a run, moving a spend ceiling)
//! with no Wardryx precheck in front of them yet, so each now takes a
//! mandatory `reason: String` argument, journals `decision: "break_glass"`,
//! and carries the reason in `params["reason"]` (never in the emitted bus
//! event's fixed-shape `data`, see [`console_command_line`]'s doc). A blank
//! reason is refused by [`require_break_glass_reason`] before the Cloud is
//! ever called - a front-line copy of the same rule
//! `genaryx_core::command::record` itself enforces
//! (`Error::BreakGlassMissingReason`), so an unjustified override cannot
//! reach the Cloud even if a caller bypassed the frontend's own disabled-
//! until-non-empty confirm button. `money_ack_incident` carries no reason
//! and journals `decision: "allow"`: acknowledging an incident someone else
//! already raised is not itself an operator override of governance.

use super::env::EnvSource;
use super::state::{MoneyClient, MoneyInner, MoneyState};
use genaryx_core::{CommandRecord, command};
use genaryx_connectors::{ConnectorError, Incident, RunAgg, SavingsSummary, Severity, Summary};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;

// ============================================================================
// DTOs
// ============================================================================

/// Tiles for the Overview view.
#[derive(Debug, Clone, Serialize)]
pub struct OverviewDto {
    pub total_spent_usd: f64,
    pub total_calls: u64,
    /// All-time run count (`GET /v1/summary`'s own `runs` field - "exact
    /// across the org's whole ingest history" per `cloud_rest.rs`'s doc).
    pub total_runs: u64,
    /// Not-killed runs, out of the `GET /v1/runs` list (a live snapshot, not
    /// necessarily identical to `total_runs` if the two calls race a
    /// concurrent write - both come from the same overview fetch so any
    /// drift is at most one event wide).
    pub active_runs: u64,
    pub killed_runs: u64,
    /// Unacknowledged incidents - the actionable count for the tile.
    /// `GET /v1/incidents` returns every incident regardless of ack state
    /// (confirmed against `tokenfuse-cloud`'s `store.rs::incidents`, which
    /// does not filter), so this filters client-side.
    pub open_incidents: u64,
    pub total_incidents: u64,
    pub total_saved_usd: f64,
}

impl OverviewDto {
    fn build(summary: &Summary, runs: &[RunAgg], incidents: &[Incident], savings: &SavingsSummary) -> Self {
        let killed_runs = runs.iter().filter(|r| r.killed).count() as u64;
        let open_incidents = incidents.iter().filter(|i| !i.acknowledged).count() as u64;
        Self {
            total_spent_usd: micros_to_usd(summary.spent_microusd),
            total_calls: summary.calls,
            total_runs: summary.runs,
            active_runs: (runs.len() as u64).saturating_sub(killed_runs),
            killed_runs,
            open_incidents,
            total_incidents: incidents.len() as u64,
            total_saved_usd: micros_to_usd(savings.total_saved_microusd),
        }
    }
}

/// One row of the Money view's runs table.
#[derive(Debug, Clone, Serialize)]
pub struct RunDto {
    pub run_id: String,
    pub model: String,
    pub agent_id: String,
    pub spent_usd: f64,
    /// `None` when this run has neither tripped `/v1/alerts`' threshold nor
    /// had its budget set via this console session - see
    /// [`MoneyState::budget_overrides`] for why a budget is not always
    /// knowable from the connector's current read surface.
    pub budget_usd: Option<f64>,
    pub calls: u64,
    pub cache_hits: u64,
    pub steps: u32,
    pub last_seen: String,
    pub killed: bool,
}

/// One row of the Money view's incidents list.
#[derive(Debug, Clone, Serialize)]
pub struct IncidentDto {
    pub id: String,
    pub run_id: Option<String>,
    pub agent_id: Option<String>,
    pub kind: String,
    /// Lowercase severity string (`"info"`.."critical"`), matching the
    /// frontend's existing `Severity`/`SeverityBadge` from the Bus Explorer
    /// (`src/types.ts`) so both surfaces render severity identically.
    pub severity: String,
    pub first_seen: String,
    pub last_seen: String,
    pub occurrences: u64,
    pub acknowledged: bool,
}

impl From<&Incident> for IncidentDto {
    fn from(i: &Incident) -> Self {
        Self {
            id: i.id.clone(),
            run_id: i.run_id.clone(),
            agent_id: i.agent_id.clone(),
            kind: i.kind.clone(),
            severity: severity_str(i.severity).to_string(),
            first_seen: millis_to_iso(i.first_seen_millis),
            last_seen: millis_to_iso(i.last_seen_millis),
            occurrences: i.occurrences,
            acknowledged: i.acknowledged,
        }
    }
}

/// The Money view's savings breakdown.
#[derive(Debug, Clone, Serialize)]
pub struct SavingsDto {
    pub blocked_spend_usd: f64,
    pub cache_saved_usd: f64,
    pub router_saved_usd: f64,
    pub budget_breaks: u64,
    pub total_saved_usd: f64,
}

impl From<&SavingsSummary> for SavingsDto {
    fn from(s: &SavingsSummary) -> Self {
        Self {
            blocked_spend_usd: micros_to_usd(s.blocked_spend_microusd),
            cache_saved_usd: micros_to_usd(s.cache_saved_microusd),
            router_saved_usd: micros_to_usd(s.router_saved_microusd),
            budget_breaks: s.budget_breaks,
            total_saved_usd: micros_to_usd(s.total_saved_microusd),
        }
    }
}

/// Whole-panel connection state, for the frontend to render up front (never
/// inferred from a read command's error shape).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MoneyStatusDto {
    /// Discovery/pairing is still running in the background (see
    /// `state.rs`'s module docs) - normal for the first moment after
    /// startup, never a stuck state on its own.
    Bootstrapping,
    NoEnvironment,
    PairingFailed { source: EnvSource, cloud_url: String, reason: String },
    Ready { source: EnvSource, cloud_url: String, org_domain: String },
}

/// Every error a money command can return. `PlanRequired` is kept
/// structurally distinct (never folded into `Cloud`'s free-text `message`)
/// specifically so the frontend can render an upsell tile instead of an
/// error toast, per this task's spec.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MoneyError {
    /// Discovery/pairing has not finished yet; retry shortly. Distinct from
    /// `NoEnvironment` so the frontend can show "connecting..." rather than
    /// "not configured" in the first moment after startup.
    Bootstrapping,
    NoEnvironment,
    PairingFailed { reason: String },
    PlanRequired { feature: String, org: String, upgrade_url: String },
    /// `money_kill_run`/`money_set_budget` was called with an empty or
    /// whitespace-only `reason` (Phase-2 wave 3B). Kept structurally
    /// distinct from `Cloud` rather than folded into its free-text
    /// `message`, same reasoning as `PlanRequired`: this never reached the
    /// Cloud at all, so reporting it as a "Cloud-side failure" would be
    /// dishonest about where the refusal actually happened. In normal use
    /// the frontend's own confirm ceremony disables the button until a
    /// reason is typed, so this is a fail-closed backstop, not the expected
    /// path.
    BreakGlassMissingReason,
    /// Any other Cloud-side failure: transport, signature rejection, a
    /// plain non-2xx, or a response that failed to parse. `status` is
    /// `None` when the request never got far enough to have one (couldn't
    /// sign, couldn't connect, couldn't parse a body).
    Cloud { status: Option<u16>, message: String },
}

impl From<ConnectorError> for MoneyError {
    fn from(e: ConnectorError) -> Self {
        match e {
            ConnectorError::PlanRequired { feature, org, upgrade_url } => {
                MoneyError::PlanRequired { feature, org, upgrade_url }
            }
            ConnectorError::SignatureRejected => MoneyError::Cloud {
                status: Some(403),
                message: "device signature rejected by the Cloud (signature_invalid)".to_string(),
            },
            ConnectorError::Api { status, body } => MoneyError::Cloud { status: Some(status), message: body },
            ConnectorError::NoDeviceSigner => MoneyError::Cloud {
                status: None,
                message: "no paired device signer attached (internal state error)".to_string(),
            },
            ConnectorError::Signing(err) => {
                MoneyError::Cloud { status: None, message: format!("signing failed: {err}") }
            }
            ConnectorError::Transport(err) => {
                MoneyError::Cloud { status: None, message: format!("could not reach the Cloud: {err}") }
            }
            ConnectorError::Json(err) => MoneyError::Cloud {
                status: None,
                message: format!("unexpected response shape from the Cloud: {err}"),
            },
        }
    }
}

/// What a successful mutation returns: the Cloud's own verdict plus whether
/// it also made it onto the local bus as a `console_command`.
#[derive(Debug, Clone, Serialize)]
pub struct MutationOutcome {
    pub summary: String,
    pub http_status: u16,
    pub verify_result: String,
    pub sig_alg: String,
    pub sig_fpr: String,
    /// Whether `genaryx_core::command::record` succeeded, i.e. whether a
    /// `console_command` line was appended to the live-wire bus the Bus
    /// Explorer tails.
    pub bus_recorded: bool,
    pub bus_error: Option<String>,
}

// ============================================================================
// helpers
// ============================================================================

fn micros_to_usd(micros: i64) -> f64 {
    micros as f64 / 1_000_000.0
}

fn millis_to_iso(millis: i64) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_else(|| millis.to_string())
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

/// The HTTP status implied by a failed connector call, `0` when the request
/// never reached a point where one exists (never fabricated - see
/// [`CommandRecord::http_status`]'s doc for why an honest `0` beats a
/// made-up `500`).
fn status_of(e: &ConnectorError) -> u16 {
    match e {
        ConnectorError::Api { status, .. } => *status,
        ConnectorError::SignatureRejected => 403,
        ConnectorError::PlanRequired { .. } => 402,
        ConnectorError::NoDeviceSigner
        | ConnectorError::Signing(_)
        | ConnectorError::Transport(_)
        | ConnectorError::Json(_) => 0,
    }
}

/// Fail-closed front-line guard for the two break-glass mutations (06 §0.5;
/// Phase-2 wave 3B): refuse before the Cloud is ever called if `reason` is
/// empty or whitespace-only. `genaryx_core::command::record` enforces the
/// same rule again just before journaling (`require_break_glass_reason` in
/// `crates/core/src/command.rs`, not `pub`, so this is a deliberate,
/// independent copy rather than a shared call) - that is the authority this
/// shell must never bypass, this is the shell's own earlier refusal so an
/// unjustified override does not even reach the network.
fn require_break_glass_reason(reason: &str) -> Result<(), MoneyError> {
    if reason.trim().is_empty() {
        return Err(MoneyError::BreakGlassMissingReason);
    }
    Ok(())
}

/// Resolve the current [`MoneyClient`] out of managed state, or the
/// appropriate [`MoneyError`] when the panel is not ready. Only holds the
/// state lock long enough to clone the (cheap, `Arc`-backed) client out -
/// the caller's actual Cloud HTTP call happens after this returns, never
/// while holding the lock (see `state.rs`'s module docs).
async fn ready_client(state: &tauri::State<'_, MoneyState>) -> Result<MoneyClient, MoneyError> {
    let guard = state.inner.lock().await;
    match &*guard {
        MoneyInner::Ready(client) => Ok(client.clone()),
        MoneyInner::Bootstrapping => Err(MoneyError::Bootstrapping),
        MoneyInner::NoEnvironment => Err(MoneyError::NoEnvironment),
        MoneyInner::PairingFailed { reason, .. } => {
            Err(MoneyError::PairingFailed { reason: reason.clone() })
        }
    }
}

/// Journal one `CommandRecord` (best-effort: a journal failure is reported,
/// never panics and never blocks the caller from learning the Cloud's own
/// verdict). `None` bus (Phase-0 live-wire seeding failed) is reported the
/// same honest way as a journal I/O error.
fn journal(client: &MoneyClient, rec: &CommandRecord) -> (bool, Option<String>) {
    let Some(bus) = &client.bus else {
        return (
            false,
            Some("no live event bus available (startup seeding did not complete)".to_string()),
        );
    };
    match genaryx_core::store::Store::open(&bus.store_db_path) {
        Ok(store) => {
            match command::record(&store, &bus.console_events_path, &client.org_domain, &client.host, rec) {
                Ok(()) => (true, None),
                Err(e) => (false, Some(e.to_string())),
            }
        }
        Err(e) => (false, Some(e.to_string())),
    }
}

/// Shared tail end of every mutation command: build the `CommandRecord` from
/// the already-resolved Cloud outcome, always attempt to journal it
/// (regardless of that outcome), then fold everything into either a
/// [`MutationOutcome`] or a [`MoneyError`] for the frontend.
///
/// `decision` is caller-supplied rather than hardcoded (Phase-2 wave 3B):
/// no Wardryx precheck exists yet in this build (CommandBroker's
/// policy-decide step, docs/PHASE1.md wave 2, is separate/unbuilt), but that
/// does not make every mutation this panel issues an operator override.
/// `money_kill_run`/`money_set_budget` genuinely are - they change Cloud
/// state with nothing else gating them - so they pass `"break_glass"` (and
/// must have already put a justification in `params["reason"]`, see
/// `require_break_glass_reason`). `money_ack_incident` passes `"allow"`:
/// acknowledging an already-raised incident overrides nothing.
fn finish_mutation<T>(
    client: &MoneyClient,
    action: &'static str,
    target: &str,
    params: Value,
    decision: &'static str,
    cloud_result: Result<T, ConnectorError>,
    on_ok: impl FnOnce(&T) -> (String, String),
) -> Result<MutationOutcome, MoneyError> {
    let (http_status, verify_result, summary) = match &cloud_result {
        Ok(value) => {
            let (summary, verify_result) = on_ok(value);
            (200u16, verify_result, summary)
        }
        Err(e) => (status_of(e), format!("error: {e}"), String::new()),
    };

    let rec = CommandRecord {
        operator: client.operator.clone(),
        env: "local".to_string(),
        action: action.to_string(),
        target: target.to_string(),
        params,
        decision: decision.to_string(),
        sig_alg: "es256".to_string(),
        sig_fpr: client.sig_fpr.to_string(),
        http_status,
        verify_result: verify_result.clone(),
    };
    let (bus_recorded, bus_error) = journal(client, &rec);

    match cloud_result {
        Ok(_) => Ok(MutationOutcome {
            summary,
            http_status,
            verify_result,
            sig_alg: "es256".to_string(),
            sig_fpr: client.sig_fpr.to_string(),
            bus_recorded,
            bus_error,
        }),
        Err(e) => Err(MoneyError::from(e)),
    }
}

// ============================================================================
// commands: status + reads
// ============================================================================

/// Whole-panel connection state. Never fails: every outcome of
/// [`super::state::bootstrap`] is a renderable [`MoneyStatusDto`] variant.
#[tauri::command]
pub async fn money_status(state: tauri::State<'_, MoneyState>) -> Result<MoneyStatusDto, ()> {
    let guard = state.inner.lock().await;
    Ok(match &*guard {
        MoneyInner::Bootstrapping => MoneyStatusDto::Bootstrapping,
        MoneyInner::NoEnvironment => MoneyStatusDto::NoEnvironment,
        MoneyInner::PairingFailed { source, cloud_url, reason } => MoneyStatusDto::PairingFailed {
            source: source.clone(),
            cloud_url: cloud_url.clone(),
            reason: reason.clone(),
        },
        MoneyInner::Ready(client) => MoneyStatusDto::Ready {
            source: client.source.clone(),
            cloud_url: client.cloud_url.clone(),
            org_domain: client.org_domain.clone(),
        },
    })
}

/// Summary + a few derived tiles (active runs, open incidents, total
/// saved) - one round trip from the frontend's perspective, four concurrent
/// Cloud reads underneath.
#[tauri::command]
pub async fn money_overview(state: tauri::State<'_, MoneyState>) -> Result<OverviewDto, MoneyError> {
    let client = ready_client(&state).await?;
    let (summary, runs, incidents, savings) = tokio::try_join!(
        client.client.summary(),
        client.client.runs(),
        client.client.incidents(),
        client.client.savings(),
    )
    .map_err(MoneyError::from)?;
    Ok(OverviewDto::build(&summary, &runs, &incidents, &savings))
}

/// The runs table. Budget is enriched from `GET /v1/alerts` (the only
/// connector read that carries `budget_micros`) overlaid with any budget
/// this console session itself has set - see
/// [`MoneyState::budget_overrides`].
#[tauri::command]
pub async fn money_runs(state: tauri::State<'_, MoneyState>) -> Result<Vec<RunDto>, MoneyError> {
    let client = ready_client(&state).await?;
    let (runs, alerts) = tokio::try_join!(client.client.runs(), client.client.alerts())
        .map_err(MoneyError::from)?;

    let alert_budgets: HashMap<&str, i64> =
        alerts.iter().map(|a| (a.run_id.as_str(), a.budget_micros)).collect();
    let overrides = state.budget_overrides.lock().await;

    Ok(runs
        .iter()
        .map(|r| {
            let budget_micros = overrides
                .get(&r.run_id)
                .copied()
                .or_else(|| alert_budgets.get(r.run_id.as_str()).copied());
            RunDto {
                run_id: r.run_id.clone(),
                model: r.model.clone(),
                agent_id: r.agent_id.clone(),
                spent_usd: micros_to_usd(r.spent_microusd),
                budget_usd: budget_micros.map(micros_to_usd),
                calls: r.calls,
                cache_hits: r.cache_hits,
                steps: r.steps,
                last_seen: millis_to_iso(r.last_seen_millis),
                killed: r.killed,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn money_incidents(state: tauri::State<'_, MoneyState>) -> Result<Vec<IncidentDto>, MoneyError> {
    let client = ready_client(&state).await?;
    let incidents = client.client.incidents().await.map_err(MoneyError::from)?;
    Ok(incidents.iter().map(IncidentDto::from).collect())
}

#[tauri::command]
pub async fn money_savings(state: tauri::State<'_, MoneyState>) -> Result<SavingsDto, MoneyError> {
    let client = ready_client(&state).await?;
    let savings = client.client.savings().await.map_err(MoneyError::from)?;
    Ok(SavingsDto::from(&savings))
}

// ============================================================================
// commands: signed mutations
// ============================================================================
// `rename_all = "snake_case"` on every mutation below: Tauri's default
// argument case is camelCase (converted from the Rust parameter name), but
// every other wire shape in this app (`UiEvent`, and every DTO above) is
// snake_case, matching the core's own convention. Pinning the argument case
// keeps the whole IPC surface consistently snake_case instead of mixing
// conventions between args and return values.

/// Kill a run: a break-glass operator override (Phase-2 wave 3B) - `reason`
/// is mandatory (checked before the Cloud is ever called, see
/// [`require_break_glass_reason`]) and rides in the journaled
/// `CommandRecord`'s `params["reason"]`, never in the emitted bus event.
#[tauri::command(rename_all = "snake_case")]
pub async fn money_kill_run(
    run_id: String,
    reason: String,
    state: tauri::State<'_, MoneyState>,
) -> Result<MutationOutcome, MoneyError> {
    require_break_glass_reason(&reason)?;
    let client = ready_client(&state).await?;
    let result = client.client.kill_run(&run_id).await;
    finish_mutation(
        &client,
        "console.kill_run",
        &run_id,
        json!({ "reason": reason }),
        "break_glass",
        result,
        |resp| {
            (
                format!("run {run_id} killed"),
                format!("killed:{}", resp.killed == run_id),
            )
        },
    )
}

/// Set a run's budget: a break-glass operator override (Phase-2 wave 3B) -
/// same mandatory-`reason` contract as [`money_kill_run`], with the amount
/// alongside it in `params` (`{"reason": ..., "budget_usd": ...}`).
#[tauri::command(rename_all = "snake_case")]
pub async fn money_set_budget(
    run_id: String,
    budget_usd: f64,
    reason: String,
    state: tauri::State<'_, MoneyState>,
) -> Result<MutationOutcome, MoneyError> {
    require_break_glass_reason(&reason)?;
    let client = ready_client(&state).await?;
    let result = client.client.set_budget(&run_id, budget_usd).await;

    if let Ok(resp) = &result {
        let mut overrides = state.budget_overrides.lock().await;
        overrides.insert(run_id.clone(), resp.budget_micros);
    }

    finish_mutation(
        &client,
        "console.set_budget",
        &run_id,
        json!({ "reason": reason, "budget_usd": budget_usd }),
        "break_glass",
        result,
        |resp| {
            (
                format!("run {run_id} budget set to ${budget_usd:.4}"),
                format!("budget_micros:{}", resp.budget_micros),
            )
        },
    )
}

/// Acknowledge an incident: NOT a break-glass override (Phase-2 wave 3B) -
/// no reason is collected or required, and the journaled `decision` is
/// `"allow"` rather than `"break_glass"`, since marking an already-raised
/// incident seen overrides no governance decision.
#[tauri::command(rename_all = "snake_case")]
pub async fn money_ack_incident(
    id: String,
    state: tauri::State<'_, MoneyState>,
) -> Result<MutationOutcome, MoneyError> {
    let client = ready_client(&state).await?;
    let result = client.client.ack_incident(&id).await;
    finish_mutation(
        &client,
        "console.ack_incident",
        &id,
        json!({}),
        "allow",
        result,
        |resp| {
            (
                format!("incident {id} acknowledged"),
                format!("acknowledged:{}", resp.acknowledged == id),
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // `require_break_glass_reason` is `money_kill_run`/`money_set_budget`'s
    // own front-line copy of `genaryx_core::command::require_break_glass_reason`
    // (crates/core, not `pub`, so it cannot be called directly from here) -
    // same three cases that module's own test covers, kept in sync by hand.

    #[test]
    fn empty_reason_is_refused() {
        assert!(matches!(
            require_break_glass_reason(""),
            Err(MoneyError::BreakGlassMissingReason)
        ));
    }

    #[test]
    fn whitespace_only_reason_is_refused() {
        assert!(matches!(
            require_break_glass_reason("   \n\t"),
            Err(MoneyError::BreakGlassMissingReason)
        ));
    }

    #[test]
    fn a_real_reason_passes() {
        assert!(require_break_glass_reason("runaway spend, operator override").is_ok());
    }
}
