//! Optimization-plane read tools (I10 "Felyx optimization recommendations"),
//! backed by the TokenFuse gateway CLI via `TokenfuseClient` - a blocking
//! shell-out, no long-lived state, so both tools use the SAME
//! `spawn_blocking` bridge `crypto_scan` already established for a CLI
//! connector (`crates/copilot/src/tools/crypto.rs`).
//!
//! Both results pass through `super::dollarize` before reaching the model,
//! same as `tools::cloud`'s money tools, so every money field here reaches
//! the model as a decimal `_usd` amount rather than a bare micro-USD
//! integer (`dollarize`'s own doc comment, `tools/mod.rs`, names the defect).
//!
//! Both tools are READ ONLY. Felyx can see the cost/savings numbers, but this
//! crate holds no signer (`crates/copilot/tests/no_signer.rs`), so it cannot
//! itself flip on caching, change a route, or touch gateway config - an
//! "enable the semantic cache" or "route this model to a cheaper one"
//! recommendation can only ever be INFORMATIONAL text in the model's answer.
//! A recommendation that DOES map to an existing signed action (e.g. "this
//! agent is burning budget with nothing to show for it, cap it") is left
//! entirely to the model's own judgment: it already has `propose_budget`
//! (`tools::propose`) available, and may call it after reading these numbers
//! if it judges that warranted. Neither tool here auto-emits a proposal -
//! that would be new, un-asked-for behavior these tools have no business
//! deciding on their own.
//!
//! `savings_breakdown` overlaps in SHAPE with `tools::cloud::Savings` (the
//! existing `savings` tool, sourced from Cloud's own `/v1/savings` ledger):
//! both report blocked/cache/router savings and budget breaks. This is a
//! genuine overlap worth the reviewing architect's attention, flagged here
//! rather than silently reconciled: the two tools read DIFFERENT sources -
//! `savings` requires the Cloud plane configured and reachable;
//! `savings_breakdown` reads the local TokenFuse Parquet trace directly via
//! the CLI, so it still works where Cloud is not configured (or serves as an
//! independent cross-check against it). `cost_per_action`'s per-model/
//! per-agent/per-tool-call breakdown has no existing equivalent anywhere in
//! this crate - that part is unambiguously new.

use async_trait::async_trait;
use serde_json::Value;

use genaryx_connectors::TokenfuseClient;

use super::{Clients, TokenfuseTraces, Tool, ToolError, dollarize, to_result};

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![Box::new(SavingsBreakdown), Box::new(CostPerAction)]
}

pub(super) struct SavingsBreakdown;

#[async_trait]
impl Tool for SavingsBreakdown {
    fn name(&self) -> &'static str {
        "savings_breakdown"
    }
    fn description(&self) -> &'static str {
        "FinOps savings read directly off the local TokenFuse call trace: runaway spend blocked \
         by budget protection, semantic-cache savings, model-router savings, budget breaks, and a \
         per-reason breakdown of blocked spend. Sourced from the TokenFuse gateway CLI itself (not \
         Cloud's ledger) - use for local/on-box savings questions, or as a cross-check against \
         `savings`. Check `trace_data_found` first: `false` means there is no trace data at all, \
         not that nothing was ever blocked."
    }
    async fn run(&self, clients: &Clients, _args: &Value) -> Result<Value, ToolError> {
        let traces = clients
            .tokenfuse
            .clone()
            .ok_or(ToolError::Unavailable("savings_breakdown"))?;
        let report = tokio::task::spawn_blocking(move || run_savings(&traces))
            .await
            .map_err(|e| ToolError::Connector {
                tool: "savings_breakdown",
                detail: e.to_string(),
            })?
            .map_err(|e| ToolError::Connector {
                tool: "savings_breakdown",
                detail: e.to_string(),
            })?;
        Ok(dollarize(to_result("savings_breakdown", report)?))
    }
}

pub(super) struct CostPerAction;

#[async_trait]
impl Tool for CostPerAction {
    fn name(&self) -> &'static str {
        "cost_per_action"
    }
    fn description(&self) -> &'static str {
        "Cost, call count, and tool-call totals from the local TokenFuse call trace, broken down \
         by model and by agent, including an average cost per tool call. `tool_calls` is only \
         recorded from I1 onward, so a row from an older trace reports it as UNKNOWN \
         (`tool_calls_known_rows == 0`) rather than zero - check that before reading \
         `cost_per_tool_call_usd` as real; it is `null` whenever the rate is not known or not \
         defined. Use to find which model or agent is expensive per unit of work."
    }
    async fn run(&self, clients: &Clients, _args: &Value) -> Result<Value, ToolError> {
        let traces = clients
            .tokenfuse
            .clone()
            .ok_or(ToolError::Unavailable("cost_per_action"))?;
        let report = tokio::task::spawn_blocking(move || run_cost_per_action(&traces))
            .await
            .map_err(|e| ToolError::Connector {
                tool: "cost_per_action",
                detail: e.to_string(),
            })?
            .map_err(|e| ToolError::Connector {
                tool: "cost_per_action",
                detail: e.to_string(),
            })?;
        Ok(dollarize(to_result("cost_per_action", report)?))
    }
}

/// The blocking body behind `savings_breakdown`, factored out so the unit
/// tests below can call it directly without going through `spawn_blocking`.
fn run_savings(
    traces: &TokenfuseTraces,
) -> Result<genaryx_connectors::TokenfuseSavings, genaryx_connectors::TokenfuseError> {
    TokenfuseClient::new(&traces.bin).savings(&traces.traces_dir)
}

/// The blocking body behind `cost_per_action`, factored out the same way.
fn run_cost_per_action(
    traces: &TokenfuseTraces,
) -> Result<genaryx_connectors::CostPerActionReport, genaryx_connectors::TokenfuseError> {
    TokenfuseClient::new(&traces.bin).cost_per_action(&traces.traces_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use genaryx_connectors::{CostBreakdownRow, CostPerActionReport, TokenfuseSavings};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    // Both tests below name the exact shaping `savings_breakdown` and
    // `cost_per_action` hand back to the model, run against the same
    // `to_result` + `dollarize` composition their `run` methods use (no live
    // `tokenfuse-gateway` binary needed to prove the shape). Each failed to
    // compile against the unfixed tree (`dollarize` did not exist yet).

    #[test]
    fn savings_breakdown_dollarizes_its_money_fields() {
        let mut by_reason = BTreeMap::new();
        by_reason.insert("budget_exceeded".to_string(), 3_000_000i64);
        let report = TokenfuseSavings {
            trace_data_found: true,
            blocked_spend_microusd: 3_500_000,
            blocked_calls: 4,
            budget_breaks_prevented: 2,
            cache_saved_microusd: 750_000,
            router_saved_microusd: 100_000,
            by_reason_microusd: by_reason,
        };
        let shaped = dollarize(to_result("savings_breakdown", report).unwrap());
        assert_eq!(shaped["blocked_spend_usd"], json!(3.5));
        assert_eq!(shaped["cache_saved_usd"], json!(0.75));
        assert_eq!(shaped["router_saved_usd"], json!(0.1));
        assert_eq!(shaped["by_reason_usd"]["budget_exceeded"], json!(3.0));
        // Non-money counts pass through untouched.
        assert_eq!(shaped["blocked_calls"], json!(4));
        assert!(shaped.get("blocked_spend_microusd").is_none());
        assert!(shaped.get("by_reason_microusd").is_none());
    }

    #[test]
    fn cost_per_action_dollarizes_its_money_fields_and_keeps_a_null_rate_null() {
        let report = CostPerActionReport {
            by_model: vec![CostBreakdownRow {
                label: "claude-opus".to_string(),
                calls: 20,
                total_cost_microusd: 28_750_000_404,
                total_tool_calls: 0,
                tool_calls_known_rows: 0,
                cost_per_tool_call_microusd: None,
            }],
            by_agent: vec![CostBreakdownRow {
                label: "planner".to_string(),
                calls: 5,
                total_cost_microusd: 1_000_000,
                total_tool_calls: 20,
                tool_calls_known_rows: 5,
                cost_per_tool_call_microusd: Some(50_000),
            }],
        };
        let shaped = dollarize(to_result("cost_per_action", report).unwrap());
        assert_eq!(
            shaped["by_model"][0]["total_cost_usd"],
            json!(28_750.000404)
        );
        assert_eq!(shaped["by_model"][0]["cost_per_tool_call_usd"], json!(null));
        assert_eq!(shaped["by_agent"][0]["total_cost_usd"], json!(1.0));
        assert_eq!(shaped["by_agent"][0]["cost_per_tool_call_usd"], json!(0.05));
        assert!(shaped["by_model"][0].get("total_cost_microusd").is_none());
    }

    fn clients_with_tokenfuse() -> Clients {
        Clients {
            tokenfuse: Some(TokenfuseTraces {
                bin: PathBuf::from("/x/tokenfuse-gateway-not-real"),
                traces_dir: PathBuf::from("/x/traces"),
            }),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn savings_breakdown_unavailable_when_no_tokenfuse_configured() {
        let err = SavingsBreakdown
            .run(&Clients::default(), &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Unavailable("savings_breakdown")));
    }

    #[tokio::test]
    async fn cost_per_action_unavailable_when_no_tokenfuse_configured() {
        let err = CostPerAction
            .run(&Clients::default(), &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Unavailable("cost_per_action")));
    }

    #[tokio::test]
    async fn savings_breakdown_surfaces_a_missing_binary_as_a_connector_error_not_a_panic() {
        // The binary does not exist, so the underlying `TokenfuseClient::savings`
        // fails closed (`TokenfuseError::Spawn`); the tool must turn that into a
        // `ToolError::Connector` for the model to see as data, never panic.
        let err = SavingsBreakdown
            .run(&clients_with_tokenfuse(), &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ToolError::Connector {
                tool: "savings_breakdown",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn cost_per_action_surfaces_a_missing_binary_as_a_connector_error_not_a_panic() {
        let err = CostPerAction
            .run(&clients_with_tokenfuse(), &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ToolError::Connector {
                tool: "cost_per_action",
                ..
            }
        ));
    }

    #[test]
    fn neither_tool_is_a_propose_tool() {
        assert!(!SavingsBreakdown.is_propose());
        assert!(!CostPerAction.is_propose());
    }

    // ---- live skip-graceful: exercises the real spawn_blocking bridge ------

    #[tokio::test]
    async fn live_tools_run_end_to_end_when_tokenfuse_is_installed() {
        let home = std::env::var("HOME").unwrap_or_default();
        let bin = PathBuf::from(format!("{home}/.taipan/bin/tokenfuse-gateway"));
        if !bin.is_file() {
            eprintln!(
                "SKIP live optimize-tools test: no tokenfuse-gateway at {}",
                bin.display()
            );
            return;
        }
        let candidates = [
            format!("{home}/.taipan/environments/p2exit.traces/gateway"),
            format!("{home}/.taipan/environments/p2gate.traces/gateway"),
        ];
        // `is_dir()` is not "populated": installing the stack creates
        // `<env>.traces/gateway` empty, so this took an empty directory and
        // failed the assert below instead of skipping. Same defect as
        // connectors/src/tokenfuse.rs had, in a copy of the same block.
        let Some(traces_dir) = candidates.iter().map(PathBuf::from).find(|p| {
            p.is_dir()
                && std::fs::read_dir(p)
                    .map(|mut d| d.next().is_some())
                    .unwrap_or(false)
        }) else {
            eprintln!(
                "SKIP live optimize-tools test: no populated traces dir among {candidates:?}"
            );
            return;
        };
        let clients = Clients {
            tokenfuse: Some(TokenfuseTraces { bin, traces_dir }),
            ..Default::default()
        };

        let savings = SavingsBreakdown
            .run(&clients, &json!({}))
            .await
            .expect("tokenfuse is installed but savings_breakdown errored");
        assert_eq!(savings["trace_data_found"], true);

        let cost = CostPerAction
            .run(&clients, &json!({}))
            .await
            .expect("tokenfuse is installed but cost_per_action errored");
        assert!(cost["by_model"].as_array().is_some_and(|a| !a.is_empty()));
    }
}
