//! Propose tools (C2, docs/PHASE6-C2.md, itrat-console/13 D13.3): the copilot
//! RECOMMENDS an action by emitting a [`ProposedAction`]. It never mutates and
//! holds no signer - a propose tool builds a descriptor only. The loop collects
//! each into `Answer.proposals`; the shell renders it as a card whose "Approve"
//! routes into the EXISTING human-signed ceremony (the human's signature, never
//! the copilot's).
//!
//! Each attaches a best-effort, side-effect-free Wardryx pre-check: a
//! `list_policies` READ that surfaces the policy targets in effect, so the card
//! can show "policies in effect - review for compliance". A precise binary
//! allow/deny PDP dry-run is deferred: Wardryx `/v1/decide` can create an
//! approval hold as a side effect, so it is not safe to call for a dry-run.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::action::{ActionKind, ProposedAction};

use super::{Clients, Tool, ToolError, to_result};

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ProposeKill),
        Box::new(ProposeBudget),
        Box::new(ProposeGrantDeny),
        Box::new(ProposeRescan),
    ]
}

/// The rationale + optional confidence/evidence every propose tool shares.
fn common(tool: &'static str, args: &Value) -> Result<(String, f32, Vec<String>), ToolError> {
    let reason = args
        .get("reason")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::BadArgs {
            tool,
            detail: "`reason` (string) is required".to_string(),
        })?
        .to_string();
    let confidence = args
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.5) as f32;
    let evidence_refs = args
        .get("evidence_refs")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Ok((reason, confidence, evidence_refs))
}

fn require_str(tool: &'static str, args: &Value, key: &'static str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| ToolError::BadArgs {
            tool,
            detail: format!("`{key}` (non-empty string) is required"),
        })
}

/// Best-effort, side-effect-free Wardryx pre-check (see the module doc). Bounded
/// so a large policy set never bloats the proposal; a read failure never blocks
/// the proposal (it just yields no policy context).
async fn policy_context(clients: &Clients) -> Vec<String> {
    let Some(wardryx) = clients.wardryx.as_ref() else {
        return Vec::new();
    };
    match wardryx.list_policies().await {
        Ok(policies) => policies
            .into_iter()
            .filter(|p| !p.policy.target.is_empty())
            .map(|p| p.policy.target)
            .take(8)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Attach the pre-check and serialize the proposal as the tool result.
async fn finish(
    tool: &'static str,
    clients: &Clients,
    mut action: ProposedAction,
) -> Result<Value, ToolError> {
    action.policy_context = policy_context(clients).await;
    to_result(tool, action)
}

pub(super) struct ProposeKill;

#[async_trait]
impl Tool for ProposeKill {
    fn name(&self) -> &'static str {
        "propose_kill"
    }
    fn description(&self) -> &'static str {
        "PROPOSE killing a run (a recommendation only - a human must approve and sign it; you cannot kill anything yourself). Use for a confirmed runaway or a clearly over-cap run."
    }
    fn is_propose(&self) -> bool {
        true
    }
    fn params_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "run_id":{"type":"string","description":"the run to kill"},
            "reason":{"type":"string","description":"why this run should be killed"},
            "confidence":{"type":"number","description":"your confidence, 0.0-1.0"},
            "evidence_refs":{"type":"array","items":{"type":"string"},"description":"run/incident/alert ids backing this"}
        },"required":["run_id","reason"]})
    }
    async fn run(&self, clients: &Clients, args: &Value) -> Result<Value, ToolError> {
        let run_id = require_str("propose_kill", args, "run_id")?;
        let (reason, confidence, evidence) = common("propose_kill", args)?;
        let action = ProposedAction::new(
            ActionKind::Kill,
            run_id,
            json!({}),
            reason,
            confidence,
            evidence,
        );
        finish("propose_kill", clients, action).await
    }
}

pub(super) struct ProposeBudget;

#[async_trait]
impl Tool for ProposeBudget {
    fn name(&self) -> &'static str {
        "propose_budget"
    }
    fn description(&self) -> &'static str {
        "PROPOSE capping a run's budget at usd_cap (a recommendation only - a human approves and signs it). Use to contain a run without killing it."
    }
    fn is_propose(&self) -> bool {
        true
    }
    fn params_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "run_id":{"type":"string"},
            "usd_cap":{"type":"number","description":"the new budget ceiling in USD"},
            "reason":{"type":"string"},
            "confidence":{"type":"number"},
            "evidence_refs":{"type":"array","items":{"type":"string"}}
        },"required":["run_id","usd_cap","reason"]})
    }
    async fn run(&self, clients: &Clients, args: &Value) -> Result<Value, ToolError> {
        let run_id = require_str("propose_budget", args, "run_id")?;
        let usd_cap = args
            .get("usd_cap")
            .and_then(Value::as_f64)
            .filter(|v| *v >= 0.0)
            .ok_or_else(|| ToolError::BadArgs {
                tool: "propose_budget",
                detail: "`usd_cap` (non-negative number) is required".to_string(),
            })?;
        let (reason, confidence, evidence) = common("propose_budget", args)?;
        let action = ProposedAction::new(
            ActionKind::Budget,
            run_id,
            json!({ "usd_cap": usd_cap }),
            reason,
            confidence,
            evidence,
        );
        finish("propose_budget", clients, action).await
    }
}

pub(super) struct ProposeGrantDeny;

#[async_trait]
impl Tool for ProposeGrantDeny {
    fn name(&self) -> &'static str {
        "propose_grant_deny"
    }
    fn description(&self) -> &'static str {
        "PROPOSE granting or denying a pending Wardryx approval (a recommendation only - a human approves and signs the decision). `verdict` is \"grant\" or \"deny\"."
    }
    fn is_propose(&self) -> bool {
        true
    }
    fn params_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "approval_id":{"type":"string"},
            "verdict":{"type":"string","enum":["grant","deny"]},
            "reason":{"type":"string"},
            "confidence":{"type":"number"},
            "evidence_refs":{"type":"array","items":{"type":"string"}}
        },"required":["approval_id","verdict","reason"]})
    }
    async fn run(&self, clients: &Clients, args: &Value) -> Result<Value, ToolError> {
        let approval_id = require_str("propose_grant_deny", args, "approval_id")?;
        let verdict = require_str("propose_grant_deny", args, "verdict")?;
        if verdict != "grant" && verdict != "deny" {
            return Err(ToolError::BadArgs {
                tool: "propose_grant_deny",
                detail: "`verdict` must be \"grant\" or \"deny\"".to_string(),
            });
        }
        let (reason, confidence, evidence) = common("propose_grant_deny", args)?;
        let action = ProposedAction::new(
            ActionKind::GrantDeny,
            approval_id,
            json!({ "verdict": verdict }),
            reason,
            confidence,
            evidence,
        );
        finish("propose_grant_deny", clients, action).await
    }
}

pub(super) struct ProposeRescan;

#[async_trait]
impl Tool for ProposeRescan {
    fn name(&self) -> &'static str {
        "propose_rescan"
    }
    fn description(&self) -> &'static str {
        "PROPOSE re-running an Idryx identity scan (a recommendation only - a human approves it). Use when identity posture looks stale or newly at risk. Optional `target` names an agent; default is the whole fleet."
    }
    fn is_propose(&self) -> bool {
        true
    }
    fn params_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "reason":{"type":"string"},
            "target":{"type":"string","description":"agent id, or omit for the whole fleet"},
            "confidence":{"type":"number"},
            "evidence_refs":{"type":"array","items":{"type":"string"}}
        },"required":["reason"]})
    }
    async fn run(&self, clients: &Clients, args: &Value) -> Result<Value, ToolError> {
        let target = args
            .get("target")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("all")
            .to_string();
        let (reason, confidence, evidence) = common("propose_rescan", args)?;
        let action = ProposedAction::new(
            ActionKind::Rescan,
            target,
            json!({}),
            reason,
            confidence,
            evidence,
        );
        finish("propose_rescan", clients, action).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn propose_kill_emits_a_proposed_action() {
        let out = ProposeKill
            .run(
                &Clients::default(),
                &json!({"run_id":"reconciliation-batch","reason":"4350 calls, runaway","confidence":0.8,"evidence_refs":["run:reconciliation-batch"]}),
            )
            .await
            .unwrap();
        let action: ProposedAction = serde_json::from_value(out).unwrap();
        assert_eq!(action.kind, ActionKind::Kill);
        assert_eq!(action.target, "reconciliation-batch");
        assert_eq!(action.confidence, 0.8);
        assert!(action.policy_context.is_empty()); // no wardryx configured
        assert!(ProposeKill.is_propose());
    }

    #[tokio::test]
    async fn propose_budget_requires_a_cap() {
        let err = ProposeBudget
            .run(
                &Clients::default(),
                &json!({"run_id":"r-1","reason":"contain it"}),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ToolError::BadArgs {
                tool: "propose_budget",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn propose_grant_deny_validates_the_verdict() {
        let err = ProposeGrantDeny
            .run(
                &Clients::default(),
                &json!({"approval_id":"a-1","verdict":"maybe","reason":"x"}),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ToolError::BadArgs {
                tool: "propose_grant_deny",
                ..
            }
        ));

        let ok = ProposeGrantDeny
            .run(
                &Clients::default(),
                &json!({"approval_id":"a-1","verdict":"deny","reason":"over cost ceiling"}),
            )
            .await
            .unwrap();
        let action: ProposedAction = serde_json::from_value(ok).unwrap();
        assert_eq!(action.kind, ActionKind::GrantDeny);
        assert_eq!(action.params["verdict"], "deny");
    }
}
