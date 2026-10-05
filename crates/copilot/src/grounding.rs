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
//!   "no budget") naming a run a tool returned a budget for.
//!
//! What it deliberately does not check, to stay free of false alarms: a
//! positive claim in a clause naming several runs (it cannot tell which run
//! the budget word is about), a run id no tool returned, budget AMOUNTS, and
//! unit budgets. Those are the model's and the prompt's.

use std::collections::BTreeSet;

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
                }
                map.values().for_each(|v| self.walk(v));
            }
            Value::Array(items) => items.iter().for_each(|v| self.walk(v)),
            _ => {}
        }
    }
}

/// Check a draft answer. Each returned string names one unsupported
/// statement, in words the model (and, if it persists, the operator) can act
/// on. Empty means nothing was found.
pub(crate) fn check(text: &str, facts: &BudgetFacts, budgets_available: bool) -> Vec<String> {
    let mut findings = Vec::new();
    let mut seen = BTreeSet::new();
    let mut unread_flagged = false;
    for clause in clauses(text) {
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
    let list: String = findings
        .iter()
        .map(|f| format!("\n- Your draft {f}."))
        .collect();
    format!(
        "Automatic check of your draft against this answer's tool results (this message is from \
         the console, not the operator):{list}\n\nRevise the answer so every budget statement \
         comes from a tool result. A run has a budget only if `budgets` (or `alerts`) returned \
         one for that run, and a budget returned for one run never belongs to another. If \
         budgets cannot be read, say so. Reply with the full revised answer."
    )
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
        if !is_budget_word(w) {
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
        let f = facts(&["p1-brume", "mig-flint"], &["p1-brume", "mig-flint"], true);
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
        let f = facts(&["p1-brume"], &["p1-brume"], false);
        assert!(check("p1-brume is over its budget", &f, false).is_empty());
        assert_eq!(check("p1-brume is over its budget", &f, true).len(), 1);
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
