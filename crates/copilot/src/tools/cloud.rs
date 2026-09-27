//! Money-plane read tools, backed by `CloudClient` (all `async`, bearer-auth).
//!
//! Every money value these tools hand to the model passes through
//! `super::dollarize` first, so it reaches the model as a decimal `_usd`
//! amount rather than a bare micro-USD integer under a key that may or may
//! not say so - see `dollarize`'s own doc comment (`tools/mod.rs`) for the
//! 2026-09-27 defect this closes.

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{Clients, Tool, ToolError, dollarize, to_result};

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(MoneySummary),
        Box::new(ListRuns),
        Box::new(ListAgents),
        Box::new(Savings),
        Box::new(Incidents),
        Box::new(Alerts),
    ]
}

/// Fetch the `cloud` client or report the plane unavailable.
macro_rules! cloud {
    ($clients:expr, $name:literal) => {
        $clients
            .cloud
            .as_ref()
            .ok_or(ToolError::Unavailable($name))?
    };
}

macro_rules! read_tool {
    ($ty:ident, $name:literal, $desc:literal, $method:ident) => {
        pub(super) struct $ty;
        #[async_trait]
        impl Tool for $ty {
            fn name(&self) -> &'static str {
                $name
            }
            fn description(&self) -> &'static str {
                $desc
            }
            async fn run(&self, clients: &Clients, _args: &Value) -> Result<Value, ToolError> {
                let data =
                    cloud!(clients, $name)
                        .$method()
                        .await
                        .map_err(|e| ToolError::Connector {
                            tool: $name,
                            detail: e.to_string(),
                        })?;
                Ok(dollarize(to_result($name, data)?))
            }
        }
    };
}

read_tool!(
    MoneySummary,
    "money_summary",
    "Org-wide totals: number of runs, calls, and total spend (spent_usd, decimal USD). Use for headline spend questions.",
    summary
);
read_tool!(
    ListAgents,
    "list_agents",
    "Per-agent spend rollup, highest spend first. Use to attribute spend to agents.",
    agents
);
read_tool!(
    Savings,
    "savings",
    "FinOps savings totals: budget-blocked spend, cache and router savings, budget breaks, and the total governed savings.",
    savings
);
read_tool!(
    Alerts,
    "alerts",
    "Runs at or above their budget alert threshold (run id, spent, budget, fraction of budget, whether killed). Use to find near-cap and over-cap runs.",
    alerts
);

/// The most rows any list tool hands back to the model in one call. The planes
/// can hold thousands of rows (a real fleet is ~9k runs, which serializes to
/// ~1M tokens - enough to blow the model's context window). A tool must never
/// dump an unbounded plane into the prompt, so the large lists return the most
/// decision-relevant rows plus the TRUE total, keeping the model both within
/// budget and honest about what it is not seeing.
const MAX_ROWS: usize = 30;

/// `list_runs`: the per-run spend rollup, but bounded. `/v1/runs` returns EVERY
/// run for the org; we sort by spend and keep the top [`MAX_ROWS`], wrapping
/// them with the org-wide run count and summed spend. That is both safe on a
/// large fleet and more useful (the top spenders are what a kill/budget question
/// is about); `money_summary` remains the headline-totals tool.
pub(super) struct ListRuns;
#[async_trait]
impl Tool for ListRuns {
    fn name(&self) -> &'static str {
        "list_runs"
    }
    fn description(&self) -> &'static str {
        "Top per-run spenders for the org, highest spend first (run id, model, agent, spent_usd, \
         calls, steps, whether killed), plus the org-wide run count and total spend. Safe on a fleet \
         of thousands (returns only the top runs). Use `money_summary` for headline totals and \
         `incidents` for budget breaks / runaways."
    }
    async fn run(&self, clients: &Clients, _args: &Value) -> Result<Value, ToolError> {
        let runs = cloud!(clients, "list_runs")
            .runs()
            .await
            .map_err(|e| ToolError::Connector {
                tool: "list_runs",
                detail: e.to_string(),
            })?;
        Ok(dollarize(top_runs_by_spend(to_result("list_runs", runs)?)))
    }
}

/// `incidents`, but bounded to [`MAX_ROWS`] newest-first (the connector already
/// orders them), wrapped with the true total so a long incident list can never
/// blow the context either.
pub(super) struct Incidents;
#[async_trait]
impl Tool for Incidents {
    fn name(&self) -> &'static str {
        "incidents"
    }
    fn description(&self) -> &'static str {
        "Open incidents for the org, newest first (id, kind, severity, run/agent, occurrences, \
         acknowledged). Returns the most recent incidents plus the true total."
    }
    async fn run(&self, clients: &Clients, _args: &Value) -> Result<Value, ToolError> {
        let incidents = cloud!(clients, "incidents")
            .incidents()
            .await
            .map_err(|e| ToolError::Connector {
                tool: "incidents",
                detail: e.to_string(),
            })?;
        Ok(cap_rows("incidents", to_result("incidents", incidents)?))
    }
}

/// Sort a runs array by `spent_microusd` desc, keep the top [`MAX_ROWS`], and
/// wrap with the true total count + summed spend. A non-array value (or one
/// already within budget) is wrapped verbatim with its totals.
///
/// Runs BEFORE `dollarize` (its caller applies that after), so this still
/// reads and sums the connector's own raw `spent_microusd` integers - exact
/// integer arithmetic for the sort key and the sum, converted to a decimal
/// `total_spent_usd` only once, at the end, by the shared helper.
fn top_runs_by_spend(data: Value) -> Value {
    let Value::Array(mut rows) = data else {
        return data;
    };
    let total_runs = rows.len();
    let total_spent = rows
        .iter()
        .filter_map(|r| r.get("spent_microusd").and_then(Value::as_u64))
        .fold(0u64, u64::saturating_add);
    rows.sort_by_key(|r| {
        std::cmp::Reverse(r.get("spent_microusd").and_then(Value::as_u64).unwrap_or(0))
    });
    rows.truncate(MAX_ROWS);
    let showing = if total_runs > MAX_ROWS {
        format!("top {MAX_ROWS} runs by spend (of {total_runs})")
    } else {
        format!("all {total_runs} runs")
    };
    json!({
        "total_runs": total_runs,
        "total_spent_microusd": total_spent,
        "showing": showing,
        "runs": rows,
    })
}

/// Keep the first [`MAX_ROWS`] rows of an array result (connectors return these
/// pre-ordered), wrapping with the true total. Non-arrays pass through.
fn cap_rows(field: &'static str, data: Value) -> Value {
    match data {
        Value::Array(rows) if rows.len() > MAX_ROWS => {
            let total = rows.len();
            json!({
                "total": total,
                "showing": format!("first {MAX_ROWS} of {total}"),
                field: rows.into_iter().take(MAX_ROWS).collect::<Vec<_>>(),
            })
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genaryx_connectors::{Alert, RunAgg, Summary};

    // These name the exact shaping `money_summary`, `list_runs` and `alerts`
    // hand back to the model, run against the same `to_result` + `dollarize`
    // composition their `run` methods use - no live Cloud needed to prove
    // the shape. Each failed to compile against the unfixed tree
    // (`dollarize` did not exist yet).

    #[test]
    fn money_summary_dollarizes_its_spend_field() {
        let summary = Summary {
            runs: 16,
            calls: 91,
            spent_microusd: 167_964,
        };
        let shaped = dollarize(to_result("money_summary", summary).unwrap());
        assert_eq!(shaped["spent_usd"], json!(0.167_964));
        assert!(shaped.get("spent_microusd").is_none());
    }

    #[test]
    fn list_runs_dollarizes_every_row_and_the_wrapping_total() {
        let runs = vec![
            RunAgg {
                run_id: "genaryx-copilot".to_string(),
                spent_microusd: 63_640,
                ..Default::default()
            },
            RunAgg {
                run_id: "other-run".to_string(),
                spent_microusd: 9_000,
                ..Default::default()
            },
        ];
        let shaped = dollarize(top_runs_by_spend(to_result("list_runs", runs).unwrap()));
        assert_eq!(shaped["total_spent_usd"], json!(0.07264));
        assert_eq!(shaped["runs"][0]["spent_usd"], json!(0.063_64));
        assert!(shaped["runs"][0].get("spent_microusd").is_none());
    }

    #[test]
    fn alerts_dollarizes_both_the_spent_and_the_ambiguously_named_budget_field() {
        // The exact 2026-09-27 evidence-log numbers: a run whose real budget
        // was five cents and real spend about six and a third cents.
        let alerts = vec![Alert {
            run_id: "genaryx-copilot".to_string(),
            spent_microusd: 63_640,
            budget_micros: 50_000,
            fraction: 1.2728,
            killed: false,
        }];
        let shaped = dollarize(to_result("alerts", alerts).unwrap());
        assert_eq!(shaped[0]["spent_usd"], json!(0.063_64));
        assert_eq!(shaped[0]["budget_usd"], json!(0.05));
        assert!(shaped[0].get("spent_microusd").is_none());
        assert!(shaped[0].get("budget_micros").is_none());
    }
}
