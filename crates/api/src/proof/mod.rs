//! The policy proof: what a candidate policy would have done to the decisions
//! already taken, read from `wardryx replay`, reported in both halves.
//!
//! # WHY THIS EXISTS
//!
//! An operator who reads a root cause and then edits a policy by hand is
//! changing enforcement blind. `wardryx replay` takes the decisions that
//! happened and puts each recorded question back to the policy engine, first
//! against the version it was decided under (so a counterfactual is offered
//! only for rows that reproduced), then against a candidate. Slice 3 of the
//! incident-to-proven-fix plan runs that CLI inside the console, the way qryx
//! and mockryx already run, and turns its report into a proof.
//!
//! # THE TWO HALVES (project rule 4)
//!
//! A proof says both whether the candidate would have stopped THIS incident
//! (the recorded decisions of the incident's run that it turns from `allow`
//! into `deny` or `hold`) and how many OTHER decisions it would have changed,
//! tightened or loosened. Wardryx's named outcomes (`not archived`,
//! `unreadable`, `diverged`, the three approval rows) are carried as they are
//! and never folded into a count: a change tally that silently omitted what
//! could not be replayed would read as coverage it does not have.
//!
//! # WHAT THE REPORT IS, AND WHY IT IS NOT TRUSTED
//!
//! `wardryx replay` prints text, and the text carries agent ids, run ids and
//! the reasons the policy gave, all of which a producer chose. A run id with a
//! newline in it can print a row that was never replayed. So the parser takes
//! the counts from the lines wardryx writes about itself and refuses a report
//! whose rows do not add up to them: the number of rows must equal the number
//! the report says changed, and the seven outcomes must sum to the number
//! replayed. Text from the report is carried as [`Data`], never as the
//! console's own words.
//!
//! # WHAT IT DOES NOT DO
//!
//! It applies nothing (project rule 6) and a proof that was not run is never
//! a proof that nothing changes: an unconfigured archive, a missing binary, a
//! timeout and a report that does not parse are each an error that says so.

pub mod commands;

use crate::case::Data;
use serde::Serialize;

/// The seven rows `wardryx replay` counts. Each is carried by its name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Outcomes {
    pub reproduced: u64,
    pub approval_decided: u64,
    pub approval_spent: u64,
    pub approval_refused: u64,
    pub not_archived: u64,
    pub unreadable: u64,
    pub diverged: u64,
}

impl Outcomes {
    /// Every decision the run found: the seven rows added.
    pub fn total(&self) -> u64 {
        self.reproduced
            + self.approval_decided
            + self.approval_spent
            + self.approval_refused
            + self.not_archived
            + self.unreadable
            + self.diverged
    }

    /// The decisions that carried a counterfactual.
    pub fn replayable(&self) -> u64 {
        self.reproduced + self.approval_decided + self.approval_spent + self.approval_refused
    }
}

/// One decision the candidate would have taken differently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangedRow {
    /// The verdict the engine reached at the time, `allow`, `deny` or `hold`.
    pub from: String,
    /// The verdict the candidate reaches.
    pub to: String,
    /// Whose decision it was. Text a producer chose.
    pub agent: Data,
    /// Which run. Text a producer chose.
    pub run: Data,
    /// The reason the policy gave then.
    pub was: Data,
    /// The reason the candidate gives.
    pub now: Data,
}

/// What `wardryx replay` reported against a candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandidateResult {
    /// How many replayable decisions change.
    pub changed: u64,
    /// How many were replayable at all.
    pub replayable: u64,
    pub rows: Vec<ChangedRow>,
}

/// A parsed `wardryx replay` report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReplayReport {
    pub replayed: u64,
    pub outcomes: Outcomes,
    /// `None` when no candidate was given.
    pub candidate: Option<CandidateResult>,
}

/// Why a report could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The text is not a `wardryx replay` report.
    Unrecognised,
    /// The report contradicts itself: its rows or its counts do not add up.
    Inconsistent(&'static str),
}

const LABELS: [&str; 7] = [
    "reproduced",
    "approval-decided",
    "approval-spent",
    "approval-refused",
    "not archived",
    "unreadable",
    "diverged",
];

/// The integer after `label` on a counter line, `None` if the line is not that
/// counter's.
fn counter(line: &str, label: &str) -> Option<u64> {
    let rest = line.trim_start().strip_prefix(label)?;
    if !rest.starts_with(' ') {
        return None;
    }
    rest.split_whitespace().next()?.parse().ok()
}

/// Read a `wardryx replay` report.
///
/// The format is wardryx's own (`internal/replay`'s `Format`) and is
/// documented in its README. The first line names how many decisions were
/// replayed, seven counter lines follow, and with a candidate the report says
/// `Against <name>, X of Y replayable decision(s) change:` and lists each
/// change as three lines. A report whose seven counters do not sum to the
/// first line, or whose listed rows are not as many as it says changed, is
/// refused.
pub fn parse_report(text: &str) -> Result<ReplayReport, ParseError> {
    let lines: Vec<&str> = text.lines().collect();
    let first = lines.first().ok_or(ParseError::Unrecognised)?;
    let replayed: u64 = first
        .strip_prefix("Replayed ")
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .ok_or(ParseError::Unrecognised)?;

    if replayed == 0 {
        return if lines
            .iter()
            .any(|l| l.contains("no decisions in this file"))
        {
            Ok(ReplayReport {
                replayed: 0,
                outcomes: Outcomes::default(),
                candidate: None,
            })
        } else {
            Err(ParseError::Unrecognised)
        };
    }

    let mut found = [None; 7];
    for line in &lines[1..] {
        for (slot, label) in found.iter_mut().zip(LABELS) {
            if slot.is_none() {
                *slot = counter(line, label);
            }
        }
    }
    let [
        Some(reproduced),
        Some(approval_decided),
        Some(approval_spent),
        Some(approval_refused),
        Some(not_archived),
        Some(unreadable),
        Some(diverged),
    ] = found
    else {
        return Err(ParseError::Unrecognised);
    };
    let outcomes = Outcomes {
        reproduced,
        approval_decided,
        approval_spent,
        approval_refused,
        not_archived,
        unreadable,
        diverged,
    };
    if outcomes.total() != replayed {
        return Err(ParseError::Inconsistent(
            "the outcomes do not add up to the decisions replayed",
        ));
    }

    let candidate = parse_candidate(&lines)?;
    if let Some(c) = &candidate
        && c.replayable != outcomes.replayable()
    {
        return Err(ParseError::Inconsistent(
            "the replayable decisions differ from the outcomes that carry a counterfactual",
        ));
    }
    Ok(ReplayReport {
        replayed,
        outcomes,
        candidate,
    })
}

/// The `Against ...` section, if there is one.
fn parse_candidate(lines: &[&str]) -> Result<Option<CandidateResult>, ParseError> {
    let Some(at) = lines.iter().position(|l| l.starts_with("Against ")) else {
        return Ok(None);
    };
    let head = lines[at]
        .strip_suffix(" replayable decision(s) change:")
        .ok_or(ParseError::Unrecognised)?;
    let (_, counts) = head.rsplit_once(", ").ok_or(ParseError::Unrecognised)?;
    let (changed, replayable) = counts.split_once(" of ").ok_or(ParseError::Unrecognised)?;
    let changed: u64 = changed
        .trim()
        .parse()
        .map_err(|_| ParseError::Unrecognised)?;
    let replayable: u64 = replayable
        .trim()
        .parse()
        .map_err(|_| ParseError::Unrecognised)?;

    let mut rows = Vec::new();
    let mut i = at + 1;
    while i < lines.len() {
        let Some((from, rest)) = lines[i].trim_start().split_once(" -> ") else {
            i += 1;
            continue;
        };
        let (Some(was), Some(now)) = (
            lines
                .get(i + 1)
                .and_then(|l| l.trim_start().strip_prefix("was: ")),
            lines
                .get(i + 2)
                .and_then(|l| l.trim_start().strip_prefix("now: ")),
        ) else {
            i += 1;
            continue;
        };
        let mut words = rest.split_whitespace();
        let to = words.next().unwrap_or_default();
        let agent = words.next().unwrap_or_default();
        let run = match words.next() {
            Some("run") => words.next().unwrap_or_default(),
            _ => "",
        };
        if !matches!(from, "allow" | "deny" | "hold") || !matches!(to, "allow" | "deny" | "hold") {
            i += 1;
            continue;
        }
        rows.push(ChangedRow {
            from: from.to_string(),
            to: to.to_string(),
            agent: Data::bounded(agent),
            run: Data::bounded(run),
            was: Data::bounded(was),
            now: Data::bounded(now),
        });
        i += 3;
    }
    if rows.len() as u64 != changed {
        return Err(ParseError::Inconsistent(
            "the rows listed are not as many as the report says changed",
        ));
    }
    Ok(Some(CandidateResult {
        changed,
        replayable,
        rows,
    }))
}

/// How the changes elsewhere sort.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Elsewhere {
    /// Allowed then, denied or held under the candidate.
    pub tightened: u64,
    /// Denied or held then, allowed under the candidate.
    pub loosened: u64,
    /// Deny to hold or hold to deny.
    pub reclassified: u64,
}

/// A proof, in both halves, with what could not be examined beside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolicyProof {
    pub replayed: u64,
    /// Wardryx's own named outcomes, never folded.
    pub outcomes: Outcomes,
    /// False when any recorded decision diverged: the record and the engine no
    /// longer agree about the past, and nothing below is to be relied on.
    pub trustworthy: bool,
    /// Whether a candidate was given. Without one only replayability is known.
    pub candidate_given: bool,
    /// First half: would the candidate have stopped this incident. `None`
    /// without a candidate and when the replay diverged.
    pub stops_this_incident: Option<bool>,
    /// The incident's own decisions the candidate changes.
    pub incident_changes: Vec<ChangedRow>,
    /// Second half: how many decisions that are not the incident's it changes.
    pub elsewhere: Elsewhere,
    /// The changes elsewhere, each listed.
    pub elsewhere_changes: Vec<ChangedRow>,
}

/// Turn a report into a proof about the incident that ran as `incident_run`.
///
/// A change counts as the incident's only when its run is the incident's run;
/// everything else is elsewhere. The incident is stopped when the candidate
/// turns one of its decisions from `allow` into `deny` or `hold`.
pub fn judge(report: &ReplayReport, incident_run: &str) -> PolicyProof {
    let trustworthy = report.outcomes.diverged == 0;
    let Some(candidate) = &report.candidate else {
        return PolicyProof {
            replayed: report.replayed,
            outcomes: report.outcomes,
            trustworthy,
            candidate_given: false,
            stops_this_incident: None,
            incident_changes: Vec::new(),
            elsewhere: Elsewhere::default(),
            elsewhere_changes: Vec::new(),
        };
    };
    let (incident_changes, elsewhere_changes): (Vec<ChangedRow>, Vec<ChangedRow>) = candidate
        .rows
        .iter()
        .cloned()
        .partition(|row| row.run.text == incident_run);
    let stops = incident_changes
        .iter()
        .any(|r| r.from == "allow" && (r.to == "deny" || r.to == "hold"));
    let mut elsewhere = Elsewhere::default();
    for row in &elsewhere_changes {
        match (row.from.as_str(), row.to.as_str()) {
            ("allow", _) => elsewhere.tightened += 1,
            (_, "allow") => elsewhere.loosened += 1,
            _ => elsewhere.reclassified += 1,
        }
    }
    PolicyProof {
        replayed: report.replayed,
        outcomes: report.outcomes,
        trustworthy,
        candidate_given: true,
        stops_this_incident: trustworthy.then_some(stops),
        incident_changes,
        elsewhere,
        elsewhere_changes,
    }
}

#[cfg(test)]
mod tests;
