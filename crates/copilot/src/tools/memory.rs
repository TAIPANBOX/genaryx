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

#[cfg(test)]
mod tests {
    use super::*;
    use genaryx_connectors::EngramClient;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    /// A stand-in `engram-mcp`: newline JSON-RPC, an initialize handshake,
    /// then `recall` echoing the arguments it was given and `why` answering
    /// one provenance, or the tool's own error for `memory_id` "locked".
    const ENGRAM_MCP: &str = r#"
import sys, json
def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    mid = msg.get("id")
    if msg.get("method") == "initialize":
        send({"jsonrpc": "2.0", "id": mid, "result": {"protocolVersion": "2025-06-18", "serverInfo": {"name": "stand-in", "version": "1"}, "capabilities": {}}})
        continue
    if msg.get("method") != "tools/call":
        continue
    name = msg["params"]["name"]
    a = msg["params"].get("arguments", {})
    if name == "why" and a.get("memory_id") == "locked":
        send({"jsonrpc": "2.0", "id": mid, "result": {"content": [{"type": "text", "text": "memory is locked"}], "isError": True}})
        continue
    if name == "recall":
        content = "q=%s limit=%s mode=%s" % (a.get("query"), a.get("limit"), a.get("mode"))
        payload = {"result": [{"id": "m1", "content": content, "score": 0.9, "importance": 0.5, "timestamp": "2026-11-01T00:00:00+00:00"}]}
    else:
        payload = {"kind": "episodic", "id": a.get("memory_id"), "content": "observed a refund", "timestamp": "2026-11-01T00:00:00+00:00", "access_count": 1}
    send({"jsonrpc": "2.0", "id": mid, "result": {"content": [{"type": "text", "text": json.dumps(payload)}], "structuredContent": payload, "isError": False}})
"#;

    /// Clients holding a live stand-in Engram, or `None` (said) where there is
    /// no `python3` to run it.
    fn clients_with_engram() -> Option<(Clients, PathBuf)> {
        let have_python = std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());
        if !have_python {
            eprintln!("SKIP memory tools: no python3 on PATH");
            return None;
        }
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "gx-copilot-memory-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let bin = dir.join("engram-mcp");
        std::fs::write(&bin, "#!/bin/sh\nexec python3 \"$0.py\" \"$@\"\n").expect("write");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        std::fs::write(dir.join("engram-mcp.py"), ENGRAM_MCP).expect("write the server");
        // A script written a moment ago can be refused once as "text file
        // busy" while another test forks; that is the only failure retried.
        let mut spawned = EngramClient::spawn(&bin, ":memory:", None);
        for _ in 0..50 {
            let busy = matches!(
                &spawned,
                Err(e) if e.to_string().to_lowercase().contains("text file busy")
            );
            if !busy {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
            spawned = EngramClient::spawn(&bin, ":memory:", None);
        }
        let engram = spawned.expect("the stand-in starts");
        let clients = Clients {
            engram: Some(Arc::new(Mutex::new(engram))),
            ..Default::default()
        };
        Some((clients, dir))
    }

    #[test]
    fn the_memory_tools_are_named_and_ask_for_their_one_argument() {
        let names: Vec<&str> = tools().iter().map(|t| t.name()).collect();
        assert_eq!(names, ["memory_recall", "memory_why"]);
        assert_eq!(
            MemoryRecall.params_schema()["required"],
            serde_json::json!(["query"])
        );
        assert_eq!(
            MemoryWhy.params_schema()["required"],
            serde_json::json!(["memory_id"])
        );
        assert!(!MemoryRecall.is_propose() && !MemoryWhy.is_propose());
        assert!(MemoryRecall.description().contains("Recall"));
        assert!(MemoryWhy.description().contains("provenance"));
    }

    #[tokio::test]
    async fn without_engram_both_tools_are_unavailable() {
        let none = Clients::default();
        assert!(matches!(
            MemoryRecall.run(&none, &json!({"query": "x"})).await,
            Err(ToolError::Unavailable("memory_recall"))
        ));
        assert!(matches!(
            MemoryWhy.run(&none, &json!({"memory_id": "m1"})).await,
            Err(ToolError::Unavailable("memory_why"))
        ));
    }

    #[tokio::test]
    async fn recall_clamps_the_limit_and_asks_in_hybrid_mode() {
        let Some((clients, dir)) = clients_with_engram() else {
            return;
        };
        for (args, want) in [
            (json!({"query": "late refund"}), "q=late refund limit=5 mode=hybrid"),
            (json!({"query": "q", "limit": 500}), "q=q limit=50 mode=hybrid"),
            (json!({"query": "q", "limit": 0}), "q=q limit=1 mode=hybrid"),
            (json!({"query": "q", "limit": "ten"}), "q=q limit=5 mode=hybrid"),
        ] {
            let got = MemoryRecall.run(&clients, &args).await.expect("recall");
            assert_eq!(got[0]["content"], want, "{args}");
            assert_eq!(got[0]["id"], "m1");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_missing_argument_is_bad_args_and_a_tool_error_is_a_connector_error() {
        let Some((clients, dir)) = clients_with_engram() else {
            return;
        };
        assert!(matches!(
            MemoryRecall.run(&clients, &json!({"limit": 3})).await,
            Err(ToolError::BadArgs {
                tool: "memory_recall",
                ..
            })
        ));
        assert!(matches!(
            MemoryWhy.run(&clients, &json!({"memory_id": 7})).await,
            Err(ToolError::BadArgs {
                tool: "memory_why",
                ..
            })
        ));
        let why = MemoryWhy
            .run(&clients, &json!({"memory_id": "m9"}))
            .await
            .expect("why");
        assert_eq!(why["kind"], "episodic");
        assert_eq!(why["id"], "m9");
        match MemoryWhy.run(&clients, &json!({"memory_id": "locked"})).await {
            Err(ToolError::Connector { tool, detail }) => {
                assert_eq!(tool, "memory_why");
                assert!(detail.contains("memory is locked"), "{detail}");
            }
            other => panic!("expected the tool's error as a connector error, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
