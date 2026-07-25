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

/// How the Bus Explorer's stream is being fed, for the UI to say so honestly.
pub fn bus_status(state: &AppState) -> BusMode {
    state.mode.clone()
}
