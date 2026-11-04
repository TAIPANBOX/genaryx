//! `remedy_check`: for one incident and one typed remedy, say whether the remedy
//! fits the incident, prove it where a proof exists, name the Mockryx scenario
//! that rehearses the failure, and name how it would be applied.
//!
//! A read in every sense that matters: it changes nothing in any plane, writes
//! no journal line, signs nothing and applies nothing (project rule 6). The
//! proofs it asks for are the ones of slices 3 and 4, with their own bounds and
//! the slots they share. A drill is named and never run: a Mockryx scenario is
//! a fire drill against the operator's own pre-production gateway, and starting
//! one is the Drills view's act, on an operator's click.

use super::{
    ApplyRoute, IncidentClass, NO_HISTORY_REPLAYS_AN_ACT, Remedy, RemedyKind, classify, offered,
    why_none,
};
use crate::bus::AppState;
use crate::case::Data;
use crate::proof::PolicyProof;
use crate::proof::budget::BudgetProof;
use crate::proof::budget_commands::{BUDGET_TIMEOUT, BudgetConfig, BudgetRequest, budget_proof_in};
use crate::proof::commands::{
    PROOF_SLOTS, ProofConfig, ProofError, ProofRequest, REPLAY_TIMEOUT, event_of, policy_proof_in,
};
use crate::proof::slots::Slots;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// A request to check a remedy against an incident.
#[derive(Debug, Clone)]
pub struct RemedyRequest {
    /// The incident Incident 360 holds (`bus:<n>`, `verdryx:<n>` or `<n>`).
    pub incident_id: String,
    pub remedy: Remedy,
}

/// Why no check was made. A check that could not be made is never a check that
/// found nothing wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemedyError {
    /// The remedy is not well formed.
    Refused { reason: &'static str },
    /// The incident could not be resolved to an event row.
    Incident { reason: String },
}

/// What was done to prove the remedy, if anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProofOutcome {
    /// `wardryx replay` over the recorded decisions.
    Policy { proof: PolicyProof },
    /// `tokenfuse backtest` over the recorded calls.
    Budget { proof: BudgetProof },
    /// No proof exists for this kind of remedy. It can still be applied by a
    /// person, and the case says it was unproven (project rule 5).
    Unproven { reason: &'static str },
    /// A proof was asked for and could not be made, with the cause.
    Failed { error: ProofError },
}

/// A Mockryx scenario that rehearses the failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DrillSuggestion {
    /// The scenario's file name in the shipped scenarios.
    pub scenario: &'static str,
    /// Whether that file is in the scenario directory this box runs drills from.
    /// `None` when the box names no scenario directory.
    pub in_scenario_dir: Option<bool>,
}

/// The check of one remedy against one incident.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RemedyCheck {
    pub incident_id: String,
    /// The incident's event type. A producer's text.
    pub event_type: Data,
    pub class: IncidentClass,
    /// The kinds of remedy offered for the class.
    pub offered: &'static [RemedyKind],
    /// Why nothing is offered, when nothing is.
    pub why_none: Option<&'static str>,
    pub remedy: Remedy,
    /// Whether the remedy's kind is one of those offered.
    pub fits_class: bool,
    pub proof: ProofOutcome,
    pub drill: Option<DrillSuggestion>,
    /// How a person would apply it. Nothing here does.
    pub apply: ApplyRoute,
}

fn incident_error(error: ProofError) -> RemedyError {
    match error {
        ProofError::Incident { reason } => RemedyError::Incident { reason },
        other => RemedyError::Incident {
            reason: format!("{other:?}"),
        },
    }
}

fn drill_for(scenario: Option<&'static str>, dir: Option<&Path>) -> Option<DrillSuggestion> {
    scenario.map(|scenario| DrillSuggestion {
        scenario,
        in_scenario_dir: dir.map(|d| d.join(scenario).is_file()),
    })
}

/// The proof the kind of remedy has, made, or the statement that it has none.
async fn prove(
    slots: &Slots,
    policy: ProofConfig,
    budget: BudgetConfig,
    request: &RemedyRequest,
    bus: &AppState,
) -> ProofOutcome {
    match &request.remedy {
        Remedy::PolicyCandidate { candidate } => {
            let asked = ProofRequest {
                incident_id: request.incident_id.clone(),
                candidate: candidate.clone(),
            };
            match policy_proof_in(slots, policy, asked, bus, REPLAY_TIMEOUT).await {
                Ok(proof) => ProofOutcome::Policy { proof },
                Err(error) => ProofOutcome::Failed { error },
            }
        }
        Remedy::BudgetLimit {
            per_run_usd,
            per_step_usd,
            max_steps,
        } => {
            let asked = BudgetRequest {
                incident_id: request.incident_id.clone(),
                per_run_usd: per_run_usd.clone(),
                per_step_usd: per_step_usd.clone(),
                max_steps: *max_steps,
            };
            match budget_proof_in(slots, budget, asked, bus, BUDGET_TIMEOUT).await {
                Ok(proof) => ProofOutcome::Budget { proof },
                Err(error) => ProofOutcome::Failed { error },
            }
        }
        Remedy::StopAgent { .. } | Remedy::RevokeDelegation { .. } => ProofOutcome::Unproven {
            reason: NO_HISTORY_REPLAYS_AN_ACT,
        },
    }
}

/// The command: check a remedy against an incident, on this box.
pub async fn remedy_check(
    request: RemedyRequest,
    bus: &AppState,
    scenario_dir: Option<PathBuf>,
) -> Result<RemedyCheck, RemedyError> {
    remedy_check_in(
        &PROOF_SLOTS,
        ProofConfig::from_env(bus),
        BudgetConfig::from_env(),
        request,
        bus,
        scenario_dir,
    )
    .await
}

/// [`remedy_check`] with its slots and the two proofs' configurations given.
pub async fn remedy_check_in(
    slots: &Slots,
    policy: ProofConfig,
    budget: BudgetConfig,
    request: RemedyRequest,
    bus: &AppState,
    scenario_dir: Option<PathBuf>,
) -> Result<RemedyCheck, RemedyError> {
    request
        .remedy
        .check()
        .map_err(|reason| RemedyError::Refused { reason })?;
    let event = event_of(bus, &request.incident_id).map_err(incident_error)?;
    let (class, scenario) = classify(&event.type_);
    let fits_class = offered(class).contains(&request.remedy.kind());
    let proof = prove(slots, policy, budget, &request, bus).await;
    Ok(RemedyCheck {
        incident_id: request.incident_id.clone(),
        event_type: Data::bounded(&event.type_),
        class,
        offered: offered(class),
        why_none: why_none(class),
        fits_class,
        proof,
        drill: drill_for(scenario, scenario_dir.as_deref()),
        apply: request.remedy.route(),
        remedy: request.remedy,
    })
}
