//! The bench: how well a named cause, the ids an answer cites and the remedy that
//! follows from the cause hold against what is known to be true.
//!
//! # WHY THIS EXISTS
//!
//! Slices 1 to 6 of the incident-to-proven-fix plan are checked piece by piece:
//! the case assembles, the model's draft is checked against it, a proof is made
//! and sealed. None of that says whether the loop, taken whole, reaches the right
//! answer on an incident whose cause is known. The bench is that question, modelled
//! on `costcrew-bench`: a registry of incidents with a known cause, a packet that
//! hides the cause, and three questions asked of what the analyst says.
//!
//! # THE THREE QUESTIONS
//!
//! 1. **Was the named cause right?** The answer is read for the cause it names
//!    ([`named_cause`]) and the cause it names must be the true one and no other: a
//!    text that names two causes names none. Judged on the cause that is NAMED, not
//!    on whether the true cause's words appear somewhere in the body, which any long
//!    answer would satisfy.
//! 2. **Does every cited id exist?** An answer passes only if the reference check
//!    left nothing unsupported in it and did not withhold the chain whole.
//! 3. **Does the remedy stop the replay?** The remedy that follows from the cause
//!    the answer named ([`Cause::remedy`]) is replayed against the incident's
//!    recorded history, and it passes if the proof says it would have stopped the
//!    incident. A cause that calls for no remedy console-side is neither a pass nor
//!    a miss, and is counted as neither.
//!
//! The cause is hidden: [`gives_away`] is how a packet is checked for the words
//! that would hand the analyst its answer.

use serde::Serialize;

/// What starts the text of an answer whose chain was withheld whole.
pub const WITHHELD_PREFIX: &str = "No root-cause chain is given";

/// The causes the registry knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    /// A run calling the same tool over and over until its budget is gone.
    RetryLoop,
    /// A call for a tool the agent's policy denies.
    DeniedTool,
    /// A call made with a key the gateway does not hold.
    UnknownKey,
}

/// Every cause, in the order a report lists them.
pub const CAUSES: [Cause; 3] = [Cause::RetryLoop, Cause::DeniedTool, Cause::UnknownKey];

/// The kind of remedy a cause calls for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemedyFor {
    /// A budget or a step limit, proved by `tokenfuse backtest`.
    Budget,
    /// A policy candidate, proved by `wardryx replay`.
    Policy,
}

impl Cause {
    pub fn label(self) -> &'static str {
        match self {
            Cause::RetryLoop => "retry_loop",
            Cause::DeniedTool => "denied_tool",
            Cause::UnknownKey => "unknown_key",
        }
    }

    /// Groups of lowercase words: the cause is named in a sentence that holds every
    /// word of some one group.
    fn groups(self) -> &'static [&'static [&'static str]] {
        match self {
            Cause::RetryLoop => &[&["retry", "loop"], &["repeat", "same tool"], &["loop"]],
            Cause::DeniedTool => &[&["denied", "tool"], &["denies", "tool"], &["policy", "denied"]],
            Cause::UnknownKey => &[
                &["unknown", "key"],
                &["key", "not held"],
                &["does not hold", "key"],
            ],
        }
    }

    /// Words that would give the cause away if a packet held them.
    fn giveaways(self) -> &'static [&'static str] {
        match self {
            Cause::RetryLoop => &["retry_loop", "retry", "loop"],
            Cause::DeniedTool => &["denied_tool", "denied tool"],
            Cause::UnknownKey => &["unknown_key", "unknown key"],
        }
    }

    /// The remedy that follows from the cause, if the console has a proof for one.
    pub fn remedy(self) -> Option<RemedyFor> {
        match self {
            Cause::RetryLoop => Some(RemedyFor::Budget),
            Cause::DeniedTool => Some(RemedyFor::Policy),
            Cause::UnknownKey => None,
        }
    }
}

/// What an answer names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "named", content = "cause", rename_all = "snake_case")]
pub enum Named {
    Cause(Cause),
    /// No cause is named.
    Nothing,
    /// More than one cause is named, which is none.
    Several,
}

/// The cause an answer names: the one cause with a sentence that names it, and no
/// other.
pub fn named_cause(text: &str) -> Named {
    let lower = text.to_lowercase();
    let sentences: Vec<&str> = lower.split(['.', '\n', ';', '!', '?']).collect();
    let matched: Vec<Cause> = CAUSES
        .iter()
        .copied()
        .filter(|cause| {
            cause.groups().iter().any(|group| {
                sentences
                    .iter()
                    .any(|sentence| group.iter().all(|word| sentence.contains(word)))
            })
        })
        .collect();
    match matched.as_slice() {
        [] => Named::Nothing,
        [one] => Named::Cause(*one),
        _ => Named::Several,
    }
}

/// The first word in `packet` that gives `cause` away, if there is one.
pub fn gives_away(packet: &str, cause: Cause) -> Option<&'static str> {
    let lower = packet.to_lowercase();
    cause.giveaways().iter().copied().find(|w| lower.contains(w))
}

/// The first question: the named cause is the true one.
pub fn cause_right(named: Named, truth: Cause) -> bool {
    named == Named::Cause(truth)
}

/// The second question: nothing the answer cites is unsupported, and the chain
/// was not withheld whole.
pub fn ids_exist(unsupported: &[String], text: &str) -> bool {
    unsupported.is_empty() && !text.starts_with(WITHHELD_PREFIX)
}

/// How one answer scored on one case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaseScore {
    pub case: String,
    pub truth: Cause,
    pub named: Named,
    pub cause_right: bool,
    pub ids_exist: bool,
    /// `None` when the named cause calls for no remedy the console can prove.
    pub remedy_stops: Option<bool>,
}

/// What a set of scores adds up to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Totals {
    pub cases: usize,
    pub cause_right: usize,
    pub ids_exist: usize,
    /// How many scores had a remedy to replay at all.
    pub remedy_applicable: usize,
    pub remedy_stops: usize,
}

pub fn tally(scores: &[CaseScore]) -> Totals {
    Totals {
        cases: scores.len(),
        cause_right: scores.iter().filter(|s| s.cause_right).count(),
        ids_exist: scores.iter().filter(|s| s.ids_exist).count(),
        remedy_applicable: scores.iter().filter(|s| s.remedy_stops.is_some()).count(),
        remedy_stops: scores
            .iter()
            .filter(|s| s.remedy_stops == Some(true))
            .count(),
    }
}

fn mark(pass: bool) -> &'static str {
    if pass { "yes" } else { "NO" }
}

/// The report, as text: one line per case, the three questions, and the totals.
pub fn render(scores: &[CaseScore]) -> String {
    let mut out = String::from("case                 truth        named        cause  ids  remedy\n");
    for s in scores {
        let named = match s.named {
            Named::Cause(c) => c.label(),
            Named::Nothing => "(nothing)",
            Named::Several => "(several)",
        };
        let remedy = match s.remedy_stops {
            Some(stops) => mark(stops),
            None => "n/a",
        };
        out.push_str(&format!(
            "{:<20} {:<12} {:<12} {:<6} {:<4} {}\n",
            s.case,
            s.truth.label(),
            named,
            mark(s.cause_right),
            mark(s.ids_exist),
            remedy
        ));
    }
    let t = tally(scores);
    out.push_str(&format!(
        "named the cause right: {} of {}; every cited id exists: {} of {}; the remedy stops the replay: {} of {}\n",
        t.cause_right, t.cases, t.ids_exist, t.cases, t.remedy_stops, t.remedy_applicable
    ));
    out
}

#[cfg(test)]
mod tests;
