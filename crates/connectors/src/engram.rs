//! `EngramClient`: a typed wrapper over [`crate::McpStdioClient`] for Engram's
//! memory plane (docs/PHASE4.md W2) - the plane the console's Memory panel
//! renders from. Grounded in engram's MCP server
//! (`~/Development/engram/engram/mcp_server.py`, read 2026-07-17): the console
//! script `engram-mcp --db <path> [--agent-id <id>]` runs FastMCP over stdio and
//! exposes five tools (`remember`, `recall`, `why`, `forget`, `stats`). This
//! wrapper knows those tools' argument and return shapes so the panel gets
//! typed data instead of raw JSON.
//!
//! ## Scope: observation first
//!
//! The Memory panel is primarily observational (docs/PHASE4.md W2:
//! "`stats`/`recall`/`why` + a timeline"), so this wrapper types the three
//! reads - [`EngramClient::stats`], [`EngramClient::recall`],
//! [`EngramClient::why`] - plus [`EngramClient::forget`] (the one plausible
//! admin action: erase a memory). `remember` is deliberately NOT wrapped:
//! agents write their own memories; a governance console does not fabricate
//! them. It remains reachable through the generic
//! [`crate::McpStdioClient::call_tool`] if ever needed.
//!
//! ## One long-lived server per client
//!
//! [`EngramClient::spawn`] starts one `engram-mcp` process and handshakes once;
//! every read reuses it. This matters for `recall`, which lazily loads a local
//! embedding model on first use (`mcp_server.py` `_EngramPool.get`), a cost
//! paid once per process rather than once per call. The caller resolves the
//! `engram-mcp` binary path (descriptor/checkout discovery is the env layer's
//! job, exactly like Idryx/Qryx); a missing binary surfaces as
//! [`crate::McpError::Spawn`], which the live test reads as "skip."
//!
//! ## Fail-closed
//!
//! Everything is [`crate::McpError`] on failure (spawn/io/protocol/rpc/tool/
//! timeout) - no panics. `why` on an unknown id returns the server's own
//! `isError` as [`crate::McpError::Tool`] ("memory not found"), never a
//! fabricated empty provenance.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::mcp_stdio::{McpError, McpStdioClient};

// ---- DTOs (exact tool return shapes, engram/mcp_server.py) ------------------

/// `stats` return (`_stats`, `mcp_server.py:281-314`). Store-wide counts for
/// the effective agent scope.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct EngramStats {
    /// The effective agent scope these counts are for (the server's default
    /// when the call passed none); `None` only if the server itself has no
    /// default agent configured.
    #[serde(default)]
    pub agent_id: Option<String>,
    pub counts: EngramCounts,
    pub vector_index_size: i64,
    pub facts_total: i64,
    pub facts_active: i64,
    pub facts_superseded: i64,
    pub entities: i64,
    pub reflections: i64,
    pub db_path: String,
    /// The DB file size in bytes, or `None` for an in-memory store
    /// (`:memory:`) or a file that does not exist yet.
    #[serde(default)]
    pub db_size_bytes: Option<i64>,
}

/// Per-kind memory counts (`counts` in `_stats`). `procedural` is always 0 in
/// this Engram version (the store implements only episodic + semantic);
/// `mcp_server.py:302-304` says so verbatim, so the panel labels it "not
/// implemented," never a real zero.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct EngramCounts {
    pub episodic: i64,
    pub semantic: i64,
    pub procedural: i64,
}

/// One `recall` hit (`_recall`, `mcp_server.py:184-194`). Ranked by relevance,
/// most relevant first.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct EngramMemory {
    pub id: String,
    pub content: String,
    /// Relevance score for the query (higher is more relevant).
    pub score: f64,
    pub importance: f64,
    /// UTC ISO-8601 encoding time (`episode.timestamp.isoformat()`).
    pub timestamp: String,
    #[serde(default)]
    pub actors: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A memory's provenance from `why` (`_why`, `mcp_server.py:198-257`). The tool
/// returns one of two shapes discriminated by `kind`: a semantic fact (with its
/// extraction chain) or an episodic observation (with encoding + access
/// metadata). Modeled as a tagged enum so the panel matches on the variant.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum EngramProvenance {
    /// An LLM-derived fact triple with its full extraction lineage.
    Semantic {
        id: String,
        subject: String,
        predicate: String,
        object: String,
        confidence: f64,
        valid_from: String,
        #[serde(default)]
        valid_to: Option<String>,
        recorded_at: String,
        #[serde(default)]
        extracted_from: Option<String>,
        #[serde(default)]
        extracted_by_reflection_run: Option<String>,
        #[serde(default)]
        extraction_model: Option<String>,
    },
    /// A raw observation: no extraction chain (it is not LLM-derived), so this
    /// carries encoding + access metadata instead.
    Episodic {
        id: String,
        content: String,
        timestamp: String,
        #[serde(default)]
        actors: Vec<String>,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        salience: Option<f64>,
        #[serde(default)]
        emotional_valence: Option<f64>,
        #[serde(default)]
        importance_score: Option<f64>,
        /// The episode ids this episode summarizes. Always a list on the wire
        /// (`Episode.summary_of: list[str]`, `engram/models.py:24`, default
        /// `[]`), NOT a scalar - typing it as a single string made every
        /// episodic `why` fail to deserialize (caught live against real
        /// `engram-mcp`).
        #[serde(default)]
        summary_of: Vec<String>,
        #[serde(default)]
        agent_id: Option<String>,
        access_count: i64,
        #[serde(default)]
        last_accessed: Option<String>,
        #[serde(default)]
        note: String,
    },
}

/// The result of [`EngramClient::forget`] (`_forget`, `mcp_server.py:260-278`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ForgetResult {
    pub id: String,
    /// `episodic` or `semantic` - which store the id was found in.
    pub kind: String,
    pub deleted: bool,
}

// ---- client ----------------------------------------------------------------

/// A typed Engram memory-plane client. Owns one long-lived `engram-mcp` process
/// (via [`McpStdioClient`]); every method is one MCP `tools/call`. Methods take
/// `&mut self` because the underlying JSON-RPC client advances a request id per
/// call - the shells hold it behind a mutex, like every other stateful handle.
#[derive(Debug)]
pub struct EngramClient {
    mcp: McpStdioClient,
    /// The agent this server is started for (`--agent-id`), or `None` for the
    /// whole file. A read is checked against it before anything is sent.
    scope: Option<String>,
}

impl EngramClient {
    /// Spawn `engram-mcp --db <db_path> [--agent-id <agent_id>]` and handshake.
    /// `agent_id` is the agent this server serves; `None` serves the whole file.
    /// `engram_mcp_bin` is the resolved console-script path. `db_path` is the
    /// engram store to read; passing a real file (not `:memory:`) is what makes
    /// the reads meaningful across restarts.
    pub fn spawn(
        engram_mcp_bin: &Path,
        db_path: &str,
        agent_id: Option<&str>,
    ) -> Result<Self, McpError> {
        let bin = engram_mcp_bin.to_string_lossy();
        let owned = spawn_args(db_path, agent_id);
        let args: Vec<&str> = owned.iter().map(String::as_str).collect();
        // The env map is empty: everything engram-mcp needs is passed as an
        // explicit flag above, never smuggled through inherited env, so the
        // console's own environment cannot silently redirect the store.
        let env = BTreeMap::new();
        let mcp = McpStdioClient::spawn(&bin, &args, &env)?;
        Ok(Self {
            mcp,
            scope: agent_id.map(str::to_string),
        })
    }

    /// The agent this server is scoped to, or `None` for the whole file.
    pub fn scope(&self) -> Option<&str> {
        self.scope.as_deref()
    }

    /// The `serverInfo`/`capabilities` the server reported at `initialize`.
    pub fn server_info(&self) -> &serde_json::Value {
        self.mcp.server_info()
    }

    /// `stats` -> store-wide counts for the agent this server is scoped to (the
    /// whole file when it is not). `agent_id` names the agent the caller asks
    /// for, and it must be this server's own, or the call is refused before it
    /// is sent (see [`stats_args`]).
    pub fn stats(&mut self, agent_id: Option<&str>) -> Result<EngramStats, McpError> {
        let args = stats_args(agent_id, self.scope.as_deref())?;
        let v = self.mcp.call_tool("stats", args)?;
        serde_json::from_value(v).map_err(|e| McpError::Protocol(format!("stats shape: {e}")))
    }

    /// `recall` -> up to `limit` memories relevant to `query`, most relevant
    /// first. `mode` is `cosine` (default embedding similarity), `spreading`
    /// (follows graph edges), or `hybrid` (embedding + BM25). `agent_id` is the
    /// agent the caller asks for, which must be this server's own (see
    /// [`recall_args`]); the server's scope does the scoping.
    pub fn recall(
        &mut self,
        query: &str,
        limit: u32,
        mode: &str,
        agent_id: Option<&str>,
    ) -> Result<Vec<EngramMemory>, McpError> {
        let args = recall_args(query, limit, mode, agent_id, self.scope.as_deref())?;
        let v = self.mcp.call_tool("recall", args)?;
        serde_json::from_value(v).map_err(|e| McpError::Protocol(format!("recall shape: {e}")))
    }

    /// `why` -> the provenance of the memory with `memory_id`. An unknown id is
    /// the server's own `isError` -> [`McpError::Tool`] ("memory not found"),
    /// never a fabricated empty result.
    pub fn why(&mut self, memory_id: &str) -> Result<EngramProvenance, McpError> {
        let v = self
            .mcp
            .call_tool("why", json!({ "memory_id": memory_id }))?;
        serde_json::from_value(v).map_err(|e| McpError::Protocol(format!("why shape: {e}")))
    }

    /// `forget` -> permanently delete the memory with `memory_id` (irreversible;
    /// the one plausible admin mutation the Memory panel exposes). An unknown id
    /// surfaces as [`McpError::Tool`].
    pub fn forget(&mut self, memory_id: &str) -> Result<ForgetResult, McpError> {
        let v = self
            .mcp
            .call_tool("forget", json!({ "memory_id": memory_id }))?;
        serde_json::from_value(v).map_err(|e| McpError::Protocol(format!("forget shape: {e}")))
    }
}

// ---- the read scope (Engram F-007) -----------------------------------------
//
// A server started with `--agent-id` serves that agent and refuses a call that
// names any other. A server started without one serves the whole file and
// refuses a call that names any agent. So a read names no agent to the server
// at all: the server's own scope answers it. A read for an agent its server is
// not scoped to is refused here, before anything is sent.

/// The `engram-mcp` arguments after the binary, as `spawn` passes them: `--db`,
/// then `--agent-id` only when the server is to be scoped to one agent.
pub(crate) fn spawn_args(db_path: &str, agent_id: Option<&str>) -> Vec<String> {
    let mut args = vec!["--db".to_string(), db_path.to_string()];
    if let Some(id) = agent_id {
        args.push("--agent-id".to_string());
        args.push(id.to_string());
    }
    args
}

/// Refuse a read that names an agent its server is not scoped to. The names
/// must be the same: a whole-file read against a per-agent server is refused
/// too, and so is a per-agent read against the whole-file server.
pub(crate) fn check_scope(asked: Option<&str>, scope: Option<&str>) -> Result<(), McpError> {
    if asked == scope {
        return Ok(());
    }
    Err(McpError::Scope(format!(
        "this engram-mcp serves {}, not {}: a read names the agent its server is scoped to",
        scope.unwrap_or("the whole file"),
        asked.unwrap_or("the whole file"),
    )))
}

/// The wire arguments of `stats`. The server's own scope answers the read, so no
/// agent goes on the wire; a named agent must be the server's own (see
/// [`check_scope`]).
pub(crate) fn stats_args(
    asked: Option<&str>,
    scope: Option<&str>,
) -> Result<serde_json::Value, McpError> {
    check_scope(asked, scope)?;
    Ok(json!({}))
}

/// The wire arguments of `recall`: the query, the limit and the mode, never an
/// agent (see [`stats_args`]).
pub(crate) fn recall_args(
    query: &str,
    limit: u32,
    mode: &str,
    asked: Option<&str>,
    scope: Option<&str>,
) -> Result<serde_json::Value, McpError> {
    check_scope(asked, scope)?;
    Ok(json!({ "query": query, "limit": limit, "mode": mode }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Offline shape tests: the exact JSON engram's tools return, deserialized
    // into the typed DTOs. A live test against a real `engram-mcp` process lives
    // in tests/, skip-gracefully when the binary is absent.

    #[test]
    fn stats_shape_parses_with_null_db_size() {
        let json = serde_json::json!({
            "agent_id": "agent://acme/support",
            "counts": {"episodic": 12, "semantic": 5, "procedural": 0},
            "vector_index_size": 17,
            "facts_total": 8,
            "facts_active": 5,
            "facts_superseded": 3,
            "entities": 4,
            "reflections": 2,
            "db_path": ":memory:",
            "db_size_bytes": null
        });
        let s: EngramStats = serde_json::from_value(json).expect("parse stats");
        assert_eq!(s.counts.episodic, 12);
        assert_eq!(s.counts.procedural, 0);
        assert_eq!(s.facts_superseded, 3);
        assert!(s.db_size_bytes.is_none());
    }

    #[test]
    fn recall_shape_parses_a_ranked_list() {
        let json = serde_json::json!([
            {"id":"m1","content":"paid invoice","score":0.91,"importance":0.5,
             "timestamp":"2026-07-17T10:00:00+00:00","actors":["alice"],"tags":["billing"]},
            {"id":"m2","content":"refund issued","score":0.72,"importance":0.3,
             "timestamp":"2026-07-17T09:00:00+00:00","actors":[],"tags":[]}
        ]);
        let ms: Vec<EngramMemory> = serde_json::from_value(json).expect("parse recall");
        assert_eq!(ms.len(), 2);
        assert_eq!(ms[0].id, "m1");
        assert!(ms[0].score > ms[1].score, "ranked most-relevant first");
        assert_eq!(ms[0].tags, vec!["billing"]);
    }

    #[test]
    fn why_semantic_and_episodic_variants_discriminate_on_kind() {
        let semantic = serde_json::json!({
            "kind":"semantic","id":"f1","subject":"acme","predicate":"owes","object":"$100",
            "confidence":0.9,"valid_from":"2026-01-01T00:00:00+00:00","valid_to":null,
            "recorded_at":"2026-07-17T10:00:00+00:00","extracted_from":"ep-3",
            "extracted_by_reflection_run":"run-7","extraction_model":"claude-sonnet-5"
        });
        match serde_json::from_value::<EngramProvenance>(semantic).expect("semantic") {
            EngramProvenance::Semantic {
                subject,
                predicate,
                extraction_model,
                ..
            } => {
                assert_eq!(subject, "acme");
                assert_eq!(predicate, "owes");
                assert_eq!(extraction_model.as_deref(), Some("claude-sonnet-5"));
            }
            other => panic!("expected Semantic, got {other:?}"),
        }

        let episodic = serde_json::json!({
            "kind":"episodic","id":"e1","content":"observed a login","timestamp":"2026-07-17T10:00:00+00:00",
            "actors":["bot"],"tags":["auth"],"salience":0.4,"emotional_valence":0.0,
            "importance_score":0.2,"summary_of":["ep-1","ep-2"],"agent_id":"agent://acme/x",
            "access_count":3,"last_accessed":"2026-07-17T11:00:00+00:00","note":"raw observation"
        });
        match serde_json::from_value::<EngramProvenance>(episodic).expect("episodic") {
            EngramProvenance::Episodic {
                access_count,
                content,
                summary_of,
                ..
            } => {
                assert_eq!(access_count, 3);
                assert_eq!(content, "observed a login");
                // summary_of is a list on the wire, not a scalar.
                assert_eq!(summary_of, vec!["ep-1", "ep-2"]);
            }
            other => panic!("expected Episodic, got {other:?}"),
        }
    }

    // ---- the read scope (Engram F-007) -------------------------------------
    // A whole-file server (no --agent-id) and a per-agent server (--agent-id)
    // are two processes. A read names no agent to either, and a read naming an
    // agent its server is not scoped to is refused before it is sent.

    const BOT: &str = "agent://acme.example/support/bot";
    const OTHER: &str = "agent://acme.example/support/other";

    #[test]
    fn a_whole_file_server_starts_with_no_agent() {
        assert_eq!(
            spawn_args("/data/.engram", None),
            vec!["--db", "/data/.engram"]
        );
    }

    #[test]
    fn a_per_agent_server_starts_with_its_agent() {
        assert_eq!(
            spawn_args("/data/.engram", Some(BOT)),
            vec!["--db", "/data/.engram", "--agent-id", BOT]
        );
    }

    #[test]
    fn a_read_naming_another_agent_than_the_servers_is_refused_before_the_call() {
        assert!(matches!(
            check_scope(Some(OTHER), Some(BOT)),
            Err(McpError::Scope(_))
        ));
        assert!(matches!(
            check_scope(Some(BOT), None),
            Err(McpError::Scope(_))
        ));
        assert!(matches!(
            stats_args(Some(OTHER), Some(BOT)),
            Err(McpError::Scope(_))
        ));
        assert!(matches!(
            recall_args("q", 5, "hybrid", Some(OTHER), Some(BOT)),
            Err(McpError::Scope(_))
        ));
    }

    #[test]
    fn a_read_never_names_an_agent_to_the_server() {
        let stats = stats_args(Some(BOT), Some(BOT)).expect("the server's own agent");
        assert!(stats.get("agent_id").is_none(), "stats wire args: {stats}");
        let recall =
            recall_args("q", 5, "hybrid", Some(BOT), Some(BOT)).expect("the server's own agent");
        assert!(
            recall.get("agent_id").is_none(),
            "recall wire args: {recall}"
        );
    }

    #[test]
    fn a_whole_file_read_carries_no_agent_and_no_provenance() {
        assert_eq!(stats_args(None, None).expect("whole file"), json!({}));
        assert_eq!(
            recall_args("q", 5, "hybrid", None, None).expect("whole file"),
            json!({"query": "q", "limit": 5, "mode": "hybrid"})
        );
    }

    #[test]
    fn forget_result_parses() {
        let json = serde_json::json!({"id":"m1","kind":"episodic","deleted":true});
        let f: ForgetResult = serde_json::from_value(json).expect("parse forget");
        assert!(f.deleted);
        assert_eq!(f.kind, "episodic");
    }
}
