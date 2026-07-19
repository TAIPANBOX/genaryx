//! The typed tool registry (docs/PHASE6.md, itrat-console/13 D13.1). Every tool
//! is a thin wrapper over an EXISTING connector read method - zero new I/O, the
//! copilot is a consumer. The set is fixed and typed: no tool synthesis, no
//! shell, no URL fetch, so the worst a prompt injection can do is trigger a
//! read the operator could already run (D13.3).
//!
//! C0 shipped 10 async read tools over Cloud / Idryx / Wardryx. C1 adds the sync
//! connectors (Qryx, Verdryx, Engram) via a `spawn_blocking` bridge: memory
//! recall/why, quality, and `crypto_scan` (the first parameterized tool).
//! Wardryx's `decide` (a POST that can create a hold) is still C2.

mod cloud;
mod crypto;
mod idryx;
mod memory;
mod quality;
mod wardryx;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use genaryx_connectors::{CloudClient, EngramClient, IdryxClient, WardryxClient};

use crate::provider::ToolSpec;

/// The connector clients the tools read through. Each is optional: a tool is
/// only registered (and only advertised to the model) when its backing client
/// is present, so an install without, say, Idryx simply has no identity tools.
#[derive(Default)]
pub struct Clients {
    pub cloud: Option<CloudClient>,
    pub idryx: Option<IdryxClient>,
    pub wardryx: Option<WardryxClient>,
    /// The Engram MCP client is long-lived (one stdio child + handshake) and
    /// `&mut self`, so it is shared behind a Mutex and its calls serialize
    /// (docs/PHASE6-C1.md, the sync-tool bridge).
    pub engram: Option<Arc<Mutex<EngramClient>>>,
    /// The `qryx` binary path; `crypto_scan` shells it fresh inside a blocking
    /// task (Qryx is a CLI, no long-lived state to hold).
    pub qryx_bin: Option<PathBuf>,
    /// The `verdryx.db` path; `quality_latest` opens it read-only inside a
    /// blocking task (a rusqlite Connection is `!Sync`, never shared).
    pub verdryx_db: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("unknown tool `{0}`")]
    Unknown(String),
    #[error("tool `{0}` is unavailable: its backing plane is not configured")]
    Unavailable(&'static str),
    #[error("tool `{tool}`: bad arguments: {detail}")]
    BadArgs { tool: &'static str, detail: String },
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
        if clients.engram.is_some() {
            tools.extend(memory::tools());
        }
        if clients.qryx_bin.is_some() {
            tools.extend(crypto::tools());
        }
        if clients.verdryx_db.is_some() {
            tools.extend(quality::tools());
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

    #[test]
    fn c1_sync_tools_register_from_their_paths() {
        // qryx / verdryx are backed by a path in Clients (no live client to
        // construct), so registration is testable without their binaries/data.
        let reg = ToolRegistry::new(Clients {
            qryx_bin: Some(PathBuf::from("/x/qryx")),
            verdryx_db: Some(PathBuf::from("/x/verdryx.db")),
            ..Default::default()
        });
        let names = reg.tool_names();
        assert!(names.contains(&"crypto_scan"));
        assert!(names.contains(&"quality_latest"));
        // Engram not configured -> its memory tools are not advertised.
        assert!(!names.contains(&"memory_recall"));
        // And its params_schema is advertised for the parameterized tool.
        let crypto = reg
            .specs()
            .into_iter()
            .find(|s| s.name == "crypto_scan")
            .expect("crypto_scan advertised");
        assert_eq!(crypto.params_schema["required"][0], "path");
    }
}
