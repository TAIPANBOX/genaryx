//! `EngramClient` over a stand-in `engram-mcp`: a few lines of Python that
//! speak newline JSON-RPC the way the real server does (an initialize
//! handshake, then `tools/call`), so the client's spawn arguments, its read
//! scope, each tool's result shape, and an RPC error, a tool error and a
//! result of the wrong shape are all held without the real memory plane.
//! Skips, saying so, where no `python3` is on the PATH.

use std::path::{Path, PathBuf};

use genaryx_connectors::{EngramClient, EngramProvenance, McpError};

use crate::stand_in::{retry_busy, scratch_dir, script};

/// The stand-in server. It answers `initialize` with its own argv inside the
/// result (so a test can read what it was started with), precedes every
/// answer with a notification the client must skip, and picks its answer by
/// tool name and argument.
const ENGRAM_MCP: &str = r#"
import sys, json
argv = sys.argv[1:]
def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    mid = msg.get("id")
    method = msg.get("method")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": mid, "result": {"protocolVersion": "2025-06-18", "serverInfo": {"name": "engram-stand-in", "version": "1"}, "capabilities": {}, "argv": argv}})
        continue
    if method != "tools/call":
        continue
    send({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info"}})
    name = msg["params"]["name"]
    a = msg["params"].get("arguments", {})
    if name == "forget" and a.get("memory_id") == "rpc-error":
        send({"jsonrpc": "2.0", "id": mid, "error": {"code": -32602, "message": "no such memory"}})
        continue
    if name == "why" and a.get("memory_id") == "tool-error":
        send({"jsonrpc": "2.0", "id": mid, "result": {"content": [{"type": "text", "text": "memory is locked"}], "isError": True}})
        continue
    if name == "stats":
        payload = {"agent_id": None, "counts": {"episodic": 2, "semantic": 1, "procedural": 0}, "vector_index_size": 3, "facts_total": 1, "facts_active": 1, "facts_superseded": 0, "entities": 4, "reflections": 0, "db_path": ":memory:", "db_size_bytes": None, "args": a}
    elif name == "recall" and a.get("query") == "wrong shape":
        payload = {"not": "a list"}
    elif name == "recall":
        content = "q=%s limit=%s mode=%s keys=%s" % (a.get("query"), a.get("limit"), a.get("mode"), ",".join(sorted(a.keys())))
        payload = {"result": [{"id": "m1", "content": content, "score": 0.9, "importance": 0.5, "timestamp": "2026-11-01T00:00:00+00:00"}]}
    elif name == "why":
        payload = {"kind": "episodic", "id": a.get("memory_id"), "content": "observed a refund", "timestamp": "2026-11-01T00:00:00+00:00", "access_count": 2}
    else:
        payload = {"id": a.get("memory_id"), "kind": "episodic", "deleted": True}
    send({"jsonrpc": "2.0", "id": mid, "result": {"content": [{"type": "text", "text": json.dumps(payload)}], "structuredContent": payload, "isError": False}})
"#;

const BOT: &str = "agent://acme.example/support/bot";
const OTHER: &str = "agent://acme.example/support/other";

fn have_python() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The stand-in as an executable `engram-mcp`, or `None` (said) without python.
fn engram_mcp(dir: &Path) -> Option<PathBuf> {
    if !have_python() {
        eprintln!("SKIP engram stand-in: no python3 on PATH");
        return None;
    }
    let bin = script(dir, "engram-mcp", "exec python3 \"$0.py\" \"$@\"\n");
    std::fs::write(format!("{}.py", bin.display()), ENGRAM_MCP).expect("write the server");
    Some(bin)
}

#[test]
fn a_scoped_client_starts_with_its_agent_and_reads_only_that_agent() {
    let dir = scratch_dir("engram-scoped");
    let Some(bin) = engram_mcp(&dir) else {
        return;
    };
    let mut c =
        retry_busy(|| EngramClient::spawn(&bin, "/data/.engram", Some(BOT))).expect("spawn");
    assert_eq!(c.scope(), Some(BOT));
    assert_eq!(c.server_info()["serverInfo"]["name"], "engram-stand-in");
    assert_eq!(
        c.server_info()["argv"],
        serde_json::json!(["--db", "/data/.engram", "--agent-id", BOT]),
        "the store and the agent travel as flags, nothing through the environment"
    );

    let stats = c
        .stats(Some(BOT))
        .expect("stats for the server's own agent");
    assert_eq!((stats.entities, stats.counts.episodic), (4, 2));
    assert!(matches!(c.stats(Some(OTHER)), Err(McpError::Scope(_))));
    assert!(matches!(
        c.recall("q", 5, "hybrid", None),
        Err(McpError::Scope(_))
    ));

    let memories = c
        .recall("late refund", 7, "hybrid", Some(BOT))
        .expect("recall");
    assert_eq!(memories.len(), 1);
    assert_eq!(
        memories[0].content, "q=late refund limit=7 mode=hybrid keys=limit,mode,query",
        "a read names no agent to the server"
    );
    assert!(memories[0].actors.is_empty() && memories[0].tags.is_empty());

    match c.why("m1").expect("why") {
        EngramProvenance::Episodic {
            id, access_count, ..
        } => assert_eq!((id.as_str(), access_count), ("m1", 2)),
        other => panic!("expected an episodic provenance, got {other:?}"),
    }
    let forgot = c.forget("m1").expect("forget");
    assert!(forgot.deleted);
    assert_eq!(forgot.id, "m1");
    drop(c);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_whole_file_client_names_no_agent_and_surfaces_every_failure_shape() {
    let dir = scratch_dir("engram-whole");
    let Some(bin) = engram_mcp(&dir) else {
        return;
    };
    let mut c = retry_busy(|| EngramClient::spawn(&bin, "/data/.engram", None)).expect("spawn");
    assert_eq!(c.scope(), None);
    assert_eq!(
        c.server_info()["argv"],
        serde_json::json!(["--db", "/data/.engram"])
    );
    assert_eq!(c.stats(None).expect("stats").db_path, ":memory:");
    assert!(matches!(c.stats(Some(BOT)), Err(McpError::Scope(_))));

    match c.forget("rpc-error") {
        Err(McpError::Rpc { code, message }) => {
            assert_eq!((code, message.as_str()), (-32602, "no such memory"));
        }
        other => panic!("expected the server's RPC error, got {other:?}"),
    }
    match c.why("tool-error") {
        Err(McpError::Tool(m)) => assert!(m.contains("memory is locked"), "{m}"),
        other => panic!("expected the tool's own error, got {other:?}"),
    }
    match c.recall("wrong shape", 5, "hybrid", None) {
        Err(McpError::Protocol(m)) => assert!(m.starts_with("recall shape:"), "{m}"),
        other => panic!("a result of the wrong shape must not parse, got {other:?}"),
    }
    // The client is still usable after every refusal above.
    assert_eq!(c.recall("again", 1, "fts", None).expect("recall").len(), 1);
    drop(c);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_engram_binary_that_is_not_there_is_a_spawn_error() {
    let r = EngramClient::spawn(
        Path::new("/nonexistent/genaryx-test/engram-mcp"),
        "db",
        None,
    );
    assert!(matches!(r, Err(McpError::Spawn { .. })));
}
