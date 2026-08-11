//! What ONE agent's normal looks like, and how today compares to it.
//!
//! # WHY A PROFILE AND NOT A BIGGER COUNTER
//!
//! [`super::stats_counts`] answers "how many". That is enough to rank a fleet
//! and not enough to describe an agent: twenty-six stops in an hour and
//! twenty-six across a month are the same number and different situations, and
//! the number cannot tell them apart.
//!
//! This compares an agent to ITSELF over time, which is the only comparison
//! that means anything here. A busy agent is not an abnormal agent, so nothing
//! in this module compares one agent to another.
//!
//! # WHAT IT DELIBERATELY DOES NOT PROFILE
//!
//! Money. The bus carries no per-call cost (`tool_call` comes from the MCP
//! broker with `{tool, upstream, decision}` and no price), so a spend profile
//! would have to come from the money plane, and the money plane already has
//! one: `spend_spike` compares a run's burn against its own recent baseline.
//! A second spend baseline computed here would be a second number for one
//! question, which is the defect this console has already paid for once.
//!
//! # WHY THE MEDIAN, AND WHY NOT A STANDARD DEVIATION
//!
//! These counts are mostly zero with occasional spikes, so the distribution is
//! heavy-tailed. A standard deviation computed over it is inflated BY the
//! spikes, so the spike hides itself: the more unusual the day, the more
//! "normal" it scores. A z-score here would look like statistics and be a
//! guess.
//!
//! The median of an agent's own days is checkable. "Three times its median
//! day, over 62 days" is a sentence somebody can verify by looking at the days.
//!
//! # THE ZERO DAYS ARE THE POINT
//!
//! The store returns no row for a day with no events. Taking the median over
//! only the days that HAVE rows gives the median of an agent's busy days, and
//! against that a quiet agent's first bad day looks ordinary. Every day from
//! the agent's first event to now is filled with zero first.
//!
//! # NO STORAGE
//!
//! There is no rollup table. A rollup is a cache, and a cache that can disagree
//! with the events will, quietly, because both numbers stay plausible. This
//! store already carries `rollup_spend_1m`, added "for later" in the first
//! migration and never populated by anything, which is what a speculative
//! rollup is worth. Recomputing from `events` keeps one source of truth; the
//! index `idx_events_agent_ts_ms` is what makes that cheap.
//!
//! Revisit on a MEASURED query time from a real box, not on a feeling.

use serde::Serialize;
use std::collections::BTreeMap;

use crate::bus::AppState;

/// Milliseconds in a UTC day, the unit the store's day index is in.
const DAY_MS: i64 = 86_400_000;

/// How many days of history a profile needs before it will call anything
/// unusual.
///
/// Two weeks. Short enough that a new agent gets a profile inside a sprint,
/// long enough that a weekly rhythm (a Monday batch job, a Friday report) has
/// been seen at least twice. Below it the profile still reports its counts and
/// refuses the comparison, rather than dividing by a number it does not have.
const MIN_DAYS_FOR_NORMAL: i64 = 14;

/// The window either side of the split used for "is this rising".
const TREND_DAYS: i64 = 7;

/// Whether an agent has been watched long enough for "unusual" to mean
/// anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Nothing stored for this agent at all.
    NoData,
    /// Seen, but for fewer than [`MIN_DAYS_FOR_NORMAL`] days. Counts are real;
    /// the comparison is refused.
    TooNew,
    /// Enough history to compare a day against the agent's own median.
    Normal,
}

/// Which way a series is going, over the last [`TREND_DAYS`] against the
/// [`TREND_DAYS`] before them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Rising,
    Falling,
    Steady,
    /// Not enough days on both sides of the split to compare.
    Unknown,
}

/// One agent's rhythm, and how its latest complete day sits in it.
#[derive(Debug, Clone, Serialize)]
pub struct AgentProfile {
    pub agent_id: String,
    pub confidence: Confidence,
    /// Days from the agent's first stored event to now, capped at the window.
    /// Always reported, including when `confidence` refuses the comparison:
    /// "we have watched this for 3 days" is the useful half of a refusal.
    pub days_held: i64,
    /// Events in the window, all types.
    pub total: u64,

    /// The agent's median day, over EVERY day held including the empty ones.
    pub median_day: f64,
    /// The most recent COMPLETE UTC day. Today is deliberately excluded: a
    /// partial day compared against full days reads as a quiet agent every
    /// morning.
    pub latest_full_day: u64,
    /// `latest_full_day` as a multiple of `median_day`, or `None` when there is
    /// no usable median (too new, or a median of zero, where any activity is
    /// "infinitely more" and the multiple says nothing a count does not).
    pub times_median: Option<f64>,

    /// Share of the window's events landing on its single busiest day.
    /// Distinguishes "a bad afternoon" from "a bad month".
    ///
    /// Reported as the raw share, with no threshold turning it into
    /// "concentrated". A cutoff here would be this module inventing a verdict,
    /// and 0.49 and 0.51 are not different situations. Whoever renders it can
    /// decide what is worth highlighting; the number stays checkable.
    pub busiest_day_share: f64,
    pub direction: Direction,
    /// The type that fired most, and its share. "The same thing over and over"
    /// and "a different thing every time" are different situations with the
    /// same count, and this is the axis that separates them.
    pub top_type: Option<String>,
    pub top_type_share: f64,

    /// Daily totals oldest-first, zero-filled, for a sparkline. The reader can
    /// check every number above against this.
    pub daily: Vec<u64>,
}

impl AgentProfile {
    fn empty(agent_id: &str) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            confidence: Confidence::NoData,
            days_held: 0,
            total: 0,
            median_day: 0.0,
            latest_full_day: 0,
            times_median: None,
            busiest_day_share: 0.0,
            direction: Direction::Unknown,
            top_type: None,
            top_type_share: 0.0,
            daily: Vec::new(),
        }
    }
}

/// Build one agent's profile over the last `window_days`.
///
/// `now_ms` is passed in rather than read here so the whole thing is
/// deterministic under test; the caller supplies the clock.
pub fn agent_profile(
    agent_id: &str,
    window_days: i64,
    now_ms: i64,
    state: &AppState,
) -> Option<AgentProfile> {
    let dir = state.events_dir.as_ref()?;
    let store = genaryx_core::store::Store::open(&dir.join("console.sqlite")).ok()?;

    let today = now_ms / DAY_MS;
    let window_start_day = today - window_days.max(1) + 1;
    let rows = store
        .daily_type_counts(agent_id, window_start_day * DAY_MS)
        .ok()?;
    let first_day = store.first_day_for_agent(agent_id).ok()?;

    let Some(first_day) = first_day else {
        return Some(AgentProfile::empty(agent_id));
    };

    // Watched from the agent's first event, or from the window's start if it
    // is older than the window. Counting from the window start for an agent
    // that only appeared yesterday would claim 90 days of evidence for one.
    let start_day = first_day.max(window_start_day);
    let days_held = (today - start_day + 1).max(0);

    // Zero-fill. See this module's doc: a median over only the days with rows
    // is a median of the busy days.
    let mut per_day: BTreeMap<i64, u64> = BTreeMap::new();
    for d in start_day..=today {
        per_day.insert(d, 0);
    }
    let mut per_type: BTreeMap<String, u64> = BTreeMap::new();
    let mut total = 0u64;
    for r in &rows {
        if r.day < start_day {
            continue;
        }
        *per_day.entry(r.day).or_insert(0) += r.count;
        *per_type.entry(r.type_.clone()).or_insert(0) += r.count;
        total += r.count;
    }

    let daily: Vec<u64> = per_day.values().copied().collect();

    // The most recent COMPLETE day: yesterday. `daily` ends with today, which
    // is partial for all but the last instant of it.
    let latest_full_day = if daily.len() >= 2 {
        daily[daily.len() - 2]
    } else {
        0
    };

    // The median EXCLUDES the day being judged, so a big day cannot pull up the
    // baseline it is being compared against. With a long window that changes
    // almost nothing; with a short one it is the difference between a spike and
    // a shrug.
    let baseline: Vec<u64> = if daily.len() >= 2 {
        daily[..daily.len() - 2].to_vec()
    } else {
        Vec::new()
    };
    let median_day = median(&baseline);

    let confidence = if total == 0 && days_held == 0 {
        Confidence::NoData
    } else if days_held < MIN_DAYS_FOR_NORMAL {
        Confidence::TooNew
    } else {
        Confidence::Normal
    };

    // `None` on a zero median rather than a division: every non-zero day would
    // be "infinitely above normal", which is a true statement that tells a
    // reader less than the count already did.
    let times_median = if confidence == Confidence::Normal && median_day > 0.0 {
        Some((latest_full_day as f64 / median_day * 100.0).round() / 100.0)
    } else {
        None
    };

    let busiest = daily.iter().copied().max().unwrap_or(0);
    let busiest_day_share = if total > 0 {
        busiest as f64 / total as f64
    } else {
        0.0
    };

    let (top_type, top_type_share) = match per_type.iter().max_by_key(|(_, n)| **n) {
        Some((name, n)) if total > 0 => (Some(name.clone()), *n as f64 / total as f64),
        _ => (None, 0.0),
    };

    Some(AgentProfile {
        agent_id: agent_id.to_string(),
        confidence,
        days_held,
        total,
        median_day,
        latest_full_day,
        times_median,
        busiest_day_share: (busiest_day_share * 100.0).round() / 100.0,
        direction: direction_of(&daily),
        top_type,
        top_type_share: (top_type_share * 100.0).round() / 100.0,
        daily,
    })
}

/// True median: the average of the middle two on an even count, not the lower
/// of them. A profile that rounded its own baseline down would report every
/// even-length history as slightly more unusual than it is.
fn median(values: &[u64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v: Vec<u64> = values.to_vec();
    v.sort_unstable();
    let mid = v.len() / 2;
    if v.len() % 2 == 1 {
        v[mid] as f64
    } else {
        (v[mid - 1] as f64 + v[mid] as f64) / 2.0
    }
}

/// Last [`TREND_DAYS`] against the [`TREND_DAYS`] before them.
///
/// Sums rather than medians on purpose: the question here is whether there is
/// MORE of it lately, and a week that went from one quiet day and six empty
/// ones to four busy days has the same median and is not the same week.
fn direction_of(daily: &[u64]) -> Direction {
    let need = (TREND_DAYS * 2) as usize;
    if daily.len() < need {
        return Direction::Unknown;
    }
    let n = daily.len();
    let recent: u64 = daily[n - TREND_DAYS as usize..].iter().sum();
    let prior: u64 = daily[n - need..n - TREND_DAYS as usize].iter().sum();

    // Both empty is steady, not a division. A rise from zero is reported as
    // rising on the strength of the count alone.
    if recent == prior {
        return Direction::Steady;
    }
    if prior == 0 {
        return Direction::Rising;
    }
    let ratio = recent as f64 / prior as f64;
    // A twentieth either way is noise on counts this small. The band is wide
    // deliberately: a profile that called every wobble a trend would be
    // ignored within a week.
    if ratio >= 1.25 {
        Direction::Rising
    } else if ratio <= 0.8 {
        Direction::Falling
    } else {
        Direction::Steady
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::BusMode;
    use genaryx_core::event::{AgentEvent, ConsoleEvent, Provenance, SchemaVersion};
    use genaryx_core::store::Store;
    use std::path::PathBuf;

    const AGENT: &str = "agent://acme.local/sre/janitor";

    fn nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }

    /// `n` events of `type_` on the UTC day `day`, each at a distinct offset so
    /// the store's dedupe key (env, file, offset, raw) keeps them apart.
    fn day_events(day: i64, type_: &str, n: u64, seq: &mut u64) -> Vec<ConsoleEvent> {
        (0..n)
            .map(|i| {
                *seq += 1;
                let ts_ms = day * DAY_MS + (i as i64 * 1000);
                let ts = genaryx_core::store::ms_to_rfc3339(ts_ms);
                let raw = serde_json::json!({ "n": *seq, "ts": ts }).to_string();
                ConsoleEvent {
                    event: AgentEvent {
                        schema: SchemaVersion::SCHEMA_V0_2.to_string(),
                        ts,
                        source: "test".into(),
                        event_type: type_.to_string(),
                        agent_id: AGENT.to_string(),
                        severity: None,
                        run_id: None,
                        on_behalf_of: Vec::new(),
                        data: None,
                        prev_hash: None,
                        extra: Default::default(),
                    },
                    provenance: Provenance {
                        env: "local".into(),
                        connector: "test".into(),
                        file: Some("t.ndjson".into()),
                        offset: Some(*seq),
                        endpoint: None,
                        received_ts: "2026-08-11T00:00:00Z".into(),
                    },
                    raw,
                    schema_version: SchemaVersion::V0_2,
                }
            })
            .collect()
    }

    fn seeded(tag: &str, events: &[ConsoleEvent]) -> (AppState, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "genaryx-profile-{}-{tag}-{}",
            std::process::id(),
            nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create_dir_all");
        let store = Store::open(&dir.join("console.sqlite")).expect("open");
        store.insert_batch(events).expect("insert");
        (
            AppState {
                events_dir: Some(dir.clone()),
                source_events_dir: None,
                mode: BusMode::Unavailable {
                    reason: "test".into(),
                },
            },
            dir,
        )
    }

    /// The property the whole module rests on. An agent quiet for a month and
    /// then loud for one day must read as loud, and it only does if the quiet
    /// days count toward the median. Taking the median over the days that HAVE
    /// events gives 30, and the bad day then looks ordinary.
    #[test]
    fn the_empty_days_count_toward_the_median() {
        let today = 20_000_i64;
        let mut seq = 0;
        let mut events = Vec::new();
        // One event on each of two days, thirty days apart, then thirty on the
        // most recent COMPLETE day.
        events.extend(day_events(today - 30, "policy_deny", 1, &mut seq));
        events.extend(day_events(today - 15, "policy_deny", 1, &mut seq));
        events.extend(day_events(today - 1, "policy_deny", 30, &mut seq));

        let (state, dir) = seeded("zero-days", &events);
        let p = agent_profile(AGENT, 90, today * DAY_MS + 1000, &state).expect("profile");

        assert_eq!(p.confidence, Confidence::Normal, "31 days is enough");
        assert_eq!(p.days_held, 31);
        assert_eq!(p.latest_full_day, 30);
        assert_eq!(
            p.median_day, 0.0,
            "a mostly-empty month has a median day of zero, not of its two busy days"
        );
        assert_eq!(
            p.times_median, None,
            "a zero median yields no multiple rather than an infinity"
        );
        assert_eq!(p.total, 32);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A steady agent with one bad day gets a multiple that means something.
    #[test]
    fn a_bad_day_against_a_steady_habit_is_a_multiple() {
        let today = 20_000_i64;
        let mut seq = 0;
        let mut events = Vec::new();
        // Twenty days of exactly 2, then 10 on the last complete day.
        for d in 2..=21 {
            events.extend(day_events(today - d, "policy_deny", 2, &mut seq));
        }
        events.extend(day_events(today - 1, "policy_deny", 10, &mut seq));

        let (state, dir) = seeded("multiple", &events);
        let p = agent_profile(AGENT, 90, today * DAY_MS + 1000, &state).expect("profile");

        assert_eq!(p.median_day, 2.0, "its habit is two a day");
        assert_eq!(p.latest_full_day, 10);
        assert_eq!(p.times_median, Some(5.0), "five times its own median day");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An agent seen for three days has counts and no normal. Reporting a
    /// multiple off three days would be arithmetic wearing evidence's clothes.
    #[test]
    fn an_agent_too_new_reports_its_counts_and_refuses_the_comparison() {
        let today = 20_000_i64;
        let mut seq = 0;
        let mut events = Vec::new();
        for d in 0..3 {
            events.extend(day_events(today - d, "policy_deny", 5, &mut seq));
        }

        let (state, dir) = seeded("too-new", &events);
        let p = agent_profile(AGENT, 90, today * DAY_MS + 1000, &state).expect("profile");

        assert_eq!(p.confidence, Confidence::TooNew);
        assert_eq!(
            p.days_held, 3,
            "and it says HOW new, which is the useful half"
        );
        assert_eq!(p.total, 15, "the counts are real either way");
        assert_eq!(p.times_median, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Today is partial for all but its last instant, so comparing it against
    /// full days would report every agent as quiet every morning.
    #[test]
    fn the_partial_day_in_progress_is_not_the_day_being_judged() {
        let today = 20_000_i64;
        let mut seq = 0;
        let mut events = Vec::new();
        for d in 1..=20 {
            events.extend(day_events(today - d, "policy_deny", 4, &mut seq));
        }
        // One event so far today: a quiet morning, not a quiet agent.
        events.extend(day_events(today, "policy_deny", 1, &mut seq));

        let (state, dir) = seeded("partial", &events);
        let p = agent_profile(AGENT, 90, today * DAY_MS + 1000, &state).expect("profile");

        assert_eq!(
            p.latest_full_day, 4,
            "yesterday, not the hour of today that has happened"
        );
        assert_eq!(*p.daily.last().unwrap(), 1, "today is still in the series");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The three descriptors that turn a count into a sentence.
    #[test]
    fn it_describes_the_shape_of_the_series_not_only_its_size() {
        let today = 20_000_i64;
        let mut seq = 0;
        let mut events = Vec::new();
        // Quiet fortnight, then a busy week: rising.
        for d in 8..=21 {
            events.extend(day_events(today - d, "policy_deny", 1, &mut seq));
        }
        for d in 1..=7 {
            events.extend(day_events(today - d, "policy_deny", 6, &mut seq));
        }

        let (state, dir) = seeded("shape", &events);
        let p = agent_profile(AGENT, 90, today * DAY_MS + 1000, &state).expect("profile");

        assert_eq!(p.direction, Direction::Rising);
        assert_eq!(
            p.top_type.as_deref(),
            Some("policy_deny"),
            "one type over and over, which is a different situation from a mix"
        );
        assert_eq!(p.top_type_share, 1.0);
        assert!(
            p.busiest_day_share < 0.2,
            "spread across a week, not dumped in one day: got {}",
            p.busiest_day_share
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Same total, one day. The count cannot tell these apart and the share
    /// can, which is the reason the share exists.
    #[test]
    fn the_same_total_in_one_day_reads_as_concentrated() {
        let today = 20_000_i64;
        let mut seq = 0;
        let mut events = day_events(today - 1, "breaker_tripped", 42, &mut seq);
        for d in 2..=21 {
            events.extend(day_events(today - d, "policy_deny", 0, &mut seq));
        }

        let (state, dir) = seeded("concentrated", &events);
        let p = agent_profile(AGENT, 90, today * DAY_MS + 1000, &state).expect("profile");

        assert_eq!(p.total, 42);
        assert_eq!(
            p.busiest_day_share, 1.0,
            "all of it on one day, which a total of 42 alone never says"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An agent the store has never seen says so, rather than returning a
    /// profile of zeros that reads as a well-behaved agent.
    #[test]
    fn an_unseen_agent_is_no_data_not_a_calm_profile() {
        let (state, dir) = seeded("unseen", &[]);
        let p = agent_profile(AGENT, 90, 20_000 * DAY_MS, &state).expect("profile");
        assert_eq!(p.confidence, Confidence::NoData);
        assert_eq!(p.days_held, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_median_of_an_even_count_is_the_average_of_the_middle_two() {
        assert_eq!(median(&[1, 3]), 2.0);
        assert_eq!(median(&[1, 2, 3]), 2.0);
        assert_eq!(median(&[]), 0.0);
    }
}
