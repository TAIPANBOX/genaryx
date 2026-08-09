//! What agents did on the web, and what was refused before they could.
//!
//! # WHY THIS IS ITS OWN PANEL AND NOT A BUS EXPLORER FILTER
//!
//! Every `web_fetch` and `web_blocked` line already reaches the Bus Explorer,
//! because it reaches everything: the explorer is deliberately typeless and
//! shows an event's `data` as JSON. That is right for an investigation and
//! wrong for the question this panel answers, which is not "what happened at
//! 14:32" but "is the fidelity of what my agents read actually what I think it
//! is".
//!
//! Answering that means reading four fields together, per row: which backend
//! answered, whether it enforced per request or only at the navigation, how
//! many subresources it was asked for, and how many it could not report at
//! all. A JSON blob shows all four and compares none.
//!
//! # THE ONE THING THIS MUST NOT DO
//!
//! It must not return an empty list when it could not look. `bus::recent_events`
//! falls back to `mock_events` when its store is unavailable, which is right
//! for an explorer that must render something; here it would be a panel saying
//! "no agent fetched anything" to an operator whose store failed to open. That
//! is the silent-zero failure this estate keeps finding, in the one place where
//! the number is the product.
//!
//! So the shape carries [`EgressPanel::measured`] and a note, and the frontend
//! is required to render the note rather than the zero.

use serde::Serialize;
use std::collections::BTreeMap;

use crate::bus::AppState;

/// The source these events are attributed to, per agent-passport SPEC 6.2.
const SCOPYX_SOURCE: &str = "scopyx";

/// The two event types the egress plane emits.
const TYPE_FETCH: &str = "web_fetch";
const TYPE_BLOCKED: &str = "web_blocked";

/// One line of the panel: a fetch that happened, or one that did not.
#[derive(Debug, Clone, Serialize)]
pub struct EgressRow {
    pub ts: String,
    pub agent_id: String,
    pub run_id: Option<String>,
    /// `"fetched"` or `"blocked"`. Derived from the event type rather than
    /// from a field, because the type is what the registry pins.
    pub outcome: &'static str,

    /// Scheme and host, which is all the event carries. The path and the query
    /// string are deliberately absent from the record: a URL is personal data,
    /// and the plane that wrote this never assembled them into the event. The
    /// console cannot show what was never written, and should not imply it can.
    pub origin: String,
    /// The hash that lets two records be compared without either holding the
    /// address.
    pub url_sha384: Option<String>,

    // Present on a fetch.
    pub backend: Option<String>,
    pub enforcement: Option<String>,
    pub content_bytes: Option<i64>,

    // Present on a refusal.
    pub verdict: Option<String>,
    pub reason: Option<String>,
}

/// The counts an operator reads before any individual row.
#[derive(Debug, Clone, Serialize, Default)]
pub struct EgressTotals {
    pub fetched: usize,
    pub blocked: usize,

    /// Refusals by verdict. A map rather than a few named fields, because the
    /// verdict vocabulary belongs to the egress plane and a console that
    /// enumerated it here would be a fifth copy of somebody else's list, going
    /// stale the first time they add one.
    pub by_verdict: BTreeMap<String, usize>,

    /// How many fetches were served by a backend that enforces only at the
    /// navigation.
    ///
    /// The most important number on the panel and the least obvious. A fetch
    /// through a backend the operator already runs is decided before it is
    /// made, and then that service loads the page's images, fonts and scripts
    /// with nothing in between. The navigation was governed; the forty requests
    /// it caused were not. An operator who believes every request is policed
    /// needs to see this figure, and it is invisible in any per-row view.
    pub navigation_only: usize,

    /// Fetches whose backend could not say what the page asked for at all.
    ///
    /// Distinct from "asked for nothing", which is what the passthrough backend
    /// honestly reports. Zero and unknown are different facts and this panel
    /// keeps them apart, because collapsing them is how a partial answer starts
    /// reading as a complete one.
    pub subresources_unknown: usize,
}

/// What the panel renders.
#[derive(Debug, Clone, Serialize)]
pub struct EgressPanel {
    /// False when nothing could be read. The frontend must show `note` in that
    /// case and must NOT render the empty list as an answer.
    pub measured: bool,
    pub note: Option<String>,
    pub totals: EgressTotals,
    pub rows: Vec<EgressRow>,
}

impl EgressPanel {
    fn unmeasured(note: impl Into<String>) -> Self {
        Self {
            measured: false,
            note: Some(note.into()),
            totals: EgressTotals::default(),
            rows: Vec::new(),
        }
    }
}

/// Recent egress activity, newest first, capped at `limit`.
///
/// `limit` bounds the rows READ, not the rows returned, and the difference is
/// deliberate: this reads the recent slice of the whole bus and keeps the
/// scopyx lines out of it. A store with a busy money plane and one fetch an
/// hour would otherwise show nothing at all, and the panel would be wrong in
/// the least visible way. The count read is stated in the note so a reader
/// knows what window they are looking at.
pub fn egress_recent(limit: usize, state: &AppState) -> EgressPanel {
    let Some(dir) = &state.events_dir else {
        return EgressPanel::unmeasured(
            "The console has no event store on this box, so nothing here was read. \
             This is not a report that your agents made no web requests.",
        );
    };

    let db_path = dir.join("console.sqlite");
    let store = match genaryx_core::store::Store::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            return EgressPanel::unmeasured(format!(
                "The event store could not be opened ({e}), so nothing here was read. \
                 This is not a report that your agents made no web requests."
            ));
        }
    };

    // Read wide, keep narrow. See the doc comment above.
    let scan = limit.saturating_mul(20).clamp(limit, 20_000);
    let rows = match store.recent_events(scan) {
        Ok(r) => r,
        Err(e) => {
            return EgressPanel::unmeasured(format!(
                "The event store could not be queried ({e}), so nothing here was read. \
                 This is not a report that your agents made no web requests."
            ));
        }
    };

    let scanned = rows.len();
    let mut out = Vec::new();
    let mut totals = EgressTotals::default();

    for e in rows {
        if e.source != SCOPYX_SOURCE {
            continue;
        }
        let is_fetch = match e.type_.as_str() {
            TYPE_FETCH => true,
            TYPE_BLOCKED => false,
            // A type from this source that this build does not know is skipped
            // rather than guessed at. It still shows in the Bus Explorer, which
            // is where an unknown thing belongs.
            _ => continue,
        };
        let d = e.data.unwrap_or_default();

        let enforcement = d
            .get("enforcement")
            .and_then(|v| v.as_str())
            .map(String::from);
        if is_fetch {
            totals.fetched += 1;
            if enforcement.as_deref() == Some("navigation_only") {
                totals.navigation_only += 1;
            }
            // `null` and absent both mean "the backend could not say". An
            // absent key is the passthrough case only when the writer chose to
            // omit it; treating both as unknown is the safe direction, because
            // over-reporting unknown costs a reader a second look and
            // under-reporting it costs them a wrong conclusion.
            match d.get("subresources_requested") {
                Some(v) if !v.is_null() => {}
                _ => totals.subresources_unknown += 1,
            }
        } else {
            totals.blocked += 1;
            let verdict = d
                .get("verdict")
                .and_then(|v| v.as_str())
                .unwrap_or("unrecorded")
                .to_string();
            *totals.by_verdict.entry(verdict).or_insert(0) += 1;
        }

        out.push(EgressRow {
            ts: e.ts,
            agent_id: e.agent_id,
            run_id: e.run_id,
            outcome: if is_fetch { "fetched" } else { "blocked" },
            origin: d
                .get("origin")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            url_sha384: d
                .get("url_sha384")
                .and_then(|v| v.as_str())
                .map(String::from),
            backend: d.get("backend").and_then(|v| v.as_str()).map(String::from),
            enforcement,
            content_bytes: d.get("content_bytes").and_then(|v| v.as_i64()),
            verdict: d.get("verdict").and_then(|v| v.as_str()).map(String::from),
            reason: d.get("reason").and_then(|v| v.as_str()).map(String::from),
        });

        if out.len() >= limit {
            break;
        }
    }

    EgressPanel {
        measured: true,
        note: Some(format!(
            "Read from the {scanned} most recent events on the bus. \
             An older fetch than that is in the Bus Explorer, not here."
        )),
        totals,
        rows: out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::BusMode;

    fn empty_state() -> AppState {
        AppState {
            events_dir: None,
            source_events_dir: None,
            mode: BusMode::Unavailable {
                reason: "test".into(),
            },
        }
    }

    /// The property this module exists to have. A panel that reports zero
    /// fetches when it could not look tells an operator their agents behaved,
    /// which is the one wrong answer that reads as good news.
    #[test]
    fn with_no_store_it_says_it_could_not_look_rather_than_reporting_zero() {
        let p = egress_recent(50, &empty_state());
        assert!(!p.measured, "an unread panel must not claim to be measured");
        let note = p.note.expect("an unmeasured panel must say why");
        assert!(
            note.contains("not a report that your agents made no web requests"),
            "the note must refuse the wrong reading explicitly, got: {note}"
        );
        assert_eq!(p.totals.fetched, 0);
        assert_eq!(p.totals.blocked, 0);
    }
}
