//! Money-plane read tools, backed by `CloudClient` (all `async`, bearer-auth).

use async_trait::async_trait;
use serde_json::Value;

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
    ListRuns,
    "list_runs",
    "Per-run spend rollup for the org (run id, model, agent, spent, calls, steps, whether killed). Use to find the top spenders or a specific run.",
    runs
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
    Incidents,
    "incidents",
    "Open incidents for the org, newest first (id, kind, severity, run/agent, occurrences, acknowledged).",
    incidents
);
read_tool!(
    Alerts,
    "alerts",
    "Runs at or above their budget alert threshold (run id, spent, budget, fraction of budget, whether killed). Use to find near-cap and over-cap runs.",
    alerts
);
