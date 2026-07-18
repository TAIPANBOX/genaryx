//! Policy-plane read tools, backed by `WardryxClient` (async, bearer-auth).
//! Only reads here: the PDP `decide` (a POST that can create a hold) and the
//! grant/deny mutations are C2, routed through the existing signed ceremony.

use async_trait::async_trait;
use serde_json::Value;

use super::{Clients, Tool, ToolError, to_result};

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![Box::new(Policies), Box::new(ApprovalsInbox)]
}

pub(super) struct Policies;

#[async_trait]
impl Tool for Policies {
    fn name(&self) -> &'static str {
        "policies"
    }
    fn description(&self) -> &'static str {
        "Every stored Wardryx policy (target glob, denied tools, allowed domains, human-approval and hard-deny cost thresholds, max steps, deny-if-unattested). Use to explain why an action would be governed."
    }
    async fn run(&self, clients: &Clients, _args: &Value) -> Result<Value, ToolError> {
        let wardryx = clients
            .wardryx
            .as_ref()
            .ok_or(ToolError::Unavailable("policies"))?;
        let data = wardryx
            .list_policies()
            .await
            .map_err(|e| ToolError::Connector {
                tool: "policies",
                detail: e.to_string(),
            })?;
        to_result("policies", data)
    }
}

pub(super) struct ApprovalsInbox;

#[async_trait]
impl Tool for ApprovalsInbox {
    fn name(&self) -> &'static str {
        "approvals_inbox"
    }
    fn description(&self) -> &'static str {
        "Pending and decided human-approval holds (agent, run, requested time, decision). Use to see what is waiting on a human."
    }
    async fn run(&self, clients: &Clients, _args: &Value) -> Result<Value, ToolError> {
        let wardryx = clients
            .wardryx
            .as_ref()
            .ok_or(ToolError::Unavailable("approvals_inbox"))?;
        let data = wardryx
            .list_approvals()
            .await
            .map_err(|e| ToolError::Connector {
                tool: "approvals_inbox",
                detail: e.to_string(),
            })?;
        to_result("approvals_inbox", data)
    }
}
