//! The budget proof: what a candidate budget or step limit would have done to the
//! calls the gateway recorded, read from `tokenfuse backtest`, in both halves.
//!
//! # WHY THIS EXISTS
//!
//! A budget set after an incident is a guess about the next one. `tokenfuse
//! backtest` replays a candidate over the recorded trace and says what it would
//! have blocked and saved, and slice 4 of the incident-to-proven-fix plan runs
//! that CLI inside the console, the way slice 3 runs `wardryx replay`.
//!
//! # THE TWO HALVES (project rule 4)
//!
//! `tokenfuse backtest` prints totals over the whole trace and nothing per run,
//! so the first half would be unanswerable from it. The incident's own calls are
//! therefore copied into a trace of their own (`tokenfuse sql` with `copy ... to
//! ... stored as parquet`) and the same CLI replays that, so the verdict on the
//! incident is the engine's and not a restatement of its rules. The second half
//! is the CLI's report over the whole trace less the incident's own numbers.
//! Calls that share a step are loaded in an order the CLI does not promise, so
//! when the incident's run has any the split is marked as approximate rather than
//! presented as exact.
//!
//! # WHAT THE CLI IS, AND WHY ITS OUTPUT IS NOT TRUSTED
//!
//! `tokenfuse` prints text, exits 0 on a flag it dropped, on a trace with no
//! calls and on an error of its own, and reads `--budget` as `(v * 1e6) as i64`,
//! which truncates: asked for `0.000249` it tests a limit of 248 micro-dollars.
//! So an amount is handed over as text that reads back to exactly the micros
//! asked for (checked here with the same arithmetic), the report must echo the
//! candidate that was asked for, and a report whose numbers do not hold together
//! is refused. A run id is a producer's text and goes into a query, so only names
//! made of a short set of characters are put there at all.

use crate::case::Data;
use serde::Serialize;

/// Money in the trace is whole micro-dollars.
pub const MICROS_PER_USD: i64 = 1_000_000;

/// The most one limit may be, in micro-dollars: a million dollars.
pub const MAX_LIMIT_MICROS: i64 = 1_000_000 * MICROS_PER_USD;

/// The most steps one limit may name.
pub const MAX_STEPS_LIMIT: u32 = 1_000_000;

/// The longest run id a proof puts in a query.
pub const RUN_ID_MAX_BYTES: usize = 128;

/// What a budget proof replays, stated with the proof.
pub const BUDGET_COVERS: &str = "the budget per run, the budget per step and the step limit, \
                                 replayed over the calls the gateway recorded";

const AMOUNT_SHAPE: &str = "an amount is dollars with at most six decimals, such as 2 or 0.35";
const AMOUNT_ZERO: &str = "a limit of zero blocks every call, which is not a candidate budget";
const AMOUNT_LARGE: &str = "an amount is at most a million dollars";
const AMOUNT_UNREADABLE: &str = "the amount cannot be handed to tokenfuse exactly";

/// A dollar amount as whole micro-dollars. Digits, at most six decimals, no sign,
/// no exponent, not zero, not more than a million dollars.
pub fn parse_usd(text: &str) -> Result<i64, &'static str> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty()
        || !all_digits(whole)
        || !all_digits(fraction)
        || fraction.len() > 6
        || (text.contains('.') && fraction.is_empty())
    {
        return Err(AMOUNT_SHAPE);
    }
    let whole: i64 = whole.parse().map_err(|_| AMOUNT_LARGE)?;
    let mut decimals: i64 = if fraction.is_empty() {
        0
    } else {
        fraction.parse().map_err(|_| AMOUNT_SHAPE)?
    };
    for _ in fraction.len()..6 {
        decimals *= 10;
    }
    let micros = whole
        .checked_mul(MICROS_PER_USD)
        .and_then(|w| w.checked_add(decimals))
        .ok_or(AMOUNT_LARGE)?;
    if micros == 0 {
        return Err(AMOUNT_ZERO);
    }
    if micros > MAX_LIMIT_MICROS {
        return Err(AMOUNT_LARGE);
    }
    Ok(micros)
}

/// The text to hand `tokenfuse backtest --budget` so that it reads exactly
/// `micros`.
///
/// The CLI reads a flag as `(v * 1e6) as i64`, which truncates, so the shortest
/// text of `micros / 1e6` is read one micro-dollar low for about 1.2% of
/// amounts (249 reads as 248). Half a micro-dollar above is read back as `micros`
/// every time, and the result is checked here with the same arithmetic, so an
/// amount that could not be handed over exactly is refused and never run.
pub fn cli_amount(micros: i64) -> Result<String, &'static str> {
    if !(1..=MAX_LIMIT_MICROS).contains(&micros) {
        return Err(AMOUNT_LARGE);
    }
    let text = ((micros as f64 + 0.5) / 1e6).to_string();
    match text.parse::<f64>() {
        Ok(read) if (read * 1e6) as i64 == micros => Ok(text),
        _ => Err(AMOUNT_UNREADABLE),
    }
}

/// A candidate budget, in the units the trace keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BudgetCandidate {
    /// The most one run may spend. A run that would pass it is stopped there.
    pub per_run_micros: Option<i64>,
    /// The most one call may cost. A call that passes it is blocked alone.
    pub per_step_micros: Option<i64>,
    /// The most calls one run may make before it is stopped.
    pub max_steps: Option<u32>,
}

impl BudgetCandidate {
    /// A candidate from what an operator typed. At least one limit, each valid.
    pub fn from_parts(
        per_run: Option<&str>,
        per_step: Option<&str>,
        max_steps: Option<u32>,
    ) -> Result<Self, &'static str> {
        let per_run_micros = per_run.map(parse_usd).transpose()?;
        let per_step_micros = per_step.map(parse_usd).transpose()?;
        if let Some(n) = max_steps
            && (n == 0 || n > MAX_STEPS_LIMIT)
        {
            return Err("the step limit is between 1 and 1,000,000");
        }
        if per_run_micros.is_none() && per_step_micros.is_none() && max_steps.is_none() {
            return Err("the candidate sets no limit");
        }
        Ok(Self {
            per_run_micros,
            per_step_micros,
            max_steps,
        })
    }

    /// The flags `tokenfuse backtest` is run with, in the order it prints them.
    pub fn args(&self) -> Result<Vec<String>, &'static str> {
        let mut args = Vec::new();
        if let Some(micros) = self.per_run_micros {
            args.push("--budget".to_string());
            args.push(cli_amount(micros)?);
        }
        if let Some(micros) = self.per_step_micros {
            args.push("--budget-per-step".to_string());
            args.push(cli_amount(micros)?);
        }
        if let Some(n) = self.max_steps {
            args.push("--max-steps".to_string());
            args.push(n.to_string());
        }
        Ok(args)
    }

    /// The line `tokenfuse backtest` prints for this candidate, which a report
    /// must carry to be taken as a report for it.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(micros) = self.per_run_micros {
            parts.push(format!("budget/run ${:.4}", micros as f64 / 1e6));
        }
        if let Some(micros) = self.per_step_micros {
            parts.push(format!("budget/step ${:.4}", micros as f64 / 1e6));
        }
        if let Some(n) = self.max_steps {
            parts.push(format!("max_steps {n}"));
        }
        parts.join(", ")
    }
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-')
}

fn is_path_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/')
}

/// A run id the proof will put in a query: a short name made of letters, digits
/// and `. _ : -`. Anything else is refused, because the id is a producer's text.
pub fn query_safe_run(run: &str) -> Result<(), &'static str> {
    if run.is_empty() || run.len() > RUN_ID_MAX_BYTES {
        return Err("the run id is empty or longer than 128 bytes");
    }
    if !run.bytes().all(is_name_byte) {
        return Err("the run id holds characters the proof will not put in a query");
    }
    Ok(())
}

/// The statement that copies one run's calls, in the order they happened, into a
/// trace of its own at `file`.
pub fn copy_statement(run: &str, file: &str) -> Result<String, &'static str> {
    query_safe_run(run)?;
    if file.is_empty() || !file.bytes().all(is_path_byte) {
        return Err("the scratch path holds characters the proof will not put in a query");
    }
    Ok(format!(
        "copy (select * from calls where run_id = '{run}' order by ts_millis, step) \
         to '{file}' stored as parquet"
    ))
}

/// The statement that counts a trace's calls and the distinct steps among them.
pub const STEP_COUNT_STATEMENT: &str =
    "select count(*) as n, count(distinct step) as steps from calls";

/// A table `tokenfuse sql` prints: the header row, then each row, cells trimmed.
/// Anything that is not that table is `None`.
pub fn parse_table(text: &str) -> Option<Vec<Vec<String>>> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 4 {
        return None;
    }
    let border = |l: &str| l.len() >= 3 && l.bytes().all(|b| b == b'+' || b == b'-');
    if !border(lines[0]) || !border(lines[2]) || !border(lines[lines.len() - 1]) {
        return None;
    }
    let cells = |l: &str| -> Option<Vec<String>> {
        let inner = l.strip_prefix("| ")?.strip_suffix(" |")?;
        Some(inner.split(" | ").map(|c| c.trim().to_string()).collect())
    };
    let header = cells(lines[1])?;
    let mut table = vec![header.clone()];
    for line in &lines[3..lines.len() - 1] {
        let row = cells(line)?;
        if row.len() != header.len() {
            return None;
        }
        table.push(row);
    }
    Some(table)
}

fn count_of(cell: &str) -> Option<u64> {
    if cell.is_empty() || !cell.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    cell.parse().ok()
}

/// How many rows a `copy` reported writing.
pub fn parse_copy_count(text: &str) -> Option<u64> {
    let table = parse_table(text)?;
    let [header, row] = table.as_slice() else {
        return None;
    };
    if header.len() != 1 || header[0] != "count" || row.len() != 1 {
        return None;
    }
    count_of(&row[0])
}

/// How many calls a trace holds and how many distinct steps they carry.
pub fn parse_step_counts(text: &str) -> Option<(u64, u64)> {
    let table = parse_table(text)?;
    let [header, row] = table.as_slice() else {
        return None;
    };
    if header.len() != 2 || header[0] != "n" || header[1] != "steps" || row.len() != 2 {
        return None;
    }
    Some((count_of(&row[0])?, count_of(&row[1])?))
}

/// What `tokenfuse backtest` reported over one trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BacktestReport {
    pub runs_total: u64,
    pub runs_affected: u64,
    pub calls_total: u64,
    pub calls_blocked: u64,
    /// What the trace spent, in micro-dollars.
    pub spent_micros: i64,
    /// What the candidate would have prevented.
    pub saved_micros: i64,
}

/// Why a backtest report could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BacktestParseError {
    /// The trace holds no calls.
    NoCalls,
    /// The text is not a `tokenfuse backtest` report.
    Unrecognised,
    /// The report contradicts itself, or is for another candidate.
    Inconsistent(&'static str),
}

const HEAD: &str = "TokenFuse backtest";
const CANDIDATE_LINE: &str = "  candidate policy   : ";
const RUNS_LINE: &str = "  runs               : ";
const CALLS_LINE: &str = "  calls              : ";
const SPEND_LINE: &str = "  spend in trace     : ";
const SAVED_LINE: &str = "  would have saved   : ";
const PROJECTED_LINE: &str = "  projected spend    : ";

/// `$0.256379` or `$-0.000100`: the CLI's dollars with six decimals, as micros.
fn money(text: &str) -> Option<i64> {
    let rest = text.strip_prefix('$')?;
    let (negative, digits) = match rest.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, rest),
    };
    let (whole, fraction) = digits.split_once('.')?;
    if whole.is_empty()
        || fraction.len() != 6
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let micros = whole
        .parse::<i64>()
        .ok()?
        .checked_mul(MICROS_PER_USD)?
        .checked_add(fraction.parse::<i64>().ok()?)?;
    Some(if negative { -micros } else { micros })
}

/// `$0.226496 (88.3%)`: the saved amount, with a percentage that is only a number.
fn saved_amount(text: &str) -> Option<i64> {
    let (amount, rest) = text.split_once(" (")?;
    let percent = rest.strip_suffix("%)")?;
    if percent.is_empty() || !percent.bytes().all(|b| b.is_ascii_digit() || b == b'.' || b == b'-')
    {
        return None;
    }
    money(amount)
}

/// `230 (31 affected)`: a total and the part of it named in `tail`.
fn pair(text: &str, tail: &str) -> Option<(u64, u64)> {
    let (total, rest) = text.split_once(" (")?;
    let part = rest.strip_suffix(tail)?;
    Some((count_of(total)?, count_of(part)?))
}

/// Read a `tokenfuse backtest` report for `asked`.
///
/// The format is `backtestcli::run`'s own: a head line and six labelled lines.
/// A report is refused when its candidate line is not the candidate that was
/// asked for (the CLI drops a flag it cannot read and says so on another
/// stream), when more runs are affected than there are runs, when a run is
/// affected with nothing blocked or the other way round, or when its projected
/// spend is not the spend less the saving.
pub fn parse_backtest(
    text: &str,
    asked: &BudgetCandidate,
) -> Result<BacktestReport, BacktestParseError> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() == 1 && lines[0].starts_with("no calls in the trace at ") {
        return Err(BacktestParseError::NoCalls);
    }
    if lines.len() != 7 || lines[0] != HEAD {
        return Err(BacktestParseError::Unrecognised);
    }
    let unread = || BacktestParseError::Unrecognised;
    let echoed = lines[1].strip_prefix(CANDIDATE_LINE).ok_or_else(unread)?;
    let (runs_total, runs_affected) = lines[2]
        .strip_prefix(RUNS_LINE)
        .and_then(|v| pair(v, " affected)"))
        .ok_or_else(unread)?;
    let (calls_total, calls_blocked) = lines[3]
        .strip_prefix(CALLS_LINE)
        .and_then(|v| pair(v, " would be blocked)"))
        .ok_or_else(unread)?;
    let spent = lines[4]
        .strip_prefix(SPEND_LINE)
        .and_then(money)
        .ok_or_else(unread)?;
    let saved = lines[5]
        .strip_prefix(SAVED_LINE)
        .and_then(saved_amount)
        .ok_or_else(unread)?;
    let projected = lines[6]
        .strip_prefix(PROJECTED_LINE)
        .and_then(money)
        .ok_or_else(unread)?;

    let inconsistent = |reason| Err(BacktestParseError::Inconsistent(reason));
    if echoed != asked.describe() {
        return inconsistent("the report is for another candidate than the one that was asked");
    }
    if runs_affected > runs_total || calls_blocked > calls_total || runs_affected > calls_blocked {
        return inconsistent("the report affects more runs or calls than it holds");
    }
    if (runs_affected > 0) != (calls_blocked > 0) {
        return inconsistent("the report blocks calls in no run, or affects a run with none");
    }
    if Some(projected) != spent.checked_sub(saved) {
        return inconsistent("the projected spend is not the spend less the saving");
    }
    Ok(BacktestReport {
        runs_total,
        runs_affected,
        calls_total,
        calls_blocked,
        spent_micros: spent,
        saved_micros: saved,
    })
}

/// The incident's own run, replayed alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RunEffect {
    pub calls: u64,
    pub calls_blocked: u64,
    pub spent_micros: i64,
    pub saved_micros: i64,
    /// What the run would have spent under the candidate.
    pub spent_after_micros: i64,
}

/// The rest of the trace: the whole-trace report less the incident's own run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BudgetElsewhere {
    pub runs: u64,
    pub runs_affected: u64,
    pub calls: u64,
    pub calls_blocked: u64,
    pub saved_micros: i64,
}

/// A budget proof, in both halves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BudgetProof {
    /// The incident's run. A producer's text.
    pub run: Data,
    pub candidate: BudgetCandidate,
    /// First half: the candidate would have blocked at least one of the run's calls.
    pub stops_this_incident: bool,
    pub incident: RunEffect,
    /// Second half: what the candidate would have done to everything else.
    pub elsewhere: BudgetElsewhere,
    /// False when the incident's run has calls at the same step, whose order the
    /// whole-trace replay does not promise: `elsewhere` is then approximate.
    pub elsewhere_exact: bool,
    /// The whole-trace report as `tokenfuse backtest` printed it.
    pub trace: BacktestReport,
    pub covers: &'static str,
}

/// Turn the two reports into a proof about the incident that ran as `run_id`.
///
/// `run` is the replay of the incident's own calls, `trace` the replay of the
/// whole trace, `copied` how many calls were copied out for the first, and
/// `tied` whether the run has calls at the same step. A pair of reports that
/// cannot both be about the same trace is refused.
pub fn judge_budget(
    run_id: &str,
    candidate: &BudgetCandidate,
    run: &BacktestReport,
    trace: &BacktestReport,
    copied: u64,
    tied: bool,
) -> Result<BudgetProof, &'static str> {
    if run.runs_total != 1 {
        return Err("the incident's run was replayed together with other runs");
    }
    if run.calls_total != copied {
        return Err("the copy of the run and its replay disagree about how many calls it has");
    }
    if trace.runs_total < 1 {
        return Err("the whole trace holds no run, and the incident's run was copied from it");
    }
    if trace.calls_total < run.calls_total {
        return Err("the whole trace holds fewer calls than the incident's own run");
    }
    if trace.runs_affected < run.runs_affected || trace.calls_blocked < run.calls_blocked {
        return Err("the whole trace blocks less than the incident's own run");
    }
    let stops = run.calls_blocked > 0;
    Ok(BudgetProof {
        run: Data::bounded(run_id),
        candidate: *candidate,
        stops_this_incident: stops,
        incident: RunEffect {
            calls: run.calls_total,
            calls_blocked: run.calls_blocked,
            spent_micros: run.spent_micros,
            saved_micros: run.saved_micros,
            spent_after_micros: run.spent_micros.saturating_sub(run.saved_micros),
        },
        elsewhere: BudgetElsewhere {
            runs: trace.runs_total - run.runs_total,
            runs_affected: trace.runs_affected - run.runs_affected,
            calls: trace.calls_total - run.calls_total,
            calls_blocked: trace.calls_blocked - run.calls_blocked,
            saved_micros: trace.saved_micros.saturating_sub(run.saved_micros),
        },
        elsewhere_exact: !tied,
        trace: *trace,
        covers: BUDGET_COVERS,
    })
}

#[cfg(test)]
mod tests;
