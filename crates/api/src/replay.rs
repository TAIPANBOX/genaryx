//! Tauri command for Run Replay (docs/PHASE3.md W4, position 5):
//! [`run_events`] returns one run's events, OLDEST-first, ready for a
//! playback clock to scrub through client-side. Mirrors
//! `graph::agent_events` exactly: same `AppState.events_dir` ->
//! `console.sqlite` -> `genaryx_core::store::Store::open` flow, same
//! fail-closed contract: a missing/failed Store is never a panic and never
//! an `Err` the frontend traps on, it is a clean empty result the UI
//! renders as "no events for this run" (there is no plausible mock replay
//! timeline the way `events::mock_events` mocks the Bus Explorer, so
//! "empty" IS the honest fallback here too, exactly as `graph.rs`'s own doc
//! comment argues for the delegation graph).
//!
//! Position 5 in full: "Run Replay = a time-window query over the SQLite
//! Store plus a playback clock (scrub/speed, the mental model of the site
//! sims)". The query is this one command, over the already-shipped
//! `Store::events_for_run` (W4-core, commit `c7615e1`). The playback clock
//! itself (play/pause, scrub position, speed) is pure frontend state
//! (`src/lib/useReplayClock.ts`) over this one fetched, static list: the
//! Store read is a plain point-in-time query, not a subscription, so there
//! is nothing further for the Rust side to do once the list is fetched.
//! `taipan up` Cloud's `/v1/replay/{run}` is a documented optional second
//! source (PHASE3.md position 5); it is not added here since no such
//! connector method exists yet on `CloudClient`. `events_for_run` alone is
//! the primary source and is sufficient on its own (per this wave's brief).

use crate::bus::AppState;
use crate::events::UiEvent;
use std::path::Path;

// ============================================================================
// pure logic (directly unit-testable without a shell wrapper -
// same rationale as `graph::build_agent_events`)
// ============================================================================

/// Open the live Store the same way `graph::open_store`/`lib.rs`'s
/// `recent_events` does, or `None` on any failure - logged, never panicked,
/// never surfaced as an `Err`. Deliberately a private duplicate of
/// `graph::open_store` rather than a shared helper: `graph.rs`'s own doc
/// comment already establishes the convention that every command module
/// keeps this tiny open-or-log-and-None helper local to itself, so a reader
/// never has to cross a module boundary to see a command's whole fail-closed
/// path.
fn open_store(events_dir: Option<&Path>) -> Option<genaryx_core::store::Store> {
    let dir = events_dir?;
    let db_path = dir.join("console.sqlite");
    match genaryx_core::store::Store::open(&db_path) {
        Ok(store) => Some(store),
        Err(e) => {
            eprintln!("genaryx: could not open store for run replay: {e}");
            None
        }
    }
}

/// [`run_events`]'s pure logic: one run's events, OLDEST-first (the order a
/// replay plays forward through - chronological), capped at `limit`. A
/// missing/failed Store, or a `run_id` never seen on the bus, both yield a
/// clean empty `Vec`, never an error.
fn build_run_events(events_dir: Option<&Path>, run_id: &str, limit: usize) -> Vec<UiEvent> {
    let Some(store) = open_store(events_dir) else {
        return Vec::new();
    };
    match store.events_for_run(run_id, limit) {
        Ok(rows) => rows.into_iter().map(UiEvent::from).collect(),
        Err(e) => {
            eprintln!("genaryx: run_events query failed for {run_id:?}: {e}");
            Vec::new()
        }
    }
}

// ============================================================================
// command
// ============================================================================

/// One run's events, oldest-first, capped at `limit` - the Run Replay
/// timeline (docs/PHASE3.md W4, position 5). A run never seen on the bus (or
/// a missing/failed Store) yields a clean empty `Vec`, never an error - the
/// frontend renders that as "no events for this run", not a fault. `pub`
/// (mirrors `graph::agent_graph` etc.): `lib.rs` declares `mod replay;` and
/// names this command as `replay::run_events` in `generate_handler!`.
pub fn run_events(run_id: String, limit: usize, state: &AppState) -> Vec<UiEvent> {
    build_run_events(state.events_dir.as_deref(), &run_id, limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use genaryx_core::demo;
    use genaryx_core::ingest::IngestService;
    use genaryx_core::store::Store;
    use std::path::PathBuf;

    // ------------------------------------------------------------------
    // empty / absent store: fail closed to an honest empty result, never a
    // panic, never fabricated data - mirrors `graph.rs`'s identical tests.
    // ------------------------------------------------------------------

    #[test]
    fn run_events_with_no_events_dir_is_empty() {
        let events = build_run_events(None, "demo-run-000", 100);
        assert!(events.is_empty());
    }

    #[test]
    fn run_events_with_a_missing_store_file_is_empty() {
        // A real directory (so `Path::join` is meaningful) that has never had
        // `console.sqlite` seeded into it - `Store::open` still succeeds
        // (rusqlite creates the file), so this also exercises the "opened
        // fine, but queried zero rows" path.
        let dir = std::env::temp_dir().join(format!(
            "genaryx-replay-test-empty-{}-{}",
            std::process::id(),
            nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let events = build_run_events(Some(&dir), "demo-run-000", 100);
        assert!(events.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // seeded store: mirrors `graph.rs`'s `seed_demo_dir` exactly (demo::generate
    // -> Store -> IngestService -> poll_once), then exercises `run_events`'s
    // pure logic against the resulting real bus.
    // ------------------------------------------------------------------

    fn nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }

    fn seed_demo_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "genaryx-replay-test-seeded-{}-{}",
            std::process::id(),
            nanos()
        ));
        demo::generate(&dir).expect("demo::generate");

        let db_path = dir.join("console.sqlite");
        let store = Store::open(&db_path).expect("Store::open");
        let mut ingest = IngestService::new(store, "local").expect("IngestService::new");

        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("read_dir")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("ndjson"))
            .collect();
        files.sort();
        for path in &files {
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            ingest
                .add_file_source(format!("filetail:{id}"), path)
                .expect("add_file_source");
        }
        ingest.poll_once().expect("poll_once");
        dir
    }

    /// The demo generator's first run id (`crates/core/src/demo.rs`:
    /// `run_id = format!("demo-run-{i:03}")` for `i in 0..RUN_COUNT`,
    /// 0-based, so the very first run is `demo-run-000`, NOT `-001`). Run 0
    /// is a "block run" (`i < BLOCK_RUN_COUNT`): exactly 3 calls (wardryx
    /// `policy_allow`, tokenfuse `budget_exhausted` since `i` is even,
    /// engram `memory_written`), all attributed to `AGENTS[0]`
    /// ("tier1-bot") and all carrying the demo delegation chain
    /// (`i.is_multiple_of(4)` and the agent is not the orchestrator),
    /// grounded in that module's source, not guessed, the same discipline
    /// `graph.rs`'s `DEMO_USER`/`DEMO_ORCHESTRATOR` constants document.
    const DEMO_RUN: &str = "demo-run-000";
    const DEMO_RUN_AGENT: &str = "agent://taipanbox.dev/demo/tier1-bot";
    const DEMO_RUN_EVENT_COUNT: usize = 3;

    #[test]
    fn run_events_over_seeded_demo_data_is_oldest_first_and_scoped_to_one_run() {
        let dir = seed_demo_dir();
        let events = build_run_events(Some(&dir), DEMO_RUN, 1000);

        assert_eq!(
            events.len(),
            DEMO_RUN_EVENT_COUNT,
            "demo-run-000 is a fixed 3-call block run (policy_allow + budget_exhausted + memory_written)"
        );
        for e in &events {
            assert_eq!(
                e.run_id.as_deref(),
                Some(DEMO_RUN),
                "run_events must not leak other runs' rows"
            );
            assert_eq!(
                e.agent_id, DEMO_RUN_AGENT,
                "every event of one run shares that run's acting agent"
            );
        }
        // Oldest-first, the reverse of `agent_events`/`recent_events` - see
        // `Store::events_for_run`'s own doc comment.
        for pair in events.windows(2) {
            assert!(
                pair[0].id <= pair[1].id,
                "events must be oldest-first by id"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_events_honors_limit_and_unknown_run_ids() {
        let dir = seed_demo_dir();

        // A `limit` of 0 must not error, just yield nothing.
        let none = build_run_events(Some(&dir), DEMO_RUN, 0);
        assert!(none.is_empty());

        // A limit smaller than the run's real event count truncates rather
        // than erroring.
        let capped = build_run_events(Some(&dir), DEMO_RUN, 1);
        assert_eq!(capped.len(), 1);

        // A run id never seen on the bus is an honest empty result, not an
        // error - the same "unknown agent -> empty, not a fault" contract
        // `graph.rs`'s `agent_events` tests assert for agents.
        let nobody = build_run_events(Some(&dir), "demo-run-999999", 100);
        assert!(nobody.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
