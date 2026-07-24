//! Console commands for the delegation graph + Agent 360 (docs/PHASE3.md W3):
//! [`agent_graph`] (the whole laid-out graph, Canvas2D-ready), [`agent_slice`]
//! (one agent's immediate delegation neighborhood), and [`agent_events`] (one
//! agent's recent bus events - the Agent 360 card's events slice, and, once
//! filtered client-side to `source == "wardryx"`, its policy slice too). All
//! three mirror `lib.rs`'s `recent_events` command exactly: same
//! `AppState.events_dir` -> `console.sqlite` -> `genaryx_core::store::Store::open`
//! flow, same fail-closed contract - a missing/failed Store is never a panic
//! and never an `Err` the frontend traps on, it is a clean empty result the
//! UI renders as "no delegation activity yet" (there is no plausible mock
//! delegation graph the way `events::mock_events` mocks the Bus Explorer, so
//! "empty" IS the honest fallback here, not a placeholder for one).
//!
//! Layout is computed once per call, in core (`genaryx_core::layout_view`),
//! never in a shell (PHASE3 §5.3, decided: "layout in core, dumb Canvas2D
//! renderers in the shells" - this avoided a WebGL-parity trap back when two
//! native shells existed, since WebGL existed in the Tauri webview but not
//! natively in SwiftUI; the choice still stands for the web shell today).
//! Every DTO here
//! (`LayoutView`, `AgentSlice`) is a genaryx-core type re-exported at the
//! crate root and already derives `Serialize`, so - like idryx's connector
//! DTOs in `identity::commands` - no UI-facing mirror struct is needed;
//! `graphTypes.ts` just names the same wire shape for the frontend.

use crate::bus::AppState;
use crate::events::UiEvent;
use genaryx_core::{AgentSlice, DelegationGraph, LayoutConfig, LayoutView, layout_view};
use std::path::Path;

// ============================================================================
// pure logic (directly unit-testable without a shell wrapper -
// same rationale as `identity::commands::status_dto` being its own free
// function)
// ============================================================================

/// Open the live Store the same way `lib.rs`'s `recent_events` does, or
/// `None` on any failure - logged, never panicked, never surfaced as an
/// `Err`. Shared by every command in this module so all three read paths
/// fail closed identically.
fn open_store(events_dir: Option<&Path>) -> Option<genaryx_core::store::Store> {
    let dir = events_dir?;
    let db_path = dir.join("console.sqlite");
    match genaryx_core::store::Store::open(&db_path) {
        Ok(store) => Some(store),
        Err(e) => {
            eprintln!("genaryx: could not open store for the delegation graph: {e}");
            None
        }
    }
}

/// Build the live [`DelegationGraph`] from the Store, or `None` on any
/// failure - same fail-closed contract as [`open_store`].
fn open_graph(events_dir: Option<&Path>) -> Option<DelegationGraph> {
    let store = open_store(events_dir)?;
    match DelegationGraph::from_store(&store) {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("genaryx: could not build the delegation graph from the store: {e}");
            None
        }
    }
}

/// [`agent_graph`]'s pure logic: the whole delegation graph, laid out and
/// ready for a Canvas2D renderer. A missing/failed Store yields a clean
/// empty [`LayoutView`] (`nodes: []`, `edges: []`), never an error.
fn build_agent_graph(events_dir: Option<&Path>) -> LayoutView {
    match open_graph(events_dir) {
        Some(g) => layout_view(&g.view(), &LayoutConfig::default()),
        None => LayoutView::default(),
    }
}

/// [`agent_slice`]'s pure logic: one agent's immediate delegation
/// neighborhood. An agent never seen on the bus (or a missing/failed Store)
/// yields an all-empty [`AgentSlice`] (`node: None`), never an error.
fn build_agent_slice(events_dir: Option<&Path>, agent_id: &str) -> AgentSlice {
    match open_graph(events_dir) {
        Some(g) => g.agent_slice(agent_id),
        None => AgentSlice::default(),
    }
}

/// [`agent_events`]'s pure logic: this agent's most recent `limit` events,
/// newest first. Mirrors `recent_events`'s exact Store-open + fail-closed-to-
/// empty contract (see `lib.rs`).
fn build_agent_events(events_dir: Option<&Path>, agent_id: &str, limit: usize) -> Vec<UiEvent> {
    let Some(store) = open_store(events_dir) else {
        return Vec::new();
    };
    match store.events_for_agent(agent_id, limit) {
        Ok(rows) => rows.into_iter().map(UiEvent::from).collect(),
        Err(e) => {
            eprintln!("genaryx: agent_events query failed for {agent_id:?}: {e}");
            Vec::new()
        }
    }
}

// ============================================================================
// commands
// ============================================================================

/// The whole delegation graph, laid out and ready for a Canvas2D renderer
/// (docs/PHASE3.md W3, position 3). A missing/failed Store yields a clean
/// empty [`LayoutView`], never an error - the frontend renders that as "no
/// delegation activity yet", not a fault. `pub` (mirrors
/// `identity::commands::identity_status` etc.): `lib.rs` declares `mod graph;`
/// and names this command as `graph::agent_graph` in `generate_handler!`, so
/// it must be visible from that parent module, not just from this one.
pub fn agent_graph(state: &AppState) -> LayoutView {
    build_agent_graph(state.events_dir.as_deref())
}

/// One agent's immediate delegation neighborhood (the Agent 360 card's
/// Delegation section, docs/PHASE3.md W3 position 4). An agent never seen on
/// the bus yields an all-empty [`AgentSlice`], never an error.
pub fn agent_slice(agent_id: String, state: &AppState) -> AgentSlice {
    build_agent_slice(state.events_dir.as_deref(), &agent_id)
}

/// This agent's most recent `limit` events, newest first - the Agent 360
/// card's Events section (docs/PHASE3.md W3 position 4), and (filtered
/// client-side to `source == "wardryx"`) its Policy section. Mirrors
/// `recent_events`'s exact Store-open + fail-closed-to-empty contract.
pub fn agent_events(agent_id: String, limit: usize, state: &AppState) -> Vec<UiEvent> {
    build_agent_events(state.events_dir.as_deref(), &agent_id, limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use genaryx_core::demo;
    use genaryx_core::ingest::IngestService;
    use genaryx_core::store::Store;
    use std::path::PathBuf;

    // ------------------------------------------------------------------
    // empty / absent store: every command must fail closed to an honest
    // empty result, never a panic, never fabricated data.
    // ------------------------------------------------------------------

    #[test]
    fn agent_graph_with_no_events_dir_is_empty() {
        let lv = build_agent_graph(None);
        assert!(lv.nodes.is_empty());
        assert!(lv.edges.is_empty());
    }

    #[test]
    fn agent_graph_with_a_missing_store_file_is_empty() {
        // A real directory (so `Path::join` is meaningful) that has never had
        // `console.sqlite` seeded into it - `Store::open` still succeeds
        // (rusqlite creates the file), so this also exercises the "opened
        // fine, but the graph built from zero rows" path.
        let dir = std::env::temp_dir().join(format!(
            "genaryx-graph-test-empty-{}-{}",
            std::process::id(),
            nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let lv = build_agent_graph(Some(&dir));
        assert!(lv.nodes.is_empty());
        assert!(lv.edges.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn agent_slice_with_no_events_dir_is_all_empty() {
        let slice = build_agent_slice(None, "agent://acme/anyone");
        assert!(slice.node.is_none());
        assert!(slice.parents.is_empty());
        assert!(slice.children.is_empty());
    }

    #[test]
    fn agent_events_with_no_events_dir_is_empty() {
        let events = build_agent_events(None, "agent://acme/anyone", 50);
        assert!(events.is_empty());
    }

    // ------------------------------------------------------------------
    // seeded store: mirrors `live.rs`'s own
    // `seeds_the_store_from_demo_fixtures` test (demo::generate -> Store ->
    // IngestService -> poll_once), then exercises all three commands' pure
    // logic against the resulting real delegation graph.
    // ------------------------------------------------------------------

    fn nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }

    /// Seed a fresh temp events dir from the demo fixtures, exactly like
    /// `live::bootstrap` does at startup. Returns the dir (caller's
    /// responsibility to best-effort clean up).
    fn seed_demo_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "genaryx-graph-test-seeded-{}-{}",
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

    /// The demo generator's fixed 2-element chain (`crates/core/src/demo.rs`,
    /// `DELEGATION_CHAIN`): every 4th run (bar the orchestrator's own runs)
    /// is attributed `[user://taipanbox.dev/j.doe, agent://taipanbox.dev/demo/orchestrator]`,
    /// root-first. Duplicated here as a literal (not imported: `demo`'s own
    /// constant is private) - the same "grounded in the generator's actual
    /// behavior, not a guess" rationale `live.rs`'s test uses for asserting
    /// exact file counts.
    const DEMO_USER: &str = "user://taipanbox.dev/j.doe";
    const DEMO_ORCHESTRATOR: &str = "agent://taipanbox.dev/demo/orchestrator";

    #[test]
    fn agent_graph_over_seeded_demo_data_has_nodes_and_edges_in_bounds() {
        let dir = seed_demo_dir();
        let lv = build_agent_graph(Some(&dir));

        assert!(
            !lv.nodes.is_empty(),
            "demo data must produce a non-empty graph"
        );
        assert!(
            !lv.edges.is_empty(),
            "the demo delegation chain must produce edges"
        );
        for n in &lv.nodes {
            assert!(
                n.x.is_finite() && n.y.is_finite(),
                "{} has a non-finite position",
                n.id
            );
            assert!((0.0..=lv.width).contains(&n.x), "{} x out of bounds", n.id);
            assert!((0.0..=lv.height).contains(&n.y), "{} y out of bounds", n.id);
        }
        // The demo's root human principal and its orchestrator hop must both
        // be present as nodes (grounded in `demo::DELEGATION_CHAIN`).
        assert!(lv.nodes.iter().any(|n| n.id == DEMO_USER));
        assert!(lv.nodes.iter().any(|n| n.id == DEMO_ORCHESTRATOR));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn agent_slice_over_seeded_demo_data_finds_the_orchestrator_hop() {
        let dir = seed_demo_dir();
        let slice = build_agent_slice(Some(&dir), DEMO_ORCHESTRATOR);

        // The orchestrator itself acts directly in its own (non-delegated)
        // runs, so it must be a real, acted-on node, not just a chain link.
        let node = slice
            .node
            .expect("orchestrator must be a node in the demo graph");
        assert_eq!(node.id, DEMO_ORCHESTRATOR);
        assert!(node.event_count > 0);

        // Exactly one parent: the fixed 2-element chain's root user.
        assert_eq!(
            slice
                .parents
                .iter()
                .map(|n| n.id.as_str())
                .collect::<Vec<_>>(),
            [DEMO_USER]
        );
        // At least one delegatee: every 4th non-orchestrator run routes
        // through the orchestrator on its way to the acting agent.
        assert!(
            !slice.children.is_empty(),
            "orchestrator must have delegatees in demo data"
        );

        // An agent never seen on the bus still yields an honest empty slice.
        let unknown = build_agent_slice(Some(&dir), "agent://taipanbox.dev/demo/nobody-at-all");
        assert!(
            unknown.node.is_none() && unknown.parents.is_empty() && unknown.children.is_empty()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn agent_events_over_seeded_demo_data_is_scoped_to_one_agent() {
        let dir = seed_demo_dir();
        let events = build_agent_events(Some(&dir), DEMO_ORCHESTRATOR, 100);

        assert!(
            !events.is_empty(),
            "the orchestrator must have its own events in demo data"
        );
        for e in &events {
            assert_eq!(
                e.agent_id, DEMO_ORCHESTRATOR,
                "agent_events must not leak other agents' rows"
            );
        }
        // Newest-first, matching `Store::events_for_agent`'s own contract.
        for pair in events.windows(2) {
            assert!(
                pair[0].id >= pair[1].id,
                "events must be newest-first by id"
            );
        }

        // A `limit` of 0 must not error, an empty result must not error.
        let none = build_agent_events(Some(&dir), DEMO_ORCHESTRATOR, 0);
        assert!(none.is_empty());
        let nobody = build_agent_events(Some(&dir), "agent://taipanbox.dev/demo/nobody-at-all", 50);
        assert!(nobody.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
