//! `McpStdioClient`: a minimal, synchronous Model Context Protocol (MCP)
//! client over a stdio child process (docs/PHASE4.md W2) - the console's first
//! MCP-client connector, the transport [`crate::EngramClient`] speaks to
//! `engram-mcp`. Grounded in the MCP stdio spec + engram's own server
//! (`~/Development/engram/engram/mcp_server.py`, read 2026-07-17, which runs
//! the official `mcp` SDK's `FastMCP(...).run(transport="stdio")`).
//!
//! ## Wire protocol (why these exact framing choices)
//!
//! MCP over stdio is **newline-delimited JSON-RPC 2.0**: each message is one
//! JSON object on its own `\n`-terminated line, no embedded newlines, no
//! Content-Length header (that is the older LSP framing; MCP stdio does not use
//! it). The server's stdout carries protocol messages ONLY - engram is careful
//! to send every diagnostic to stderr (`mcp_server.py:476-481`), so a line on
//! stdout is always a JSON-RPC message. The mandated handshake is
//! `initialize` (request/response) then a `notifications/initialized`
//! notification, before any `tools/call` (MCP base protocol). [`McpStdioClient::spawn`]
//! performs it once; the process is then long-lived and every tool call reuses
//! it (re-spawning per call would re-run engram's lazy embedding-model load, a
//! multi-second cost, on every `recall`).
//!
//! ## A tool result is an envelope, not the value
//!
//! `tools/call` returns `{content: [{type:"text", text: "<json>"}], isError,
//! structuredContent?}`. FastMCP JSON-serializes a tool's `dict`/`list` return
//! into that `text` (and, on newer SDKs, also into `structuredContent`).
//! [`parse_tool_result`] unwraps it: `isError == true` becomes
//! [`McpError::Tool`]; otherwise it prefers `structuredContent` when present and
//! falls back to parsing `content[0].text` as JSON - the text path is the
//! stable one every FastMCP version emits.
//!
//! ## Fail-closed + no hang (06 §0.5)
//!
//! A dedicated reader thread turns the child's stdout into a channel of lines,
//! so every wait is bounded by a [`std::time::Instant`] deadline
//! ([`McpStdioClient::timeout`]): a server that never answers becomes
//! [`McpError::Timeout`], never an infinite block. A spawn failure is
//! [`McpError::Spawn`] (the live test reads it as "engram-mcp absent, skip");
//! malformed framing is [`McpError::Protocol`]; a JSON-RPC `error` object is
//! [`McpError::Rpc`]. Nothing panics across the boundary.
//!
//! ## Process ownership
//!
//! The child is one THIS client spawned, so [`Drop`] closes its stdin (EOF, the
//! clean MCP shutdown signal) then `kill`+`wait`s it - reaping our own child,
//! which is allowed (the process-kill restriction is about PIDs discovered via
//! `ps`/`lsof`, never a subprocess we launched ourselves).

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The MCP protocol version this client advertises in `initialize`. The server
/// negotiates and returns its own supported version; FastMCP accepts a recent
/// value and echoes what it speaks.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// Default per-call deadline. Generous on purpose: engram's first `recall`
/// lazily loads a local embedding model, which can take several seconds
/// (`mcp_server.py` `_EngramPool.get` -> `Engram(...)`).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

// ---- error -----------------------------------------------------------------

/// Every failure mode an [`McpStdioClient`] call can surface. Fail-closed: a
/// spawn failure, an I/O failure, a protocol/parse failure, a JSON-RPC error
/// object, a tool-level error, and a timeout are all distinct, never a panic.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// The MCP server binary could not be spawned (missing, not executable).
    /// The live test reads this as "server absent, skip."
    #[error("mcp spawn {command}: {source}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },

    /// Writing a request to the child's stdin failed - usually because the
    /// child has exited (a crashed or killed server).
    #[error("mcp io: {0}")]
    Io(#[from] std::io::Error),

    /// A stdout line was not valid JSON, or a response was missing a field the
    /// protocol requires (no `result`/`error`, unmatched framing).
    #[error("mcp protocol: {0}")]
    Protocol(String),

    /// The server answered with a JSON-RPC `error` object (bad method, bad
    /// params) - carries its code and message.
    #[error("mcp rpc error {code}: {message}")]
    Rpc { code: i64, message: String },

    /// A `tools/call` returned `isError: true`: the tool ran and reported a
    /// failure (e.g. engram's "memory not found"). Distinct from [`Self::Rpc`]
    /// (a protocol-level error) - this is an application-level tool failure.
    #[error("mcp tool error: {0}")]
    Tool(String),

    /// The server did not answer within [`McpStdioClient::timeout`], or its
    /// stdout closed (the reader thread ended) before the response arrived.
    #[error("mcp timeout/closed after {0:?}")]
    Timeout(Duration),
}

// ---- tool metadata (tools/list) --------------------------------------------

/// One tool the server advertises via `tools/list` (`{name, description?,
/// inputSchema}`). The console can render the catalog, though [`crate::EngramClient`]
/// calls engram's five known tools by name directly.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ToolDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The tool's JSON-Schema for its arguments, kept raw (display-only).
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Value,
}

// ---- result-envelope parsing (pure, unit-tested without a process) ---------

/// Unwrap a `tools/call` result envelope into the tool's actual JSON return
/// value. `isError: true` becomes [`McpError::Tool`] (with the text content as
/// its message); otherwise prefer `structuredContent`, else parse the first
/// text content block as JSON. A result with neither is [`McpError::Protocol`].
pub fn parse_tool_result(result: &Value) -> Result<Value, McpError> {
    let text_blocks = || -> String {
        result
            .get("content")
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    };

    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        let msg = text_blocks();
        return Err(McpError::Tool(if msg.is_empty() {
            "tool reported isError with no message".to_string()
        } else {
            msg
        }));
    }

    // FastMCP represents a tool's return two ways, and they differ by shape
    // (both confirmed by a raw JSON-RPC probe of real `engram-mcp`):
    //  - `structuredContent` holds the WHOLE value as a JSON object: a dict
    //    tool (engram `stats`/`why`/`forget`) is the dict as-is; a NON-object
    //    return (engram `recall`'s list) is wrapped under a synthetic single
    //    `result` key, `{"result": [...]}`, so it stays an object.
    //  - the `content` blocks are per-ITEM for a list return (one text block
    //    per memory), so joining them is NOT valid JSON.
    // So prefer `structuredContent` (unwrapping the `{"result": …}` wrapper) -
    // it is the one intact source - and fall back to the text block only when
    // structuredContent is absent (a dict tool on an SDK predating it).
    // Preferring the per-item text blocks instead broke `recall` for any
    // single-item result: the lone block parsed as one object, not an array.
    if let Some(sc) = result.get("structuredContent")
        && !sc.is_null()
    {
        return Ok(unwrap_fastmcp_structured(sc));
    }

    let text = text_blocks();
    if !text.is_empty() {
        return match serde_json::from_str::<Value>(&text) {
            Ok(v) => Ok(unwrap_fastmcp_structured(&v)),
            // A bare-string tool return: surface the text as a JSON string.
            Err(_) => Ok(Value::String(text)),
        };
    }

    Err(McpError::Protocol(
        "tools/call result had neither structuredContent nor text content".to_string(),
    ))
}

/// FastMCP wraps a NON-object tool return (a list, a scalar, a string) under a
/// synthetic single `result` key so the structured payload stays a JSON object.
/// Unwrap exactly that shape - a single `result` key whose value is NOT itself
/// an object (matching FastMCP's wrap-only-non-objects rule) - so a
/// list-returning tool round-trips to its array. A genuine object return (every
/// engram dict tool: `stats`/`why`/`forget`, all multi-key) and any other shape
/// pass through untouched.
fn unwrap_fastmcp_structured(v: &Value) -> Value {
    if let Value::Object(map) = v
        && map.len() == 1
        && let Some(inner) = map.get("result")
        && !inner.is_object()
    {
        return inner.clone();
    }
    v.clone()
}

// ---- client ----------------------------------------------------------------

/// A synchronous MCP client bound to one stdio child process. Not `Sync` (it
/// owns the child's pipes and a monotonically increasing request id); a caller
/// that needs concurrency spawns one per context, exactly as
/// [`crate::VerdryxClient`] opens one SQLite connection per read context.
#[derive(Debug)]
pub struct McpStdioClient {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<String>,
    reader: Option<JoinHandle<()>>,
    next_id: i64,
    timeout: Duration,
    server_info: Value,
}

impl McpStdioClient {
    /// Spawn `program` with `args` and the extra `env` vars, then perform the
    /// MCP `initialize` handshake. stdin/stdout are piped; **stderr is
    /// inherited** so the server's own diagnostics still reach the operator's
    /// terminal and never pollute the protocol channel. Returns once the server
    /// has answered `initialize` and been sent `notifications/initialized`.
    pub fn spawn(
        program: &str,
        args: &[&str],
        env: &BTreeMap<String, String>,
    ) -> Result<Self, McpError> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|source| McpError::Spawn {
            command: program.to_string(),
            source,
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Protocol("child stdin was not captured".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Protocol("child stdout was not captured".to_string()))?;

        // Reader thread: one line -> one channel message. Ends on EOF/err,
        // which drops the sender and turns later receives into a clean
        // Timeout/closed rather than a hang.
        let (tx, rx) = channel::<String>();
        let reader = std::thread::spawn(move || {
            let buf = BufReader::new(stdout);
            for line in buf.lines() {
                match line {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break; // client dropped
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        let mut client = Self {
            child,
            stdin: Some(stdin),
            rx,
            reader: Some(reader),
            next_id: 0,
            timeout: DEFAULT_TIMEOUT,
            server_info: Value::Null,
        };
        client.handshake()?;
        Ok(client)
    }

    /// Override the default per-call deadline (builder-style).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The `serverInfo`/`capabilities` object the server returned at
    /// `initialize` (raw). Lets a panel show which server/version it is talking
    /// to.
    pub fn server_info(&self) -> &Value {
        &self.server_info
    }

    fn handshake(&mut self) -> Result<(), McpError> {
        let id = self.take_id();
        let init = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "genaryx", "version": env!("CARGO_PKG_VERSION") },
            }
        });
        self.send(&init)?;
        let resp = self.recv_response(id)?;
        self.server_info = resp.get("result").cloned().unwrap_or(Value::Null);

        // The post-initialize notification (no id, no response expected).
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        self.send(&note)?;
        Ok(())
    }

    /// `tools/list` -> the server's advertised tool catalog.
    pub fn list_tools(&mut self) -> Result<Vec<ToolDef>, McpError> {
        let id = self.take_id();
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": "tools/list", "params": {} });
        self.send(&req)?;
        let resp = self.recv_response(id)?;
        let result = self.result_or_rpc_err(&resp)?;
        let tools = result.get("tools").cloned().unwrap_or(Value::Array(vec![]));
        serde_json::from_value(tools)
            .map_err(|e| McpError::Protocol(format!("tools/list shape: {e}")))
    }

    /// `tools/call` with `name` + `arguments` -> the tool's parsed JSON return
    /// value (see [`parse_tool_result`]). `arguments` should be a JSON object;
    /// pass `json!({})` for a no-arg tool.
    pub fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, McpError> {
        let id = self.take_id();
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        });
        self.send(&req)?;
        let resp = self.recv_response(id)?;
        let result = self.result_or_rpc_err(&resp)?;
        parse_tool_result(&result)
    }

    // ---- internals ---------------------------------------------------------

    fn take_id(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn send(&mut self, msg: &Value) -> Result<(), McpError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| McpError::Protocol("stdin already closed".to_string()))?;
        let mut line = serde_json::to_string(msg)
            .map_err(|e| McpError::Protocol(format!("encode request: {e}")))?;
        line.push('\n');
        stdin.write_all(line.as_bytes())?;
        stdin.flush()?;
        Ok(())
    }

    /// Read stdout lines until the JSON-RPC response whose `id` matches, bounded
    /// by [`Self::timeout`]. Server-initiated notifications and any non-matching
    /// message (id present but different, or no id) are skipped, never
    /// mistaken for the answer.
    fn recv_response(&self, id: i64) -> Result<Value, McpError> {
        let deadline = Instant::now() + self.timeout;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or(McpError::Timeout(self.timeout))?;
            let line = match self.rx.recv_timeout(remaining) {
                Ok(l) => l,
                Err(RecvTimeoutError::Timeout) => return Err(McpError::Timeout(self.timeout)),
                Err(RecvTimeoutError::Disconnected) => return Err(McpError::Timeout(self.timeout)),
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(trimmed)
                .map_err(|e| McpError::Protocol(format!("stdout line not JSON: {e}")))?;
            if v.get("id").and_then(Value::as_i64) == Some(id) {
                return Ok(v);
            }
            // A different id or a notification (no id): not our answer, keep reading.
        }
    }

    /// Extract `result` from a JSON-RPC response, mapping an `error` object to
    /// [`McpError::Rpc`] and a response with neither to [`McpError::Protocol`].
    fn result_or_rpc_err(&self, resp: &Value) -> Result<Value, McpError> {
        if let Some(err) = resp.get("error") {
            let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
            let message = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("<no message>")
                .to_string();
            return Err(McpError::Rpc { code, message });
        }
        resp.get("result")
            .cloned()
            .ok_or_else(|| McpError::Protocol("response had neither result nor error".to_string()))
    }
}

impl Drop for McpStdioClient {
    fn drop(&mut self) {
        // Close stdin first: EOF is the clean MCP shutdown signal, giving the
        // server a chance to exit on its own. Then force-kill+reap our own
        // child so drop never blocks on a server that ignores EOF. Killing a
        // subprocess we launched is allowed (unlike a ps/lsof-discovered PID).
        self.stdin.take(); // drop -> close the pipe
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tool_result_dict_return_round_trips() {
        // A dict-returning tool (stats/why/forget): structuredContent is the
        // dict as-is (multi-key, so the result-wrapper unwrap is a no-op).
        let result = json!({
            "content": [{"type": "text", "text": "{\"agent_id\":\"a\",\"counts\":{\"episodic\":3}}"}],
            "structuredContent": {"agent_id": "a", "counts": {"episodic": 3}},
            "isError": false
        });
        let v = parse_tool_result(&result).expect("parse");
        assert_eq!(v["agent_id"], "a");
        assert_eq!(v["counts"]["episodic"], 3);
    }

    #[test]
    fn parse_tool_result_list_return_is_the_array_not_the_fastmcp_result_wrapper() {
        // The recall regression (caught live against real engram-mcp): FastMCP
        // wraps a list return under {"result": [...]} in BOTH the text block AND
        // structuredContent (this SDK version), so we must unwrap it from
        // whichever source, returning the bare array so Vec<EngramMemory>
        // deserializes.
        let result = json!({
            "content": [{"type": "text", "text": "{\"result\":[{\"id\":\"m1\"},{\"id\":\"m2\"}]}"}],
            "structuredContent": {"result": [{"id":"m1"},{"id":"m2"}]},
            "isError": false
        });
        let v = parse_tool_result(&result).expect("parse");
        assert!(v.is_array(), "must be the array, not the result wrapper");
        assert_eq!(v.as_array().unwrap().len(), 2);
        assert_eq!(v[0]["id"], "m1");
    }

    #[test]
    fn parse_tool_result_does_not_unwrap_a_genuine_object_return() {
        // A single-key `result` object whose value is itself an OBJECT is NOT
        // FastMCP's non-object wrapper, so it must pass through untouched (guards
        // the `!inner.is_object()` precision).
        let result = json!({
            "content": [{"type": "text", "text": "{\"result\":{\"nested\":true}}"}]
        });
        let v = parse_tool_result(&result).expect("parse");
        assert!(v.is_object());
        assert_eq!(v["result"]["nested"], true);
    }

    #[test]
    fn parse_tool_result_unwraps_result_wrapper_when_only_structured_present() {
        // No text block at all: fall back to structuredContent and still unwrap
        // FastMCP's {"result": X} single-key wrapper for a non-object return.
        let result = json!({ "structuredContent": {"result": [{"id":"m1"}]} });
        let v = parse_tool_result(&result).expect("parse");
        assert!(v.is_array());
        assert_eq!(v[0]["id"], "m1");
    }

    #[test]
    fn parse_tool_result_falls_back_to_text_json() {
        // No structuredContent: the text block is the JSON return value.
        let result = json!({
            "content": [{"type": "text", "text": "[{\"id\":\"m1\",\"score\":0.9}]"}]
        });
        let v = parse_tool_result(&result).expect("parse");
        assert!(v.is_array());
        assert_eq!(v[0]["id"], "m1");
    }

    #[test]
    fn parse_tool_result_is_error_becomes_tool_error_with_message() {
        let result = json!({
            "content": [{"type": "text", "text": "memory not found: 'x'"}],
            "isError": true
        });
        match parse_tool_result(&result) {
            Err(McpError::Tool(msg)) => assert!(msg.contains("memory not found")),
            other => panic!("expected Tool error, got {other:?}"),
        }
    }

    #[test]
    fn parse_tool_result_empty_is_protocol_error() {
        let result = json!({ "content": [] });
        match parse_tool_result(&result) {
            Err(McpError::Protocol(_)) => {}
            other => panic!("expected Protocol error, got {other:?}"),
        }
    }

    #[test]
    fn spawn_missing_binary_is_fail_closed_spawn_error() {
        let env = BTreeMap::new();
        match McpStdioClient::spawn("/nonexistent/engram-mcp-xyz", &["--db", ":memory:"], &env) {
            Err(McpError::Spawn { .. }) => {}
            other => panic!("expected Spawn error, got {other:?}"),
        }
    }

    // End-to-end over a real spawned process, using a tiny newline-JSON mock
    // MCP server (no `mcp` SDK needed) - exercises framing + the initialize
    // handshake + a tools/call round trip. Skips gracefully if python3 is
    // absent. A live test against real `engram-mcp` lives in tests/.
    #[test]
    fn end_to_end_handshake_and_tool_call_over_a_mock_server() {
        if which_python3().is_none() {
            eprintln!("skip: python3 not found");
            return;
        }
        // Mock: reads newline JSON-RPC; answers initialize, then a tools/call
        // for "stats" with a structuredContent payload; ignores the
        // notifications/initialized line (no id).
        let mock = r#"
import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    mid = msg.get("id")
    method = msg.get("method")
    if method == "initialize":
        sys.stdout.write(json.dumps({"jsonrpc":"2.0","id":mid,"result":{"protocolVersion":"2025-06-18","serverInfo":{"name":"mock","version":"1"},"capabilities":{}}})+"\n")
        sys.stdout.flush()
    elif method == "notifications/initialized":
        pass
    elif method == "tools/call":
        name = msg["params"]["name"]
        payload = {"tool": name, "ok": True}
        res = {"content":[{"type":"text","text":json.dumps(payload)}],"structuredContent":payload,"isError":False}
        sys.stdout.write(json.dumps({"jsonrpc":"2.0","id":mid,"result":res})+"\n")
        sys.stdout.flush()
    elif method == "tools/list":
        res = {"tools":[{"name":"stats","description":"d","inputSchema":{"type":"object"}}]}
        sys.stdout.write(json.dumps({"jsonrpc":"2.0","id":mid,"result":res})+"\n")
        sys.stdout.flush()
"#;
        let env = BTreeMap::new();
        let mut client = McpStdioClient::spawn("python3", &["-c", mock], &env)
            .expect("spawn mock server")
            .with_timeout(Duration::from_secs(10));

        assert_eq!(client.server_info()["serverInfo"]["name"], "mock");

        let tools = client.list_tools().expect("list_tools");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "stats");

        let out = client
            .call_tool("stats", json!({}))
            .expect("call_tool stats");
        assert_eq!(out["tool"], "stats");
        assert_eq!(out["ok"], true);
    }

    fn which_python3() -> Option<()> {
        std::process::Command::new("python3")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok()
            .filter(|s| s.success())
            .map(|_| ())
    }
}
