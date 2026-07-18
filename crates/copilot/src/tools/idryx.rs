//! Identity-plane read tools, backed by `IdryxClient` (async, unauthenticated
//! by Idryx's own design - it serves a loaded snapshot).

use async_trait::async_trait;
use serde_json::Value;

use super::{Clients, Tool, ToolError, to_result};

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![Box::new(Identities), Box::new(IdentityAlerts)]
}

pub(super) struct Identities;

#[async_trait]
impl Tool for Identities {
    fn name(&self) -> &'static str {
        "identities"
    }
    fn description(&self) -> &'static str {
        "Every agent/service identity in the loaded Idryx snapshot (id, kind, permissions, attestation state). Use to see who exists and how they are attested."
    }
    async fn run(&self, clients: &Clients, _args: &Value) -> Result<Value, ToolError> {
        let idryx = clients
            .idryx
            .as_ref()
            .ok_or(ToolError::Unavailable("identities"))?;
        let data = idryx
            .list_identities()
            .await
            .map_err(|e| ToolError::Connector {
                tool: "identities",
                detail: e.to_string(),
            })?;
        to_result("identities", data)
    }
}

pub(super) struct IdentityAlerts;

#[async_trait]
impl Tool for IdentityAlerts {
    fn name(&self) -> &'static str {
        "identity_alerts"
    }
    fn description(&self) -> &'static str {
        "Idryx detector alerts, severity-desc (over-privilege, missing attestation, stale rotation, and more). Use to see identity risk."
    }
    async fn run(&self, clients: &Clients, _args: &Value) -> Result<Value, ToolError> {
        let idryx = clients
            .idryx
            .as_ref()
            .ok_or(ToolError::Unavailable("identity_alerts"))?;
        let data = idryx
            .list_alerts()
            .await
            .map_err(|e| ToolError::Connector {
                tool: "identity_alerts",
                detail: e.to_string(),
            })?;
        to_result("identity_alerts", data)
    }
}
