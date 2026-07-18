//! The `Propose` tier of the action model (docs/PHASE6.md, itrat-console/13
//! D13.3). A [`ProposedAction`] is the ONLY thing a "propose" tool can produce:
//! a structured recommendation with its evidence, never an executed mutation.
//!
//! There is deliberately no `Act` here. Accepting a proposal is the host's job,
//! and it routes into the EXISTING human-signed ceremony (desktop enclave
//! signature, phone Face-ID signature, or the Wardryx approvals/break-glass
//! flow). This crate holds no signer, so it cannot execute one itself. The
//! type is defined in C0 for stability; it is only emitted from C2.

use serde::{Deserialize, Serialize};

/// The kinds of action the copilot may PROPOSE. Each maps to an existing
/// human-signed mutation the host already implements; the copilot never calls
/// those paths, it only names one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// Kill a run (maps to the signed `POST /v1/runs/{id}/kill`).
    Kill,
    /// Set a run's budget (maps to the signed `POST /v1/runs/{id}/budget`).
    Budget,
    /// Grant or deny a Wardryx approval (maps to the signed approvals flow).
    GrantDeny,
    /// Re-run an Idryx identity scan (maps to `idryx detect`).
    Rescan,
}

/// A recommendation with its evidence. `evidence_refs` are ids the shell renders
/// verbatim next to the model's text (run ids, incident ids, store event ids),
/// so a claim is always checkable against source rows - the anti-hallucination
/// surface (D13.6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposedAction {
    pub kind: ActionKind,
    /// The subject: a run id, approval id, agent id, etc.
    pub target: String,
    /// Action parameters (e.g. `{"usd_cap": 5}` for a budget proposal). `{}` if none.
    pub params: serde_json::Value,
    /// Why the copilot proposes this, in one or two sentences.
    pub rationale: String,
    /// The model's self-reported confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// Source row ids backing the rationale, rendered verbatim by the shell.
    pub evidence_refs: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proposed_action_round_trips_json() {
        let action = ProposedAction {
            kind: ActionKind::Budget,
            target: "reconciliation-batch".into(),
            params: serde_json::json!({"usd_cap": 5}),
            rationale: "Burn tripled after a policy hold; cap it while investigating.".into(),
            confidence: 0.74,
            evidence_refs: vec!["incident:182".into(), "run:r-abc".into()],
        };
        let json = serde_json::to_string(&action).unwrap();
        let back: ProposedAction = serde_json::from_str(&json).unwrap();
        assert_eq!(action, back);
        assert!(json.contains("\"kind\":\"budget\""));
    }
}
