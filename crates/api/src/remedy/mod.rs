//! Typed remedies: what an operator may do about an incident, each of a kind the
//! console can name, check, prove where a proof exists, and route to the command
//! that applies it.
//!
//! # WHY THIS EXISTS
//!
//! Until slice 5 a proposed action was a kind and a bag of JSON parameters, and
//! nothing said which proof applies to it, whether it fits the incident in front
//! of the operator, or how it would be applied. The four remedies here are the
//! ones the plan names (a policy candidate, a budget or step limit, stop an agent,
//! revoke a delegation); each is a value that can be refused before anything
//! runs, and each knows its proof and its route.
//!
//! # THREE RULES, HELD IN THE TYPES
//!
//! - **A remedy with no proof is still a remedy, and says it is unproven**
//!   (project rule 5). A stop or a revocation is an act on one subject, and no
//!   recorded history replays one, so its proof is [`NO_HISTORY_REPLAYS_AN_ACT`]
//!   and not a quiet absence.
//! - **Nothing is applied here** (project rule 6). [`ApplyRoute`] names the
//!   existing command a person runs through its ceremony, with the arguments
//!   filled in, or says that no console command does it. Nothing in this module
//!   calls one.
//! - **An identity refusal is filed under its key, and so are its remedies**
//!   (invariant 19). A refusal's claimed agent did not make the call, so stopping
//!   it or revoking its delegation would charge it with an attempt it did not
//!   make: an identity incident is offered no remedy of these four kinds.
//!
//! # THE CLASS OF AN INCIDENT
//!
//! [`classify`] reads the event type against one fixed table. The web console
//! routes by SOURCE because the registry of types grows more often than the
//! registry of sources; this table is the exception, and it is a small one:
//! only the types that mean something to a remedy are in it, and any other type is
//! [`IncidentClass::Unclassified`], never guessed. Beside each is the shipped
//! Mockryx scenario that rehearses the failure, if there is one.

pub mod commands;

use crate::delegation::stop::is_agent_id;
use crate::proof::budget::BudgetCandidate;
use crate::proof::commands::{ProofError, check_candidate};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Why a stop or a revocation has no proof.
pub const NO_HISTORY_REPLAYS_AN_ACT: &str = "a stop or a revocation is an act on one subject; \
                                             no recorded history replays one, so it is unproven";

/// Why an identity incident is offered no remedy.
pub const IDENTITY_IS_FILED_UNDER_THE_KEY: &str = "a refusal is filed under the key that made the \
                                                   call; stopping or revoking the agent it \
                                                   claimed would charge it with an attempt it \
                                                   did not make";

/// Why an incident of a type this console does not know is offered nothing.
pub const NOT_CLASSIFIED: &str =
    "the console does not know this type of incident, so it offers no remedy for it";

/// What kind of failure an incident is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidentClass {
    /// Spend past a budget.
    Spend,
    /// A run repeating itself.
    Loop,
    /// A policy refused or held a call.
    Policy,
    /// A call waits for, or was refused, a person's approval.
    Approval,
    /// Personal data or a secret in a prompt.
    Data,
    /// A call whose key does not match the identity it claimed.
    Identity,
    /// A model's quality drifted.
    Quality,
    /// A type the table does not hold.
    Unclassified,
}

struct Entry {
    event_type: &'static str,
    class: IncidentClass,
    /// The file name of the shipped Mockryx scenario that rehearses it.
    scenario: Option<&'static str>,
}

const fn entry(
    event_type: &'static str,
    class: IncidentClass,
    scenario: Option<&'static str>,
) -> Entry {
    Entry {
        event_type,
        class,
        scenario,
    }
}

const RUNAWAY: &str = "runaway-budget.yaml";
const DENIED_TOOL: &str = "wardryx-denied-tool.yaml";
const APPROVAL: &str = "approval-required.yaml";
const PII: &str = "pii-in-prompt.yaml";
const SECRET: &str = "dlp-secret-leak.yaml";

/// The types that mean something to a remedy. A scenario is named only where the
/// shipped one rehearses that failure: a refused key has none (the forged chain
/// of `on-behalf-of-forged-chain.yaml` is another failure), and a quality drift
/// has none (`verdryx-quality-drift.yaml` rehearses Verdryx noticing a denied
/// attempt, not a model regressing).
const TABLE: &[Entry] = &[
    entry("budget_exhausted", IncidentClass::Spend, Some(RUNAWAY)),
    entry("budget_exceeded", IncidentClass::Spend, Some(RUNAWAY)),
    entry("budget_threshold", IncidentClass::Spend, Some(RUNAWAY)),
    entry("loop_detected", IncidentClass::Loop, Some(RUNAWAY)),
    entry("policy_deny", IncidentClass::Policy, Some(DENIED_TOOL)),
    entry("policy_hold", IncidentClass::Approval, Some(APPROVAL)),
    entry("approval_requested", IncidentClass::Approval, Some(APPROVAL)),
    entry("approval_unanswered", IncidentClass::Approval, Some(APPROVAL)),
    entry("approval_denied", IncidentClass::Approval, Some(APPROVAL)),
    entry("dlp_pii", IncidentClass::Data, Some(PII)),
    entry("dlp_shadow", IncidentClass::Data, Some(PII)),
    entry("dlp_block", IncidentClass::Data, Some(SECRET)),
    entry("identity_mismatch", IncidentClass::Identity, None),
    entry("quality_drift", IncidentClass::Quality, None),
];

/// The class of an event type, and the Mockryx scenario that rehearses it.
pub fn classify(event_type: &str) -> (IncidentClass, Option<&'static str>) {
    let _ = event_type;
    (IncidentClass::Unclassified, None)
}

/// The kinds of remedy there are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemedyKind {
    PolicyCandidate,
    BudgetLimit,
    StopAgent,
    RevokeDelegation,
}

/// The kinds offered for an incident of `class`.
pub fn offered(class: IncidentClass) -> &'static [RemedyKind] {
    let _ = class;
    &[]
}

/// Why `class` is offered nothing, if it is not.
pub fn why_none(class: IncidentClass) -> Option<&'static str> {
    let _ = class;
    None
}

/// A remedy, as a person or Felyx names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Remedy {
    /// A policy the policy plane would load, as it reads one.
    PolicyCandidate { candidate: String },
    /// A budget per run, a budget per call, a step limit, or any of them.
    BudgetLimit {
        per_run_usd: Option<String>,
        per_step_usd: Option<String>,
        max_steps: Option<u32>,
    },
    /// Stop an agent: ban it at its issuer and kill its open runs.
    StopAgent { subject: String },
    /// Revoke the delegation of an agent or a user.
    RevokeDelegation { subject: String },
}

/// Where a remedy is applied, if it is.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "via", rename_all = "snake_case")]
pub enum ApplyRoute {
    /// An existing console command, run by a person through its ceremony. The
    /// arguments are filled in; `person_supplies` lists what only a person can.
    Command {
        command: &'static str,
        arguments: serde_json::Value,
        person_supplies: &'static [&'static str],
    },
    /// No console command does it; a person does it where `at` says.
    Outside { at: &'static str },
}

/// The commands a remedy can be routed to. The web crate holds that each is a
/// command the console has and that each needs the ceremony.
pub const ROUTED_COMMANDS: &[&str] = &["agent_stop", "delegation_revoke"];

impl Remedy {
    pub fn kind(&self) -> RemedyKind {
        match self {
            Remedy::PolicyCandidate { .. } => RemedyKind::PolicyCandidate,
            Remedy::BudgetLimit { .. } => RemedyKind::BudgetLimit,
            Remedy::StopAgent { .. } => RemedyKind::StopAgent,
            Remedy::RevokeDelegation { .. } => RemedyKind::RevokeDelegation,
        }
    }

    /// Whether the remedy is well formed. A remedy that is not is refused before
    /// anything is proved or routed.
    pub fn check(&self) -> Result<(), &'static str> {
    Ok(())
}

    /// How the remedy is applied.
    pub fn route(&self) -> ApplyRoute {
    ApplyRoute::Outside { at: "" }
}
}

fn is_user_id(s: &str) -> bool {
    s.strip_prefix("user://").is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .bytes()
                .all(|b| b.is_ascii_graphic() && b != b'\\' && b != b'"')
    })
}

#[cfg(test)]
mod tests;
