//! Money-plane read tools, backed by `CloudClient` (all `async`, bearer-auth).

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{Clients, Tool, ToolError, to_result};

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
                to_result($name, data)
            }
        }
    };
}

read_tool!(
    MoneySummary,
    "money_summary",
    "Org-wide totals: number of runs, calls, and total spend (microdollars). Use for headline spend questions.",
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
        "Top per-run spenders for the org, highest spend first (run id, model, agent, spent_microusd, \
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
        Ok(top_runs_by_spend(to_result("list_runs", runs)?))
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
