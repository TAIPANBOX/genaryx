//! The budget check on Felyx's draft answer: a statement about a run's budget
//! must have a tool result behind it.
//!
//! Measured 2026-10-05 on the forge lab: Felyx wrote "p1-beryl2 exceeded
//! budget alert (spent $0.004019 vs $0.001 cap)" when no tool had returned a
//! budget for p1-beryl2 (the cap was p1-brume's), and "the other router runs
//! (p1-beryl2, mig-flint, ...) do not have budgets set" when the Cloud held a
//! budget for mig-flint. A prompt rule asks the model not to do this; this
//! module checks that it did not, deterministically, against the tool results
//! of the same answer.
//!
//! What it checks, per clause of the draft (a line, or a sentence, split
//! again at "but"/"while"/"whereas"/";"):
//! - a clause that names a known run and a budget word, or a budget word and
//!   a dollar amount, while the `budgets` tool was available and not called;
//! - a clause that names exactly ONE known run and states it has a budget
//!   ("over budget", "$0.001 cap"), when no tool returned a budget for it;
//! - a clause that says its runs have NO budget ("do not have budgets set",
//!   "no budget") naming a run a tool returned a budget for;
//! - a clause that says a run, or an AGENT, is over a budget ("flint has
//!   budget overages") when no tool returned that run, or any run of that
//!   agent, at or over its budget. Added after the second forge run the same
//!   day: every run was right and the summary line still said "Flint and
//!   brume have budget overages", and "flint" is an agent name, not a run id.
//!
//! What it deliberately does not check, to stay free of false alarms: a
//! positive claim in a clause naming several runs (it cannot tell which run
//! the budget word is about), a run id no tool returned, budget AMOUNTS, and
//! unit budgets. Those are the model's and the prompt's.
//!
//! The same per-clause pass checks one claim about MODELS (2026-10-14): a
//! clause that names exactly one run and exactly one model a tool returned,
//! as the model that run used, when the tools did not return that model as
//! the run's only one. `list_runs` returns `last_model`, the model of the
//! run's last call, and `models` only when the Cloud reports every model a
//! run used. Measured on the forge lab the same day: asked which models an
//! Opus-then-Haiku run used, Felyx answered that it "used claude-haiku-5-5",
//! and this check passed it, because the tool result carried only the last
//! model. A clause that says "last model" (or "latest", "most recent", "last
//! call") is a claim the tools do support and is left alone.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

/// Budget facts the tools returned during one answer.
#[derive(Debug, Default)]
pub(crate) struct BudgetFacts {
    /// Every run id any read tool returned.
    known_runs: BTreeSet<String>,
    /// Run ids a tool returned WITH a budget (`budgets` rows, `alerts` rows).
    budgeted: BTreeSet<String>,
    /// Whether `budgets` ran (successfully) this answer.
    budgets_read: bool,
    /// Runs a tool returned at or over their budget (`budgets`'
    /// `at_or_over`, or an alert's `fraction` of 1 or more).
    runs_over: BTreeSet<String>,
    /// Run id to agent id, from any row that carries both.
    run_agent: BTreeMap<String, String>,
    /// Every agent id any read tool returned.
    agents: BTreeSet<String>,
    /// What the tools returned about each run's models.
    run_models: BTreeMap<String, RunModels>,
    /// Every model id any read tool returned.
    known_models: BTreeSet<String>,
}

/// One run's models as the tools returned them.
#[derive(Debug, Default)]
struct RunModels {
    /// Every model the run used, when a tool returned the list.
    models: Option<Vec<String>>,
    /// Model changes, when a tool returned the count.
    switches: Option<u64>,
}

impl BudgetFacts {
    /// Fold one read tool's result in. Propose tools must not be recorded: a
    /// proposed budget is not a budget.
    pub(crate) fn record(&mut self, tool: &str, ok: bool, value: &Value) {
        if !ok {
            return;
        }
        if tool == "budgets" {
            self.budgets_read = true;
        }
        self.walk(value);
    }

    fn walk(&mut self, value: &Value) {
        match value {
            Value::Object(map) => {
                if let Some(id) = map.get("run_id").and_then(Value::as_str)
                    && !id.is_empty()
                {
                    self.known_runs.insert(id.to_string());
                    if map.get("budget_usd").is_some_and(|b| !b.is_null()) {
                        self.budgeted.insert(id.to_string());
                    }
                    let over = map.get("at_or_over").and_then(Value::as_bool) == Some(true)
                        || map
                            .get("fraction")
                            .and_then(Value::as_f64)
                            .is_some_and(|f| f >= 1.0);
                    if over {
                        self.runs_over.insert(id.to_string());
                    }
                    if let Some(agent) = map.get("agent_id").and_then(Value::as_str)
                        && !agent.is_empty()
                    {
                        self.run_agent.insert(id.to_string(), agent.to_string());
                    }
                    self.record_models(id, map);
                }
                if let Some(agent) = map.get("agent_id").and_then(Value::as_str)
                    && !agent.is_empty()
                {
                    self.agents.insert(agent.to_string());
                }
                map.values().for_each(|v| self.walk(v));
            }
            Value::Array(items) => items.iter().for_each(|v| self.walk(v)),
            _ => {}
        }
    }
}

impl BudgetFacts {
    /// A run row's model facts: `last_model` (or a bare `model`), `models`
    /// and `model_switches`. A row with none of them says nothing about the
    /// run's models and is not recorded.
    fn record_models(&mut self, id: &str, map: &serde_json::Map<String, Value>) {
        let last = map
            .get("last_model")
            .or_else(|| map.get("model"))
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty());
        let list: Option<Vec<String>> = map.get("models").and_then(Value::as_array).map(|a| {
            a.iter()
                .filter_map(|m| m.get("model").and_then(Value::as_str))
                .filter(|m| !m.is_empty())
                .map(str::to_string)
                .collect()
        });
        let switches = map.get("model_switches").and_then(Value::as_u64);
        if last.is_none() && list.is_none() && switches.is_none() {
            return;
        }
        if let Some(m) = last {
            self.known_models.insert(m.to_string());
        }
        for m in list.iter().flatten() {
            self.known_models.insert(m.clone());
        }
        let entry = self.run_models.entry(id.to_string()).or_default();
        if list.is_some() {
            entry.models = list;
        }
        if switches.is_some() {
            entry.switches = switches;
        }
    }
}

/// Phrases that make a model claim about a run's LAST call, which
/// `last_model` supports.
const LAST_MODEL_PHRASES: [&str; 7] = [
    "last model",
    "latest model",
    "most recent model",
    "last call",
    "latest call",
    "most recent call",
    "last_model",
];

/// A clause naming one run and one model, as the model that run used, when
/// the tools did not return that model as the run's only one.
fn model_claim(clause: &str, facts: &BudgetFacts) -> Option<(String, String)> {
    let runs = runs_named(clause, &facts.known_runs);
    let [run] = runs.as_slice() else {
        return None;
    };
    let fact = facts.run_models.get(*run)?;
    let lower = clause.to_lowercase();
    if LAST_MODEL_PHRASES.iter().any(|p| lower.contains(p)) {
        return None;
    }
    let models = models_named(clause, &facts.known_models);
    let [model] = models.as_slice() else {
        return None;
    };
    let key = format!("model:{run}");
    let finding = match &fact.models {
        Some(list) if list.len() == 1 && list[0] == **model && fact.switches.unwrap_or(0) == 0 => {
            return None;
        }
        Some(list) if list.len() > 1 => format!(
            "names `{model}` as the model run `{run}` used, but the tools returned {} models \
             for `{run}`: {}",
            list.len(),
            list.join(", ")
        ),
        Some(list) => format!(
            "names `{model}` as the model run `{run}` used, but the tools returned {} for `{run}`",
            if list.is_empty() {
                "no model".to_string()
            } else {
                format!("`{}`", list.join(", "))
            }
        ),
        None => format!(
            "names `{model}` as the model run `{run}` used, but the tools return only the model \
             of `{run}`'s last call, so which models `{run}` used is not known"
        ),
    };
    Some((key, finding))
}

/// Known model ids named in the clause as whole ids: "claude-haiku-5-5" is
/// not found inside "claude-haiku-5-5-thinking", and a dot inside an id
/// ("gpt-4.1-mini") is part of it while a dot ending a sentence is not.
fn models_named<'a>(clause: &str, known: &'a BTreeSet<String>) -> Vec<&'a String> {
    let lower = clause.to_lowercase();
    let is_id_char = |c: char| c.is_alphanumeric() || matches!(c, '-' | '_' | '/');
    known
        .iter()
        .filter(|id| {
            let needle = id.to_lowercase();
            lower.match_indices(needle.as_str()).any(|(at, _)| {
                let before = lower[..at].chars().next_back();
                let mut rest = lower[at + needle.len()..].chars();
                let after = rest.next();
                let after_after = rest.next();
                let continues = after.is_some_and(is_id_char)
                    || (after == Some('.') && after_after.is_some_and(char::is_alphanumeric));
                !before.is_some_and(|c| is_id_char(c) || c == '.') && !continues
            })
        })
        .collect()
}

/// Check a draft answer. Each returned string names one unsupported
/// statement, in words the model (and, if it persists, the operator) can act
/// on. Empty means nothing was found.
pub(crate) fn check(text: &str, facts: &BudgetFacts, budgets_available: bool) -> Vec<String> {
    let mut findings = Vec::new();
    let mut seen = BTreeSet::new();
    let mut unread_flagged = false;
    for clause in clauses(text) {
        if let Some((key, finding)) = model_claim(&clause, facts)
            && seen.insert(key)
        {
            findings.push(finding);
        }
        let words = words(&clause);
        if !words.iter().any(|w| is_budget_word(w)) {
            continue;
        }
        let runs = runs_named(&clause, &facts.known_runs);
        if budgets_available
            && !facts.budgets_read
            && !unread_flagged
            && (!runs.is_empty() || clause.contains('$'))
        {
            unread_flagged = true;
            findings.push(
                "states budget facts without calling `budgets`, the only tool that says which \
                 runs have a budget"
                    .to_string(),
            );
        }
        if words.iter().any(|w| is_over_word(w)) && !words.iter().any(|w| is_negator(w)) {
            if let [run] = runs.as_slice()
                && facts.budgeted.contains(*run)
                && !facts.runs_over.contains(*run)
                && seen.insert(format!("over:{run}"))
            {
                findings.push(format!(
                    "says run `{run}` is over its budget, but the tools returned `{run}` under it"
                ));
            }
            for (label, ids) in agents_named(&clause, &facts.agents) {
                let any_over = facts
                    .runs_over
                    .iter()
                    .any(|r| facts.run_agent.get(r).is_some_and(|a| ids.contains(a)));
                if !any_over && seen.insert(format!("agent-over:{label}")) {
                    findings.push(format!(
                        "says agent `{label}` is over a budget, but no tool returned a run of \
                         `{label}` at or over its budget"
                    ));
                }
            }
        }
        if says_no_budget(&words) {
            for run in &runs {
                if facts.budgeted.contains(*run) && seen.insert(format!("none:{run}")) {
                    findings.push(format!(
                        "says run `{run}` has no budget, but a tool returned a budget for `{run}`"
                    ));
                }
            }
        } else if let [run] = runs.as_slice()
            && !facts.budgeted.contains(*run)
            && seen.insert(format!("has:{run}"))
        {
            findings.push(format!(
                "states a budget for run `{run}`, but no tool returned a budget for `{run}`"
            ));
        }
    }
    findings
}

/// The revision request fed back to the model once, as a user turn.
pub(crate) fn revision_request(findings: &[String]) -> String {
    let models = model_rule(findings);
    let list: String = findings
        .iter()
        .map(|f| format!("\n- Your draft {f}."))
        .collect();
    format!(
        "Automatic check of your draft against this answer's tool results (this message is from \
         the console, not the operator):{list}\n\nRevise the answer so every budget statement \
         comes from a tool result. A run has a budget only if `budgets` (or `alerts`) returned \
         one for that run, and a budget returned for one run never belongs to another. If \
         budgets cannot be read, say so.{models} Reply with only the corrected answer for the \
         operator, written as if it were your first answer. Do not mention this check, \
         apologise, or say that you are revising."
    )
}

/// The model sentence of the revision request, only when a model claim was
/// found.
fn model_rule(findings: &[String]) -> &'static str {
    if findings.iter().any(|f| f.contains("as the model run")) {
        " A run's `last_model` is the model of its last call only: name it as the last \
         model, and say a run used one model only when `models` lists exactly that one. When \
         `models` is null, say which models the run used is not known."
    } else {
        ""
    }
}

/// The note appended to an answer whose statements survived the revision.
pub(crate) fn residual_note(findings: &[String]) -> String {
    let list: String = findings
        .iter()
        .map(|f| format!("\n- the answer {f}"))
        .collect();
    format!(
        "\n\nCheck: these statements are not supported by any tool result in this answer:{list}"
    )
}

const BUDGET_WORDS: [&str; 6] = ["budget", "budgets", "budgeted", "cap", "caps", "capped"];
/// Words that claim a budget was reached or passed.
const OVER_WORDS: [&str; 12] = [
    "over",
    "overage",
    "overages",
    "overspent",
    "overspend",
    "overrun",
    "exceed",
    "exceeds",
    "exceeded",
    "exceeding",
    "breach",
    "breached",
];
/// A budget word followed by one of these is about trouble, not about having
/// a budget: "no budget issues" says nothing about whether one exists.
const NOT_ABSENCE: [&str; 12] = [
    "issue", "issues", "problem", "problems", "concern", "concerns", "alert", "alerts", "overage",
    "overages", "warning", "warnings",
];

fn is_over_word(w: &str) -> bool {
    OVER_WORDS.contains(&w)
}
const NEGATORS: [&str; 7] = ["no", "not", "without", "lack", "lacks", "lacking", "none"];
/// Words that may sit between a negator and the budget word it governs:
/// "do NOT have budgets", "has NO control-plane budget", "WITHOUT a budget".
const FILLER: [&str; 15] = [
    "have", "has", "had", "a", "an", "any", "set", "its", "their", "the", "control", "plane",
    "central", "run", "per",
];

fn is_budget_word(w: &str) -> bool {
    BUDGET_WORDS.contains(&w)
}

fn is_negator(w: &str) -> bool {
    NEGATORS.contains(&w) || w.ends_with("n't")
}

/// Whether the clause says its runs have no budget. A negator counts only
/// when nothing but filler sits between it and the budget word, so "has not
/// exceeded its budget" (which implies a budget) is not a "no budget" claim.
fn says_no_budget(words: &[String]) -> bool {
    if words.iter().any(|w| w == "unbudgeted") {
        return true;
    }
    words.iter().enumerate().any(|(i, w)| {
        if !is_budget_word(w)
            || words
                .get(i + 1)
                .is_some_and(|n| NOT_ABSENCE.contains(&n.as_str()))
        {
            return false;
        }
        let mut j = i;
        while j > 0 {
            j -= 1;
            let prev = words[j].as_str();
            if is_negator(prev) {
                return true;
            }
            if !FILLER.contains(&prev) {
                return false;
            }
        }
        false
    })
}

/// Lowercased words: runs of letters, digits and apostrophes, so "over-budget"
/// yields "budget" and "don't" stays one word.
fn words(clause: &str) -> Vec<String> {
    clause
        .to_lowercase()
        .replace('\u{2019}', "'")
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Known run ids named in the clause as whole ids: "p1-brume" is not found
/// inside "p1-brume-2".
fn runs_named<'a>(clause: &str, known: &'a BTreeSet<String>) -> Vec<&'a String> {
    let is_id_char = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
    known
        .iter()
        .filter(|id| {
            clause.match_indices(id.as_str()).any(|(at, _)| {
                let before = clause[..at].chars().next_back();
                let after = clause[at + id.len()..].chars().next();
                !before.is_some_and(is_id_char) && !after.is_some_and(is_id_char)
            })
        })
        .collect()
}

/// Agents named in the clause, by full id or by the last segment of their id
/// ("flint" for `agent://taipanbox.dev/routers/flint`), case-insensitive and
/// whole: "flint" is not found inside the run id "mig-flint". Returns each
/// label with every agent id it may stand for.
fn agents_named(clause: &str, agents: &BTreeSet<String>) -> Vec<(String, BTreeSet<String>)> {
    let mut by_label: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for id in agents {
        let last = id.rsplit('/').next().unwrap_or(id).to_lowercase();
        if !last.is_empty() {
            by_label.entry(last).or_default().insert(id.clone());
        }
    }
    let lower = clause.to_lowercase();
    let is_id_char = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
    by_label
        .into_iter()
        .filter(|(label, _)| {
            lower.match_indices(label.as_str()).any(|(at, _)| {
                let before = lower[..at].chars().next_back();
                let after = lower[at + label.len()..].chars().next();
                !before.is_some_and(is_id_char) && !after.is_some_and(is_id_char)
            })
        })
        .collect()
}

/// Lines, then sentences (a '.', '!' or '?' followed by whitespace or the
/// end, so "$0.001" stays whole), then ';', then the contrast words.
fn clauses(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let next_is_gap = chars.get(i + 1).is_none_or(|n| n.is_whitespace());
        if c == '\n' || c == ';' || (matches!(c, '.' | '!' | '?') && next_is_gap) {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out.into_iter()
        .flat_map(|c| split_contrast(&c))
        .filter(|c| !c.trim().is_empty())
        .collect()
}

fn split_contrast(clause: &str) -> Vec<String> {
    let lower = clause.to_lowercase();
    let mut cuts = Vec::new();
    for word in [" but ", " while ", " whereas "] {
        cuts.extend(lower.match_indices(word).map(|(at, w)| (at, w.len())));
    }
    cuts.sort_unstable();
    let mut out = Vec::new();
    let mut start = 0;
    for (at, len) in cuts {
        if at >= start && clause.is_char_boundary(at) && clause.is_char_boundary(at + len) {
            out.push(clause[start..at].to_string());
            start = at + len;
        }
    }
    out.push(clause[start..].to_string());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn facts(known: &[&str], budgeted: &[&str], read: bool) -> BudgetFacts {
        BudgetFacts {
            known_runs: known.iter().map(|s| s.to_string()).collect(),
            budgeted: budgeted.iter().map(|s| s.to_string()).collect(),
            budgets_read: read,
            ..Default::default()
        }
    }

    #[test]
    fn record_takes_budgets_from_rows_that_carry_one_and_ignores_failed_calls() {
        let mut f = BudgetFacts::default();
        f.record(
            "budgets",
            true,
            &json!({"run_budgets": [{"run_id": "a", "budget_usd": 0.001}, {"run_id": "b", "budget_usd": null}],
                    "runs_without_budget": [{"run_id": "c", "agent_id": "x"}]}),
        );
        f.record(
            "alerts",
            false,
            &json!([{"run_id": "d", "budget_usd": 1.0}]),
        );
        assert!(f.budgets_read);
        assert_eq!(f.budgeted, BTreeSet::from(["a".to_string()]));
        assert!(f.known_runs.contains("c"));
        assert!(
            !f.known_runs.contains("d"),
            "a failed call returned nothing"
        );
    }

    #[test]
    fn a_run_id_is_matched_whole_never_inside_a_longer_one() {
        let f = facts(&["p1-brume"], &[], true);
        assert!(check("p1-brume-2 is over its budget", &f, true).is_empty());
        assert_eq!(check("p1-brume is over its budget", &f, true).len(), 1);
    }

    #[test]
    fn has_not_exceeded_its_budget_is_not_a_no_budget_claim() {
        let f = facts(&["p1-brume"], &["p1-brume"], true);
        assert!(check("p1-brume has not exceeded its budget.", &f, true).is_empty());
        let none = check("p1-brume does not have a budget.", &f, true);
        assert_eq!(none.len(), 1, "{none:?}");
    }

    #[test]
    fn a_dollar_amount_with_a_decimal_point_does_not_split_the_sentence() {
        let f = facts(&["p1-beryl2"], &[], true);
        let found = check("p1-beryl2 spent $0.004019 vs $0.001 cap.", &f, true);
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn a_contrast_word_separates_two_claims() {
        let mut f = facts(&["p1-brume", "mig-flint"], &["p1-brume", "mig-flint"], true);
        f.runs_over.insert("p1-brume".to_string());
        assert!(
            check(
                "p1-brume is over its budget, but mig-flint is under its budget",
                &f,
                true
            )
            .is_empty()
        );
        let found = check(
            "p1-brume is over budget while mig-flint has no budget",
            &f,
            true,
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("mig-flint"));
    }

    #[test]
    fn no_budgets_tool_in_this_install_means_no_demand_to_call_it() {
        let mut f = facts(&["p1-brume"], &["p1-brume"], false);
        f.runs_over.insert("p1-brume".to_string());
        assert!(check("p1-brume is over its budget", &f, false).is_empty());
        assert_eq!(check("p1-brume is over its budget", &f, true).len(), 1);
    }

    #[test]
    fn no_budget_issues_is_not_a_no_budget_claim() {
        let f = facts(&["mig-flint"], &["mig-flint"], true);
        assert!(check("mig-flint: no budget issues.", &f, true).is_empty());
        assert_eq!(check("mig-flint: no budget set.", &f, true).len(), 1);
    }

    #[test]
    fn an_agent_is_over_only_if_one_of_its_runs_is() {
        let mut f = BudgetFacts::default();
        f.record(
            "budgets",
            true,
            &json!({"run_budgets": [
                {"run_id": "p1-brume", "agent_id": "agent://x/routers/brume", "budget_usd": 0.001, "at_or_over": true},
                {"run_id": "mig-flint", "agent_id": "agent://x/routers/flint", "budget_usd": 0.0045, "at_or_over": false}
            ]}),
        );
        let found = check("Flint and Brume have budget overages.", &f, true);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("`flint`"));
        assert!(
            check(
                "brume is over budget, but flint is not over its budget.",
                &f,
                true
            )
            .is_empty()
        );
        assert!(check("mig-flint is a flint run under its budget.", &f, true).is_empty());
    }

    #[test]
    fn an_alert_at_a_fraction_of_one_or_more_counts_as_over() {
        let mut f = BudgetFacts::default();
        f.record(
            "list_runs",
            true,
            &json!({"runs": [{"run_id": "r", "agent_id": "agent://x/a/alpha"}]}),
        );
        f.record(
            "alerts",
            true,
            &json!([{"run_id": "r", "budget_usd": 0.01, "fraction": 1.2}]),
        );
        f.record("budgets", true, &json!({}));
        assert!(check("alpha exceeded its budget.", &f, true).is_empty());
        assert!(check("r is over its budget.", &f, true).is_empty());
    }

    /// The forge lab's B11 run of 2026-10-14: Opus, then Haiku. Today's Cloud
    /// returns only the last call's model.
    fn switched_run(models: Option<serde_json::Value>) -> BudgetFacts {
        let mut row = json!({"run_id": "w2-forge-switch-a3-133712",
                             "agent_id": "agent://lab/w2/switcher",
                             "last_model": "claude-haiku-5-5", "spent_usd": 0.000566});
        if let Some(m) = models {
            row["models"] = m;
            row["model_switches"] = json!(1);
        }
        let mut f = BudgetFacts::default();
        f.record("list_runs", true, &json!({"runs": [row]}));
        f
    }

    #[test]
    fn a_single_model_claim_for_a_run_whose_models_are_unknown_is_flagged() {
        // The measured answer, word for word in substance.
        let f = switched_run(None);
        let found = check(
            "Run w2-forge-switch-a3-133712 used claude-haiku-5-5 and cost $0.000158.",
            &f,
            false,
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("only the model of"), "{found:?}");
        assert!(revision_request(&found).contains("name it as the last model"));
    }

    #[test]
    fn a_single_model_claim_for_a_run_that_used_several_is_flagged() {
        let f = switched_run(Some(json!([
            {"model": "claude-opus-5-5", "calls": 1, "spent_usd": 0.000408},
            {"model": "claude-haiku-5-5", "calls": 1, "spent_usd": 0.000158}
        ])));
        let found = check(
            "w2-forge-switch-a3-133712 ran on claude-haiku-5-5",
            &f,
            false,
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].contains("2 models") && found[0].contains("claude-opus-5-5"),
            "{found:?}"
        );
        // Both named, or the last one named as the last: supported.
        assert!(
            check(
                "w2-forge-switch-a3-133712 used claude-opus-5-5 then claude-haiku-5-5",
                &f,
                false
            )
            .is_empty()
        );
        assert!(
            check(
                "w2-forge-switch-a3-133712's last model was claude-haiku-5-5",
                &f,
                false
            )
            .is_empty()
        );
    }

    #[test]
    fn a_run_the_tools_say_used_one_model_may_be_said_to_have_used_it() {
        let mut f = switched_run(Some(json!([{"model": "claude-haiku-5-5", "calls": 2}])));
        f.run_models
            .get_mut("w2-forge-switch-a3-133712")
            .expect("recorded")
            .switches = Some(0);
        assert!(check("w2-forge-switch-a3-133712 used claude-haiku-5-5", &f, false).is_empty());
        // ...but not a model the tools did not return for it.
        f.known_models.insert("claude-opus-5-5".to_string());
        let found = check("w2-forge-switch-a3-133712 used claude-opus-5-5", &f, false);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(!revision_request(&["budget only".to_string()]).contains("last_model"));
    }

    #[test]
    fn a_model_id_is_matched_whole_with_its_inner_dots() {
        let mut f = BudgetFacts::default();
        f.record(
            "list_runs",
            true,
            &json!({"runs": [{"run_id": "w2-b1-openai", "last_model": "openai/gpt-4.1-mini"}]}),
        );
        assert_eq!(
            check("w2-b1-openai used openai/gpt-4.1-mini.", &f, false).len(),
            1
        );
        assert!(check("w2-b1-openai used openai/gpt-4.1-mini-2024", &f, false).is_empty());
        assert!(check("w2-b1-openai used xopenai/gpt-4.1-mini", &f, false).is_empty());
        // A clause naming no run, or two, is not checked.
        assert!(check("openai/gpt-4.1-mini is cheap", &f, false).is_empty());
        // A row with no model facts at all records nothing.
        f.record(
            "alerts",
            true,
            &json!([{"run_id": "w2-b1-openai", "fraction": 0.2}]),
        );
        assert_eq!(f.run_models.len(), 1);
    }

    #[test]
    fn hostile_model_text_never_panics_and_never_flags_a_last_model_clause() {
        let f = switched_run(None);
        let alphabet: Vec<char> =
            "w2-forge-switch-a3-133712 claude-haiku-5-5 used last model İ. \n"
                .chars()
                .collect();
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        for _ in 0..200 {
            let mut s = String::new();
            for _ in 0..(seed % 160) {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                s.push(alphabet[(seed % alphabet.len() as u64) as usize]);
            }
            let _ = check(&s, &f, false);
            let tagged = format!("w2-forge-switch-a3-133712 last model claude-haiku-5-5 {s}");
            let tagged_findings = check(&tagged, &f, false);
            assert!(
                clauses(&tagged).len() > 1
                    || tagged_findings
                        .iter()
                        .all(|x| !x.contains("as the model run")),
                "{tagged:?}: {tagged_findings:?}"
            );
            seed = seed.wrapping_add(1);
        }
    }

    #[test]
    fn hostile_text_never_panics() {
        let f = facts(&["a", "é-run"], &["a"], false);
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let alphabet: Vec<char> = "ab é-run $0.1 .;!?\n budget no not n't cap but while \u{2019}"
            .chars()
            .collect();
        for _ in 0..200 {
            let mut s = String::new();
            for _ in 0..(seed % 120) {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                s.push(alphabet[(seed % alphabet.len() as u64) as usize]);
            }
            let _ = check(&s, &f, true);
            seed = seed.wrapping_add(1);
        }
    }
}
