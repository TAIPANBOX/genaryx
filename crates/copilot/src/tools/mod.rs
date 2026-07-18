//! The typed tool registry (docs/PHASE6.md, itrat-console/13 D13.1). Every tool
//! is a thin wrapper over an EXISTING connector read method - zero new I/O, the
//! copilot is a consumer. The set is fixed and typed: no tool synthesis, no
//! shell, no URL fetch, so the worst a prompt injection can do is trigger a
//! read the operator could already run (D13.3).
//!
//! C0 ships the 10 read tools that are all `async` over Cloud / Idryx / Wardryx.
//! The sync connectors (Qryx, Verdryx, Engram) and Wardryx's `decide` (a POST)
//! arrive in C1/C2, where the registry grows a `spawn_blocking` bridge.

mod cloud;
mod idryx;
mod wardryx;

use async_trait::async_trait;
use serde_json::{Value, json};

use genaryx_connectors::{CloudClient, IdryxClient, WardryxClient};

use crate::provider::ToolSpec;

/// The connector clients the tools read through. Each is optional: a tool is
/// only registered (and only advertised to the model) when its backing client
/// is present, so an install without, say, Idryx simply has no identity tools.
#[derive(Default)]
pub struct Clients {
    pub cloud: Option<CloudClient>,
    pub idryx: Option<IdryxClient>,
    pub wardryx: Option<WardryxClient>,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("unknown tool `{0}`")]
    Unknown(String),
    #[error("tool `{0}` is unavailable: its backing plane is not configured")]
    Unavailable(&'static str),
    #[error("tool `{tool}` failed: {detail}")]
    Connector { tool: &'static str, detail: String },
    #[error("could not serialize `{tool}` result: {source}")]
    Serialize {
        tool: &'static str,
        source: serde_json::Error,
    },
}

/// A single read tool. `run` returns the tool's result as a JSON value, ready to
/// hand back to the model as DATA (never instructions).
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// JSON-Schema for the arguments. C0 tools take none, so the default is an
    /// empty object; tools with parameters override this.
    fn params_schema(&self) -> Value {
        json!({"type": "object", "properties": {}, "additionalProperties": false})
    }
    async fn run(&self, clients: &Clients, args: &Value) -> Result<Value, ToolError>;
}

/// The fixed set of tools plus the clients they read through.
pub struct ToolRegistry {
    clients: Clients,
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    /// Build the registry, registering only tools whose backing client is
    /// present in `clients`.
    pub fn new(clients: Clients) -> Self {
        let mut tools: Vec<Box<dyn Tool>> = Vec::new();
        if clients.cloud.is_some() {
            tools.extend(cloud::tools());
        }
        if clients.idryx.is_some() {
            tools.extend(idryx::tools());
        }
        if clients.wardryx.is_some() {
            tools.extend(wardryx::tools());
        }
        Self { clients, tools }
    }

    /// The tool specs advertised to the model this turn.
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools
            .iter()
            .map(|t| ToolSpec {
                name: t.name().to_string(),
                description: t.description().to_string(),
                params_schema: t.params_schema(),
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn tool_names(&self) -> Vec<&'static str> {
        self.tools.iter().map(|t| t.name()).collect()
    }

    /// Dispatch a model-requested call by name. An unknown name is an error the
    /// loop feeds back to the model (so it can correct), never a panic.
    pub async fn dispatch(&self, name: &str, args: &Value) -> Result<Value, ToolError> {
        let tool = self
            .tools
            .iter()
            .find(|t| t.name() == name)
            .ok_or_else(|| ToolError::Unknown(name.to_string()))?;
        tool.run(&self.clients, args).await
    }
}

/// Shared helper: serialize a connector DTO into the tool's JSON result.
fn to_result<T: serde::Serialize>(tool: &'static str, value: T) -> Result<Value, ToolError> {
    serde_json::to_value(value).map_err(|source| ToolError::Serialize { tool, source })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_only_advertises_configured_planes() {
        // No clients -> no tools.
        let empty = ToolRegistry::new(Clients::default());
        assert!(empty.is_empty());
        assert!(empty.specs().is_empty());
    }

    #[tokio::test]
    async fn unknown_tool_is_an_error_not_a_panic() {
        let reg = ToolRegistry::new(Clients::default());
        let err = reg
            .dispatch("does_not_exist", &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Unknown(_)));
    }
}
