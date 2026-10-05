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
//!
//! I10 ("Felyx optimization recommendations") adds `optimize`: two more read
//! tools over the TokenFuse gateway CLI (`savings_breakdown`/
//! `cost_per_action`), shelled fresh per call via the SAME `spawn_blocking`
//! bridge `crypto_scan` uses (a CLI has no long-lived state to hold). Still
//! READ only - Felyx can see the cost/savings numbers, but proposing a
//! concrete action from them (e.g. capping a wasteful agent's budget) goes
//! through the model choosing to call an EXISTING `tools::propose` tool, not
//! a new one; see `crates/copilot/src/tools/optimize.rs`'s module doc for the
//! full rationale, including why this overlaps in shape (but not source)
//! with `tools::cloud`'s existing `savings` tool.

mod budgets;
mod cloud;
mod crypto;
mod idryx;
mod memory;
mod optimize;
mod propose;
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
    /// The resolved tokenfuse gateway binary plus its traces dir
    /// (`TOKENFUSE_DATA_DIR`), resolved once at bootstrap (see
    /// `crates/api/src/copilot/state.rs`'s `resolve_clients`).
    /// `optimize::tools()` shells `tokenfuse` fresh per call inside a
    /// blocking task - the same "no long-lived state, just a resolved path"
    /// pattern as `qryx_bin` - reading `traces_dir` rather than accepting it
    /// as a model-supplied argument: unlike `crypto_scan`'s `path` (an
    /// arbitrary filesystem target Qryx is meant to scan on request), the
    /// trace directory is a fixed environment fact, not something the model
    /// should be able to redirect.
    pub tokenfuse: Option<TokenfuseTraces>,
}

/// The resolved TokenFuse binary + traces dir pair [`Clients::tokenfuse`]
/// holds; see its doc comment for why both are fixed at bootstrap rather than
/// model-supplied.
#[derive(Debug, Clone)]
pub struct TokenfuseTraces {
    pub bin: PathBuf,
    pub traces_dir: PathBuf,
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
    /// A "propose" tool (C2) returns a `ProposedAction` rather than reading a
    /// plane; the loop collects its result into `Answer.proposals` for the shell
    /// to render as an approve/reject card. Read tools leave this `false`.
    fn is_propose(&self) -> bool {
        false
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
            tools.extend(budgets::tools());
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
        if clients.tokenfuse.is_some() {
            tools.extend(optimize::tools());
        }
        // Propose tools (C2) are always available: a proposal is a descriptor,
        // not a plane read, so the copilot can recommend an action even where a
        // plane is unconfigured. They emit a `ProposedAction` and never mutate;
        // their Wardryx pre-check is best-effort (uses `clients.wardryx` if set).
        tools.extend(propose::tools());
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

    /// Whether the named tool is a "propose" tool (C2), so the loop knows to
    /// collect its result into `Answer.proposals`. Unknown names are `false`.
    pub fn is_propose_tool(&self, name: &str) -> bool {
        self.tools
            .iter()
            .find(|t| t.name() == name)
            .is_some_and(|t| t.is_propose())
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

/// Convert every money value a tool hands to the model from a bare micro-USD
/// integer into a decimal amount under a key that says "usd", so the model
/// never has to do the arithmetic (or guess the unit) itself.
///
/// The defect this exists for, 2026-09-27: asked "Which planes can you see,
/// and is each one healthy?", Felyx answered "Run `genaryx-copilot` exceeded
/// its $50 budget by 27% ($63.64 spent)". The Cloud's own figures were
/// `budget_micros: 50000` (five cents) and `spent_microusd: 63640` (about
/// six and a third cents) - both read straight off a `genaryx_connectors`
/// DTO and handed to the model as bare integers, with only the tool's prose
/// description (not the JSON itself) saying the unit was microdollars. In
/// the SAME session, a different question about the SAME run's alert got
/// the conversion right ("spent $0.0547 on a $0.05 budget"): the model can
/// do this arithmetic, it is just not required to, and a value that is only
/// sometimes converted correctly is a defect even on the calls where it
/// happens to come out right.
///
/// Every `genaryx_connectors` DTO these tools serialize keeps its wire-exact
/// field names (`crates/connectors/src/cloud_rest.rs`'s own doc: read
/// directly off the Cloud's `store.rs`, not guessed), so this walks the
/// JSON produced by [`to_result`] rather than touching those structs - the
/// same "wire DTO stays wire-exact, a separate type carries the converted
/// value" split `crates/api/src/money/commands.rs` already draws for the web
/// frontend (its own `micros_to_usd`, this function's sibling on the other
/// side of a crate boundary this crate cannot depend across).
///
/// Recurses through every object and array, so a tool need not call this per
/// field: build the result through [`to_result`] as always, then wrap it in
/// `dollarize`. A key that names a nested reason -> amount map
/// (`by_reason_microusd`) is walked one level further, since there the unit
/// lives in the outer key, not a sibling field.
pub(super) fn dollarize(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (key, val) in map {
                match usd_key_for(&key) {
                    Some(usd_key) => {
                        out.insert(usd_key, micro_leaves_to_usd(val));
                    }
                    None => {
                        out.insert(key, dollarize(val));
                    }
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(dollarize).collect()),
        other => other,
    }
}

/// The value under a matched micro-money key: ordinarily a bare integer
/// amount, or (for `by_reason_microusd`) a map of amounts. Converts every
/// integer found; a `null` (an `Option<i64>` amount with nothing known, e.g.
/// `cost_per_tool_call_microusd`) stays `null` under the renamed key, since
/// "no rate known" and "zero dollars" are different answers a model must
/// not be left to conflate.
fn micro_leaves_to_usd(value: Value) -> Value {
    match value {
        Value::Number(n) => n
            .as_i64()
            .map(micros_to_usd_json)
            .unwrap_or(Value::Number(n)),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, micro_leaves_to_usd(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(micro_leaves_to_usd).collect()),
        other => other,
    }
}

/// Which key a raw micro-money key becomes, on the way to the model. An
/// ALLOW-LIST, not a bare `_micros` suffix match: `budget_micros`
/// (`Alert`/`BudgetResponse`, `store.rs`'s own wire name) is the one field in
/// this whole contract that means microUSD without saying "usd" anywhere in
/// its own name, so it is named explicitly here rather than matched by a
/// suffix a future, unrelated `..._micros` field (a duration, say) could
/// also match.
fn usd_key_for(key: &str) -> Option<String> {
    if let Some(base) = key.strip_suffix("_microusd") {
        return Some(format!("{base}_usd"));
    }
    if key == "budget_micros" {
        return Some("budget_usd".to_string());
    }
    None
}

/// `crates/api/src/money/commands.rs`'s own `micros_to_usd`, repeated here
/// because this crate cannot depend on `genaryx-api` (the same "second
/// independent reader" shape CLAUDE.md's invariant 13 already sanctions for
/// `GENARYX_ORG_DOMAIN`/`TOKENFUSE_GATEWAY_ADMIN_KEY`), returned already as
/// the `Value` this module's callers need.
fn micros_to_usd_json(micros: i64) -> Value {
    serde_json::Number::from_f64(micros as f64 / 1_000_000.0)
        .map(Value::Number)
        .unwrap_or_else(|| Value::Number(0.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_gates_read_tools_on_planes_but_always_offers_propose_tools() {
        // No connector clients -> no READ tools, but the propose tools (C2) are
        // always available (a proposal is a descriptor, not a plane read).
        let reg = ToolRegistry::new(Clients::default());
        let names = reg.tool_names();
        assert!(!names.contains(&"money_summary")); // no cloud -> no read tools
        assert!(!names.contains(&"memory_recall"));
        assert!(!names.contains(&"savings_breakdown")); // no tokenfuse -> no optimize tools
        assert!(!names.contains(&"cost_per_action"));
        assert!(names.contains(&"propose_kill")); // propose tools always present
        assert!(names.contains(&"propose_budget"));
        assert!(reg.is_propose_tool("propose_kill"));
        assert!(!reg.is_propose_tool("money_summary"));
        assert!(!reg.is_propose_tool("does_not_exist"));
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

    // ---- dollarize: every money value reaches the model as USD ------------
    //
    // The regression these hold: 2026-09-27, Felyx read `budget_micros:
    // 50000` and `spent_microusd: 63640` straight off a connector DTO and
    // told an operator a run had "exceeded its $50 budget ... $63.64
    // spent" - three orders of magnitude off both figures. Each test below
    // was run against the unfixed tree first: `dollarize` did not exist, so
    // every one of these failed to compile (the same "red as a compile
    // failure" shape CLAUDE.md's invariants 10, 13 and 14 already record).

    #[test]
    fn dollarize_converts_a_microusd_field_to_a_decimal_usd_field() {
        let summary = json!({"runs": 16u64, "calls": 91u64, "spent_microusd": 167_964i64});
        let shaped = dollarize(summary);
        assert_eq!(shaped["spent_usd"], json!(0.167_964));
        assert!(
            shaped.get("spent_microusd").is_none(),
            "the bare micro field must not also reach the model: {shaped}"
        );
    }

    #[test]
    fn dollarize_converts_the_allow_listed_budget_micros_key_without_a_usd_suffix() {
        // `Alert::budget_micros` is the one field in this contract that
        // means microUSD without saying "usd" in its own name at all.
        let alert = json!({
            "run_id": "genaryx-copilot",
            "spent_microusd": 63_640i64,
            "budget_micros": 50_000i64,
            "fraction": 1.2728,
            "killed": false,
        });
        let shaped = dollarize(alert);
        assert_eq!(shaped["spent_usd"], json!(0.063_64));
        assert_eq!(shaped["budget_usd"], json!(0.05));
        assert!(shaped.get("budget_micros").is_none());
        assert!(shaped.get("spent_microusd").is_none());
        // Non-money fields pass through untouched.
        assert_eq!(shaped["run_id"], json!("genaryx-copilot"));
        assert_eq!(shaped["killed"], json!(false));
    }

    #[test]
    fn the_real_defect_amounts_convert_to_the_dollars_the_cloud_actually_recorded() {
        // The exact numbers from the 2026-09-27 evidence log: a run whose
        // real budget was five cents and real spend about six and a third
        // cents, which Felyx reported as a $50 budget and $63.64 spent.
        let shaped = dollarize(json!({"spent_microusd": 63_640i64, "budget_micros": 50_000i64}));
        assert_eq!(shaped["spent_usd"], json!(0.063_64));
        assert_eq!(shaped["budget_usd"], json!(0.05));
    }

    #[test]
    fn dollarize_converts_every_row_of_an_array_and_the_wrapping_total() {
        // `list_runs`' own wrapper shape (`top_runs_by_spend`, `cloud.rs`):
        // a top-level total beside a `runs` array of per-run rows.
        let wrapped = json!({
            "total_runs": 2u64,
            "total_spent_microusd": 17_000i64,
            "runs": [
                {"run_id": "r1", "spent_microusd": 9_000i64},
                {"run_id": "r2", "spent_microusd": 8_000i64},
            ],
        });
        let shaped = dollarize(wrapped);
        assert_eq!(shaped["total_spent_usd"], json!(0.017));
        assert_eq!(shaped["runs"][0]["spent_usd"], json!(0.009));
        assert_eq!(shaped["runs"][1]["spent_usd"], json!(0.008));
    }

    #[test]
    fn dollarize_walks_into_a_nested_reason_to_amount_map() {
        // `TokenfuseSavings::by_reason_microusd`: the unit lives in the
        // OUTER key, and the map's own keys are reason strings, not field
        // names ending in a money suffix.
        let savings = json!({
            "blocked_spend_microusd": 3_500_000i64,
            "by_reason_microusd": {
                "budget_exceeded": 3_000_000i64,
                "loop_detected": 500_000i64,
            },
        });
        let shaped = dollarize(savings);
        assert_eq!(shaped["blocked_spend_usd"], json!(3.5));
        assert_eq!(shaped["by_reason_usd"]["budget_exceeded"], json!(3.0));
        assert_eq!(shaped["by_reason_usd"]["loop_detected"], json!(0.5));
        assert!(shaped.get("by_reason_microusd").is_none());
    }

    #[test]
    fn dollarize_leaves_a_null_amount_as_null_under_the_renamed_key() {
        // `cost_per_tool_call_microusd: None` means "the rate is not known",
        // never "zero dollars"; the renamed key must carry that same null,
        // not a computed 0.0 a model could read as a real answer.
        let row = json!({"cost_per_tool_call_microusd": null});
        let shaped = dollarize(row);
        assert_eq!(shaped["cost_per_tool_call_usd"], json!(null));
    }

    #[test]
    fn dollarize_never_misreads_an_unrelated_micros_field_as_money() {
        // The allow-list (not a bare `_micros` suffix) is the point: a
        // hypothetical future duration field must not silently become a
        // dollar amount just because its name ends the same way.
        let row = json!({"latency_micros": 42_000i64});
        let shaped = dollarize(row);
        assert_eq!(shaped["latency_micros"], json!(42_000));
        assert!(shaped.get("latency_usd").is_none());
    }

    #[test]
    fn i10_optimize_tools_register_from_a_resolved_tokenfuse_client() {
        // Like qryx/verdryx, tokenfuse is backed by resolved paths in
        // `Clients` (no live binary/traces needed to prove registration).
        let reg = ToolRegistry::new(Clients {
            tokenfuse: Some(TokenfuseTraces {
                bin: PathBuf::from("/x/tokenfuse-gateway"),
                traces_dir: PathBuf::from("/x/traces"),
            }),
            ..Default::default()
        });
        let names = reg.tool_names();
        assert!(names.contains(&"savings_breakdown"));
        assert!(names.contains(&"cost_per_action"));
        assert!(!reg.is_propose_tool("savings_breakdown"));
        assert!(!reg.is_propose_tool("cost_per_action"));
    }
}
