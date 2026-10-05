//! `budgets`: which runs and units have a budget on the control plane, and
//! where each stands against it.
//!
//! The defect this exists for, measured 2026-10-05 on the forge lab: with no
//! budgets tool, Felyx read budgets off `alerts`, which lists a run only once
//! it is near or over its limit. It attributed p1-brume's $0.001 cap to
//! p1-beryl2, which has no budget at all, and called mig-flint (budget 4500
//! uUSD, spent 3210) "no budget" because nothing had alerted on it.
//!
//! So this reads the source of truth, `GET /v1/budgets`, and joins it with
//! each run's spend from `GET /v1/runs`; unit budgets come from
//! `GET /v1/unit-budgets` joined with `GET /v1/units`' month-to-date spend.
//! Every read is a GET the operator could already run; nothing here writes.

use std::collections::HashMap;

use async_trait::async_trait;
use genaryx_connectors::{RunAgg, UnitAgg};
use serde_json::{Value, json};

use super::{Clients, Tool, ToolError, dollarize};

/// The most budget rows handed to the model in one call, as `cloud::MAX_ROWS`.
const MAX_ROWS: usize = 30;

/// The most no-budget run ids listed; the true total is always given beside them.
const MAX_UNBUDGETED: usize = 100;

/// What the result covers, said in the result itself so the model reads it
/// with the rows. The first sentence is what lets "absent" mean "none".
const SCOPE: &str = "Budgets set on the control plane (TokenFuse Cloud). A run not listed here has \
no budget on the control plane; a unit not listed under unit_budgets has no control-plane unit \
budget. Not visible to this tool: a ceiling a gateway applies on its own (a per-run default \
budget, or a unit cap from the gateway's identity map). spent_usd is the run's lifetime spend; \
null means the Cloud holds no spend record for that run, not zero.";

pub(super) struct Budgets;

#[async_trait]
impl Tool for Budgets {
    fn name(&self) -> &'static str {
        "budgets"
    }
    fn description(&self) -> &'static str {
        "Every run budget and unit budget set on the control plane, each with its spend, the \
         fraction used and whether it is at or over the budget. The ONLY tool that says which \
         runs have a budget: a run absent here has none on the control plane. Use it for any \
         question about budgets, caps or limits, including runs well under their budget, which \
         `alerts` does not list."
    }
    async fn run(&self, clients: &Clients, _args: &Value) -> Result<Value, ToolError> {
        let cloud = clients
            .cloud
            .as_ref()
            .ok_or(ToolError::Unavailable("budgets"))?;
        let err = |e: genaryx_connectors::ConnectorError| ToolError::Connector {
            tool: "budgets",
            detail: e.to_string(),
        };
        let budgets = cloud.budgets().await.map_err(err)?;
        let runs = cloud.runs().await.map_err(err)?;
        let unit_budgets = cloud.unit_budgets().await.map_err(err)?;
        let units = match unit_budgets {
            Some(ref b) if !b.is_empty() => cloud.units().await.map_err(err)?,
            _ => None,
        };
        Ok(dollarize(shape(
            &budgets,
            &runs,
            unit_budgets.as_ref(),
            units.as_deref(),
        )))
    }
}

/// The joined result, still in micro-USD (the caller dollarizes it).
fn shape(
    budgets: &HashMap<String, i64>,
    runs: &[RunAgg],
    unit_budgets: Option<&HashMap<String, i64>>,
    units: Option<&[UnitAgg]>,
) -> Value {
    let by_run: HashMap<&str, &RunAgg> = runs.iter().map(|r| (r.run_id.as_str(), r)).collect();
    let mut rows: Vec<Value> = budgets
        .iter()
        .map(|(run_id, &budget)| {
            let run = by_run.get(run_id.as_str());
            let spent = run.map(|r| r.spent_microusd);
            let mut row = standing(budget, spent);
            row["run_id"] = json!(run_id);
            row["agent_id"] = json!(run.map(|r| r.agent_id.clone()));
            row["killed"] = json!(run.map(|r| r.killed));
            row
        })
        .collect();
    // Over or at first, then by fraction used, then by id so the order is stable.
    rows.sort_by(|a, b| {
        let key = |v: &Value| {
            (
                v["at_or_over"].as_bool().unwrap_or(false),
                v["fraction"].as_f64().unwrap_or(-1.0),
            )
        };
        let (ao, af) = key(a);
        let (bo, bf) = key(b);
        bo.cmp(&ao)
            .then(bf.total_cmp(&af))
            .then_with(|| a["run_id"].as_str().cmp(&b["run_id"].as_str()))
    });
    let total = rows.len();
    let complete = total <= MAX_ROWS;
    rows.truncate(MAX_ROWS);

    // The runs the Cloud holds spend for and no budget, highest spend first,
    // so "has none" is a row the model reads rather than an absence it infers.
    let mut unbudgeted: Vec<&RunAgg> = runs
        .iter()
        .filter(|r| !budgets.contains_key(&r.run_id))
        .collect();
    unbudgeted.sort_by(|a, b| {
        b.spent_microusd
            .cmp(&a.spent_microusd)
            .then_with(|| a.run_id.cmp(&b.run_id))
    });
    let unbudgeted_total = unbudgeted.len();
    let unbudgeted_rows: Vec<Value> = unbudgeted
        .into_iter()
        .take(MAX_UNBUDGETED)
        .map(|r| json!({"run_id": r.run_id, "agent_id": r.agent_id}))
        .collect();

    let mut out = json!({
        "scope": SCOPE,
        "runs_without_budget_total": unbudgeted_total,
        "runs_without_budget": unbudgeted_rows,
        "run_budgets_total": total,
        "complete": complete,
        "showing": if complete {
            format!("all {total} run budgets")
        } else {
            format!("the {MAX_ROWS} closest to or over their budget, of {total}; a run absent from \
                     this page may still have a budget")
        },
        "run_budgets": rows,
    });

    match unit_budgets {
        None => {
            out["unit_budgets"] = Value::Null;
            out["unit_budgets_unavailable"] = json!(
                "this Cloud does not answer GET /v1/unit-budgets, so unit budgets cannot be read \
                 here; that is not the same as having none"
            );
        }
        Some(map) => {
            let by_unit: HashMap<&str, &UnitAgg> = units
                .unwrap_or_default()
                .iter()
                .map(|u| (u.unit.as_str(), u))
                .collect();
            let mut unit_rows: Vec<Value> = map
                .iter()
                .map(|(unit, &budget)| {
                    let agg = by_unit.get(unit.as_str());
                    let mut row = standing(budget, agg.map(|u| u.month_spent_microusd));
                    // The month spend is under `month_spent_microusd`, so
                    // the generic `spent_microusd` slot is moved there.
                    let spent = row["spent_microusd"].take();
                    row.as_object_mut().unwrap().remove("spent_microusd");
                    row["month_spent_microusd"] = spent;
                    row["unit"] = json!(unit);
                    row["month"] = json!(agg.map(|u| u.month.clone()));
                    row
                })
                .collect();
            unit_rows.sort_by(|a, b| a["unit"].as_str().cmp(&b["unit"].as_str()));
            out["unit_budgets"] = Value::Array(unit_rows);
        }
    }
    out
}

/// Budget, spend, fraction and at-or-over for one budget. Spend `None` is an
/// unknown spend and leaves the derived fields `null`, never zero. A budget
/// of zero or less has no meaningful fraction, so those stay `null` too.
fn standing(budget: i64, spent: Option<i64>) -> Value {
    let (fraction, at_or_over) = match spent {
        Some(s) if budget > 0 => (json!(s as f64 / budget as f64), json!(s >= budget)),
        _ => (Value::Null, Value::Null),
    };
    json!({
        "budget_micros": budget,
        "spent_microusd": spent,
        "fraction": fraction,
        "at_or_over": at_or_over,
    })
}

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![Box::new(Budgets)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: &str, spent: i64) -> RunAgg {
        RunAgg {
            run_id: id.to_string(),
            spent_microusd: spent,
            ..Default::default()
        }
    }

    #[test]
    fn at_or_over_is_exact_integer_comparison_at_the_boundary() {
        assert_eq!(standing(1278, Some(1278))["at_or_over"], json!(true));
        assert_eq!(standing(1278, Some(1277))["at_or_over"], json!(false));
        assert_eq!(standing(0, Some(5))["at_or_over"], json!(null));
        assert_eq!(standing(1000, None)["fraction"], json!(null));
    }

    #[test]
    fn more_budgets_than_a_page_say_the_page_is_not_complete() {
        let budgets: HashMap<String, i64> =
            (0..MAX_ROWS + 5).map(|i| (format!("r{i}"), 1000)).collect();
        let runs: Vec<RunAgg> = (0..MAX_ROWS + 5)
            .map(|i| run(&format!("r{i}"), i as i64))
            .collect();
        let out = shape(&budgets, &runs, Some(&HashMap::new()), None);
        assert_eq!(out["complete"], json!(false));
        assert_eq!(out["run_budgets_total"], json!(MAX_ROWS + 5));
        assert_eq!(out["run_budgets"].as_array().unwrap().len(), MAX_ROWS);
        assert!(
            out["showing"]
                .as_str()
                .unwrap()
                .contains("may still have a budget")
        );
    }

    #[test]
    fn over_budget_runs_sort_first() {
        let budgets: HashMap<String, i64> =
            [("under".to_string(), 4500), ("over".to_string(), 1000)].into();
        let runs = vec![run("under", 3210), run("over", 2008)];
        let out = shape(&budgets, &runs, Some(&HashMap::new()), None);
        assert_eq!(out["run_budgets"][0]["run_id"], json!("over"));
        assert_eq!(out["run_budgets"][1]["run_id"], json!("under"));
    }
}
