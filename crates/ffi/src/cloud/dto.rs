//! Wire DTOs and error taxonomy for [`super::CloudHandle`], mirroring
//! `apps/desktop/src-tauri/src/money/commands.rs`'s `OverviewDto`/`RunDto`/
//! `IncidentDto`/`SavingsDto`/`MutationOutcome`/`MoneyError` field-for-field
//! (docs/PHASE1.md wave 3, "both shells behave identically") but as UniFFI
//! `Record`/`Error` types instead of Tauri-IPC `Serialize` structs.
//!
//! `genaryx_connectors::Incident`/`Severity` are imported under aliases: this
//! module defines its own `Incident` (the UI-facing DTO) and represents
//! severity as the same raw lowercase `String` the Bus Explorer's `UiEvent`
//! already uses (`Theme.severityColor(_ severity: String?)` in the Swift
//! shell already knows how to render it), so no second severity enum needs
//! to cross the FFI boundary at all.

use genaryx_connectors::{
    ConnectorError, Incident as ApiIncident, RunAgg, SavingsSummary, Severity as ApiSeverity,
    Summary,
};

// ============================================================================
// DTOs
// ============================================================================

/// Tiles for the Overview view. Mirrors `money::commands::OverviewDto`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Overview {
    pub total_spent_usd: f64,
    pub total_calls: u64,
    /// All-time run count (`GET /v1/summary`'s own `runs` field).
    pub total_runs: u64,
    /// Not-killed runs, out of the `GET /v1/runs` list (a live snapshot).
    pub active_runs: u64,
    pub killed_runs: u64,
    /// Unacknowledged incidents - the actionable count for the tile.
    pub open_incidents: u64,
    pub total_incidents: u64,
    pub total_saved_usd: f64,
}

impl Overview {
    pub(super) fn build(
        summary: &Summary,
        runs: &[RunAgg],
        incidents: &[ApiIncident],
        savings: &SavingsSummary,
    ) -> Self {
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

/// One row of the Money view's runs table. Mirrors `money::commands::RunDto`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Run {
    pub run_id: String,
    pub model: String,
    pub agent_id: String,
    pub spent_usd: f64,
    /// `None` when this run has neither tripped `/v1/alerts`' threshold nor
    /// had its budget set via this session (see `CloudHandle::budget_overrides`).
    pub budget_usd: Option<f64>,
    pub calls: u64,
    pub cache_hits: u64,
    pub steps: u32,
    pub last_seen: String,
    pub killed: bool,
}

/// One row of the Money view's incidents list. Mirrors `money::commands::IncidentDto`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Incident {
    pub id: String,
    pub run_id: Option<String>,
    pub agent_id: Option<String>,
    pub kind: String,
    /// Lowercase severity string (`"info"`.."critical"`), the same tolerant
    /// convention `UiEvent.severity` already uses.
    pub severity: String,
    pub first_seen: String,
    pub last_seen: String,
    pub occurrences: u64,
    pub acknowledged: bool,
}

impl From<&ApiIncident> for Incident {
    fn from(i: &ApiIncident) -> Self {
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

/// The Money view's savings breakdown. Mirrors `money::commands::SavingsDto`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Savings {
    pub blocked_spend_usd: f64,
    pub cache_saved_usd: f64,
    pub router_saved_usd: f64,
    pub budget_breaks: u64,
    pub total_saved_usd: f64,
}

impl From<&SavingsSummary> for Savings {
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

/// What a successful mutation returns: the Cloud's own verdict plus whether
/// it also made it onto the local bus as a `console_command`. Mirrors
/// `money::commands::MutationOutcome`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct MutationOutcome {
    pub summary: String,
    pub http_status: u16,
    pub verify_result: String,
    pub sig_alg: String,
    pub sig_fpr: String,
    /// Whether `genaryx_core::command::record` succeeded, i.e. whether a
    /// `console_command` line was appended to the events file.
    pub bus_recorded: bool,
    pub bus_error: Option<String>,
}

// ============================================================================
// error taxonomy
// ============================================================================

/// Every failure mode a [`super::CloudHandle`] call can surface, fail-closed
/// throughout (06 §0.5: no panics/unwraps cross the FFI boundary). Mirrors
/// `money::commands::MoneyError`, minus the Tauri-only `Bootstrapping`
/// variant: `CloudHandle`'s constructors are synchronous, so "not ready yet"
/// collapses into the constructor's own `Result` instead of a separate state.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CloudError {
    /// [`super::env::discover`] found nothing usable: no `taipan up`
    /// descriptor and no `TOKENFUSE_CLOUD_ADMIN_KEY`.
    #[error(
        "no TokenFuse Cloud environment found (no taipan up descriptor, no TOKENFUSE_CLOUD_ADMIN_KEY)"
    )]
    NoEnvironment,
    /// An environment resolved (or was given explicitly via `connect`), but
    /// building the client, pairing, or attaching the device failed.
    #[error("pairing failed: {reason}")]
    PairingFailed { reason: String },
    /// `402 plan_required` - kept structurally distinct (never folded into
    /// `Cloud`'s free-text `message`) so the Swift shell can render an
    /// upsell tile instead of an error banner.
    #[error("plan required: feature={feature} org={org} (upgrade: {upgrade_url})")]
    PlanRequired {
        feature: String,
        org: String,
        upgrade_url: String,
    },
    /// Any other Cloud-side failure: transport, signature rejection, a
    /// plain non-2xx, or a response that failed to parse. `status` is
    /// `None` when the request never got far enough to have one.
    #[error("cloud error (status {status:?}): {message}")]
    Cloud {
        status: Option<u16>,
        message: String,
    },
}

impl From<ConnectorError> for CloudError {
    fn from(e: ConnectorError) -> Self {
        match e {
            ConnectorError::PlanRequired {
                feature,
                org,
                upgrade_url,
            } => CloudError::PlanRequired {
                feature,
                org,
                upgrade_url,
            },
            ConnectorError::SignatureRejected => CloudError::Cloud {
                status: Some(403),
                message: "device signature rejected by the Cloud (signature_invalid)".to_string(),
            },
            ConnectorError::Api { status, body } => CloudError::Cloud {
                status: Some(status),
                message: body,
            },
            ConnectorError::NoDeviceSigner => CloudError::Cloud {
                status: None,
                message: "no paired device signer attached (internal state error)".to_string(),
            },
            ConnectorError::Signing(err) => CloudError::Cloud {
                status: None,
                message: format!("signing failed: {err}"),
            },
            ConnectorError::Transport(err) => CloudError::Cloud {
                status: None,
                message: format!("could not reach the Cloud: {err}"),
            },
            ConnectorError::Json(err) => CloudError::Cloud {
                status: None,
                message: format!("unexpected response shape from the Cloud: {err}"),
            },
        }
    }
}

// ============================================================================
// helpers
// ============================================================================

pub(super) fn micros_to_usd(micros: i64) -> f64 {
    micros as f64 / 1_000_000.0
}

pub(super) fn millis_to_iso(millis: i64) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_else(|| millis.to_string())
}

fn severity_str(s: ApiSeverity) -> &'static str {
    match s {
        ApiSeverity::Info => "info",
        ApiSeverity::Low => "low",
        ApiSeverity::Medium => "medium",
        ApiSeverity::High => "high",
        ApiSeverity::Critical => "critical",
    }
}

/// The HTTP status implied by a failed connector call, `0` when the request
/// never reached a point where one exists (never fabricated).
pub(super) fn status_of(e: &ConnectorError) -> u16 {
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

/// Map a run agg + budget lookup into a [`Run`] row - shared by
/// `CloudHandle::runs`'s mapping closure.
pub(super) fn build_run(r: &RunAgg, budget_micros: Option<i64>) -> Run {
    Run {
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
}
