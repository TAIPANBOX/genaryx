//! Memory-plane read tools, backed by `EngramClient` over its MCP-stdio child
//! (docs/PHASE6-C1.md). The client is `&mut self` + synchronous, so it lives
//! behind an `Arc<Mutex<>>` in [`Clients`] and each call runs in
//! `spawn_blocking` with the lock held only for that one call (calls serialize).
//! Read-only here: `remember` (an append) is a human-gated `CopilotService`
//! method, never a free model tool (D13.3 - memory reflects human rulings).

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{Clients, Tool, ToolError, to_result};

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![Box::new(MemoryRecall), Box::new(MemoryWhy)]
}

pub(super) struct MemoryRecall;

#[async_trait]
impl Tool for MemoryRecall {
    fn name(&self) -> &'static str {
        "memory_recall"
    }
    fn description(&self) -> &'static str {
        "Recall past memories relevant to a query - prior incidents, human rulings, false-alarm notes. Use to check whether a pattern has been seen or judged before."
    }
    fn params_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "what to recall, in natural language"},
                "limit": {"type": "integer", "description": "max memories to return (default 5)"}
            },
            "required": ["query"]
        })
    }
    async fn run(&self, clients: &Clients, args: &Value) -> Result<Value, ToolError> {
        let engram = clients
            .engram
            .clone()
            .ok_or(ToolError::Unavailable("memory_recall"))?;
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::BadArgs {
                tool: "memory_recall",
                detail: "`query` (string) is required".to_string(),
            })?
            .to_string();
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(5)
            .clamp(1, 50) as u32;
        let memories = tokio::task::spawn_blocking(move || {
            let mut guard = engram.lock().expect("engram mutex poisoned");
            guard.recall(&query, limit, "hybrid", None)
        })
        .await
        .map_err(|e| ToolError::Connector {
            tool: "memory_recall",
            detail: e.to_string(),
        })?
        .map_err(|e| ToolError::Connector {
            tool: "memory_recall",
            detail: e.to_string(),
        })?;
        to_result("memory_recall", memories)
    }
}

pub(super) struct MemoryWhy;

#[async_trait]
impl Tool for MemoryWhy {
    fn name(&self) -> &'static str {
        "memory_why"
    }
    fn description(&self) -> &'static str {
        "Explain the provenance of one memory by id (where it came from, its lineage). Use after memory_recall to justify a recalled fact."
    }
    fn params_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "memory_id": {"type": "string", "description": "the memory id from a memory_recall result"}
            },
            "required": ["memory_id"]
        })
    }
    async fn run(&self, clients: &Clients, args: &Value) -> Result<Value, ToolError> {
        let engram = clients
            .engram
            .clone()
            .ok_or(ToolError::Unavailable("memory_why"))?;
        let memory_id = args
            .get("memory_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::BadArgs {
                tool: "memory_why",
                detail: "`memory_id` (string) is required".to_string(),
            })?
            .to_string();
        let provenance = tokio::task::spawn_blocking(move || {
            let mut guard = engram.lock().expect("engram mutex poisoned");
            guard.why(&memory_id)
        })
        .await
        .map_err(|e| ToolError::Connector {
            tool: "memory_why",
            detail: e.to_string(),
        })?
        .map_err(|e| ToolError::Connector {
            tool: "memory_why",
            detail: e.to_string(),
        })?;
        to_result("memory_why", provenance)
    }
}
