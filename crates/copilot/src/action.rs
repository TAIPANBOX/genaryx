//! The `Propose` tier of the action model (docs/PHASE6.md, itrat-console/13
//! D13.3). A [`ProposedAction`] is the ONLY thing a "propose" tool can produce:
//! a structured recommendation with its evidence, never an executed mutation.
//!
//! There is deliberately no `Act` here. Accepting a proposal is the host's job,
//! and it routes into the EXISTING human-signed ceremony (desktop enclave
//! signature, phone Face-ID signature, or the Wardryx approvals/break-glass
//! flow). This crate holds no signer, so it cannot execute one itself. The
//! type was defined in C0 for stability; C2's propose tools
//! (`tools::propose`) emit it, the loop collects it into `Answer.proposals`,
//! and the shell renders it as an approve/reject card.

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
    /// C2 Wardryx pre-check (side-effect-free): the targets of any policies that
    /// govern this action, read from `list_policies` when Wardryx is configured,
    /// so the card can show "governed by policy X". Empty when Wardryx is absent
    /// or no policy matches. A precise binary allow/deny PDP dry-run is deferred
    /// (it needs a genuine dry mode on Wardryx `/v1/decide`, which today can
    /// create a hold as a side effect). `#[serde(default)]` keeps older payloads
    /// that predate this field decodable.
    #[serde(default)]
    pub policy_context: Vec<String>,
}

/// The optional `copilot` block that can be attached to an alert: a one-line
/// summary, an optional recommended action, a confidence, and the cross-plane
/// chain. It only ever ENRICHES an alert that was already going to fire. A
/// dispatcher must send the deterministic alert first and attach this if it
/// arrives, so a slow or disabled copilot can never suppress or delay one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CopilotAnnotation {
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_action: Option<ProposedAction>,
    pub confidence: f32,
    #[serde(default)]
    pub chain: Vec<String>,
}

impl ProposedAction {
    /// Build a proposal with an empty `policy_context` (the propose tool fills it
    /// in from a `list_policies` read when Wardryx is available).
    pub fn new(
        kind: ActionKind,
        target: impl Into<String>,
        params: serde_json::Value,
        rationale: impl Into<String>,
        confidence: f32,
        evidence_refs: Vec<String>,
    ) -> Self {
        Self {
            kind,
            target: target.into(),
            params,
            rationale: rationale.into(),
            confidence: confidence.clamp(0.0, 1.0),
            evidence_refs,
            policy_context: Vec::new(),
        }
    }
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
            policy_context: vec!["agent://meridian/*".into()],
        };
        let json = serde_json::to_string(&action).unwrap();
        let back: ProposedAction = serde_json::from_str(&json).unwrap();
        assert_eq!(action, back);
        assert!(json.contains("\"kind\":\"budget\""));
    }

    #[test]
    fn new_clamps_confidence_and_defaults_policy_context() {
        let a = ProposedAction::new(
            ActionKind::Kill,
            "r-1",
            serde_json::json!({}),
            "why",
            1.5,
            vec![],
        );
        assert_eq!(a.confidence, 1.0); // clamped
        assert!(a.policy_context.is_empty());
    }

    #[test]
    fn a_payload_without_policy_context_still_decodes() {
        // #[serde(default)] keeps a C0/C1-era ProposedAction JSON decodable.
        let json = r#"{"kind":"kill","target":"r-1","params":{},"rationale":"x",
                       "confidence":0.5,"evidence_refs":[]}"#;
        let a: ProposedAction = serde_json::from_str(json).unwrap();
        assert!(a.policy_context.is_empty());
    }
}
