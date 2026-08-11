//! Where the console's bus lives and how it is being fed.
//!
//! Plain data, deliberately shell-free: the web shell holds it in its
//! `Arc<AppCtx>`, the same way the removed desktop shell used to manage it as
//! Tauri state, but neither shape belongs to either shell, and a command
//! reading it cannot tell them apart.

use std::path::PathBuf;

/// The generic feeder that fills the bus and keeps it updated: the tailer,
/// the demo feeder, the live-vs-demo decision, and the [`feed::EventSink`]
/// trait each shell implements to receive what it forwards. See its own
/// module doc for the full story.
pub mod feed;

/// Where the Bus Explorer's stream comes from, and therefore what the UI must
/// say about it. Serialized to the frontend by the `bus_status` command.
///
/// This is deliberately a first-class value rather than a boolean: "demo"
/// and "the bus could not be opened at all" are different states with
/// different meanings, and collapsing them is how a broken console ends up
/// looking like a working one.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BusMode {
    /// Tailing a real environment's `events.dir`.
    Live { env: String, dir: String },
    /// No environment on this machine: generated fixtures plus a synthetic
    /// feeder. Everything shown under this mode is invented.
    Demo { dir: String },
    /// Startup failed outright; `recent_events` serves `events::mock_events`.
    Unavailable { reason: String },
}

/// Where the console's store lives (if startup
/// succeeded) and how it is being fed. `events_dir: None` means
/// the events reader falls back to `events::mock_events` (fail-closed:
/// a startup failure degrades the Bus Explorer; it never crashes the app or
/// traps the UI).
pub struct AppState {
    /// The console's own store directory (fresh per launch).
    pub events_dir: Option<PathBuf>,
    /// Where the products write their NDJSON: the directory this bus tails,
    /// and the only correct destination for a `console_command` line. Writing
    /// one into `events_dir` above puts it where nothing tails and nothing
    /// keeps it.
    pub source_events_dir: Option<PathBuf>,
    pub mode: BusMode,
}

/// Recent events for the Bus Explorer, newest first, capped at `limit`.
///
/// Reads the real `genaryx-core` `Store` seeded at startup (see
/// [`feed::bootstrap`]) through its own short-lived reader connection (WAL
/// mode lets this coexist with the live feeder's writer thread). Never panics
/// and never surfaces an `Err` to the frontend: a missing store (startup
/// seeding failed) or a failed query both fall back to
/// [`crate::events::mock_events`], so the Bus Explorer always renders
/// something rather than trapping on a broken bus.
pub fn recent_events(limit: usize, state: &AppState) -> Vec<crate::events::UiEvent> {
    if let Some(dir) = &state.events_dir {
        let db_path = dir.join("console.sqlite");
        match genaryx_core::store::Store::open(&db_path) {
            Ok(store) => match store.recent_events(limit) {
                Ok(rows) => {
                    return rows.into_iter().map(crate::events::UiEvent::from).collect();
                }
                Err(e) => {
                    eprintln!("genaryx: recent_events query failed, falling back to mock data: {e}")
                }
            },
            Err(e) => eprintln!(
                "genaryx: could not open store for recent_events, falling back to mock data: {e}"
            ),
        }
    }
    crate::events::mock_events(limit)
}

/// How many refused lines to describe. One row per distinct reason, and a
/// producer with a broken envelope emits one reason on every line, so a dozen
/// covers a badly misconfigured estate with room to spare.
const QUARANTINE_REASONS: usize = 12;

/// What the bus REFUSED, and why.
///
/// # THE GAP THIS CLOSES
///
/// Lines that fail conformance have always been kept, with their file, offset,
/// raw bytes and the validator's own reason. Nothing ever read them back. The
/// only report was one `eprintln!` at startup, so a producer that began
/// emitting a broken envelope after boot was invisible for as long as the
/// console stayed up.
///
/// That is worse than it sounds, because the console does not go blank when it
/// happens. It shows the rest of the bus, correctly, and the broken producer's
/// agents simply look quiet. `aws-comparable-176` is the real instance: twelve
/// events, every one of them refused for `agent_id: "aws-comparable-agent"`
/// with no `agent://` prefix, and the console's honest answer for that agent
/// was nothing at all.
///
/// # WHY IT IS NOT AN ERROR STATE
///
/// A refused line is the bus working. The alternative is accepting whatever
/// arrives, and then the envelope means nothing. So this reports a fault in a
/// PRODUCER, names it, and points at the file and offset to fix it at the
/// source. It never repairs a line: rewriting an operator's data to make it fit
/// is the one thing this console must not do.
#[derive(Debug, Clone, serde::Serialize)]
pub struct QuarantinePanel {
    /// False when the store could not be read. The frontend must show `note`
    /// rather than rendering "nothing was refused", which is the same wrong
    /// answer that reads as good news `crate::stats` is built against.
    pub measured: bool,
    pub note: Option<String>,
    /// Every refused line this box still holds, across all reasons.
    pub total: u64,
    /// One row per reason, worst first, capped at [`QUARANTINE_REASONS`].
    pub reasons: Vec<genaryx_core::store::QuarantineReason>,
}

impl QuarantinePanel {
    fn unmeasured(note: impl Into<String>) -> Self {
        Self {
            measured: false,
            note: Some(note.into()),
            total: 0,
            reasons: Vec::new(),
        }
    }
}

/// What this bus refused and why. See [`QuarantinePanel`].
pub fn quarantine(state: &AppState) -> QuarantinePanel {
    let Some(dir) = &state.events_dir else {
        return QuarantinePanel::unmeasured(
            "The console has no event store on this box, so nothing could be checked. \
             This is not a report that every line your producers sent was accepted.",
        );
    };
    let store = match genaryx_core::store::Store::open(&dir.join("console.sqlite")) {
        Ok(s) => s,
        Err(e) => {
            return QuarantinePanel::unmeasured(format!(
                "The event store could not be opened ({e}), so nothing could be checked. \
                 This is not a report that every line your producers sent was accepted."
            ));
        }
    };

    let total = match store.quarantine_count() {
        Ok(n) => n,
        Err(e) => {
            return QuarantinePanel::unmeasured(format!(
                "The event store could not be queried ({e}), so nothing could be checked. \
                 This is not a report that every line your producers sent was accepted."
            ));
        }
    };
    let reasons = store
        .quarantine_by_reason(QUARANTINE_REASONS)
        .unwrap_or_default();
    let shown: u64 = reasons.iter().map(|r| r.count).sum();

    let note = if total == 0 {
        Some(
            "Every line this bus has read conformed to the envelope. A producer that starts \
             emitting a broken one will appear here, and its agents would otherwise just look \
             quiet."
                .to_string(),
        )
    } else {
        let mut n = format!(
            "{total} line(s) were refused by the envelope and are NOT on the bus. \
             The agents they were about will look quieter than they were. \
             Fix the producer at the file and offset below; nothing here rewrites a line to \
             make it fit."
        );
        if shown < total {
            n.push_str(&format!(
                " Showing the {QUARANTINE_REASONS} most common reasons, covering {shown} of them."
            ));
        }
        Some(n)
    };

    QuarantinePanel {
        measured: true,
        note,
        total,
        reasons,
    }
}

/// How the Bus Explorer's stream is being fed, for the UI to say so honestly.
pub fn bus_status(state: &AppState) -> BusMode {
    state.mode.clone()
}
