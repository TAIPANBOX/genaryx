//! genaryx-ffi: the UniFFI boundary between `genaryx-core` and the SwiftUI
//! shell (Phase-0 spike 1, `docs/PHASE0.md`).
//!
//! Design, in one paragraph: proc-macro scaffolding (`setup_scaffolding!`, no
//! UDL file) exports three things. [`UiEvent`] is a flat Record mirroring the
//! UI-relevant fields of `genaryx_core::store::StoredEvent`. [`FleetHandle`]
//! is an Object whose constructor seeds a throwaway on-disk demo world (temp
//! dir, `demo::generate`, WAL Store, `IngestService` tailing the six demo
//! NDJSON files) and spawns two plain threads: an ingest thread that owns the
//! `Send`-but-not-`Sync` `IngestService` outright (no lock, single owner, per
//! its own docs) and a feeder thread that appends one conforming line per
//! second so the stream is visibly live. [`EventListener`] is a callback
//! interface; the ingest thread drains the core's broadcast receiver with the
//! synchronous `try_recv` after each poll and pushes each event to every
//! registered listener, so live push crosses the FFI without any async
//! runtime on either side.
//!
//! Fail-closed at the boundary (06 §0.5): nothing here panics across FFI.
//! Every exported call returns `Result<_, FfiError>` or is infallible by
//! construction; the background threads log-and-continue on per-cycle errors
//! rather than dying silently.
//!
//! Phase-1 wave 3 (docs/PHASE1.md) adds [`cloud::CloudHandle`]: a second
//! UniFFI Object over `genaryx_connectors::CloudClient`, for the SwiftUI
//! Money + Overview surface. See `cloud/mod.rs` for its own design docs
//! (the async-to-sync bridge differs from `FleetHandle`'s: one owned
//! `tokio::runtime::Runtime`, `block_on` per call, no background threads).
//!
//! Phase-2 wave 2 (docs/PHASE2.md) adds [`wardryx::WardryxHandle`]: a third
//! UniFFI Object, over `genaryx_connectors::WardryxClient`, for the SwiftUI
//! Policy surface (decision stream, approvals inbox, policy view). Same
//! owned-runtime async-to-sync bridge as `CloudHandle`, simpler in one
//! respect (bearer-only auth, no pairing/device/signer) - see
//! `wardryx/mod.rs` for its own design docs.
//!
//! Phase-3 wave 2 (docs/PHASE3.md) adds [`idryx::IdryxHandle`]: a fourth
//! UniFFI Object, over `genaryx_connectors::IdryxClient`, for the SwiftUI
//! Identity surface (identities list, the 21-detector alert stream, Rescan).
//! Same owned-runtime async-to-sync bridge again, simpler still than
//! `WardryxHandle`: no auth at all (not even a bearer), and no
//! `command::record` journal (Identity is read-only this wave) - see
//! `idryx/mod.rs` for its own design docs.
//!
//! Phase-3 wave 3 (docs/PHASE3.md) adds three more [`FleetHandle`] reads for
//! the delegation graph + Agent 360 + deep-links: [`FleetHandle::agent_graph`]
//! (the whole delegation graph, laid out for a Canvas2D renderer),
//! [`FleetHandle::agent_slice`] (one agent's immediate delegation
//! neighborhood), and [`FleetHandle::events_for_agent`] (one agent's own
//! event history, reusing [`UiEvent`]). No new Object: this is core's own
//! bus-fed graph (`genaryx_core::DelegationGraph`, built from the bus, NOT
//! from Idryx - PHASE3.md architecture position 1), so it belongs on the
//! same handle that already owns the bus-backed `reader: Mutex<Store>`, not
//! on `IdryxHandle`. [`LayoutViewRecord`], [`PositionedNodeRecord`],
//! [`GraphEdgeRecord`], [`AgentSliceRecord`], [`GraphNodeRecord`], and
//! [`NodeKind`] are hand-written UniFFI mirrors of `genaryx_core`'s
//! same-named (or, for `NodeKind`'s mirror, same-shaped) graph/layout types -
//! those derive `Serialize`/`Deserialize` for the Tauri shell's IPC but are
//! not themselves `uniffi::Record`/`uniffi::Enum`, exactly the reason
//! [`UiEvent`] already mirrors `StoredEvent` by hand.
//!
//! Phase-3 wave 4 (docs/PHASE3.md, position 5, "Run Replay") adds
//! [`FleetHandle::events_for_run`]: one more read on the same handle, one
//! more reuse of [`UiEvent`], mirroring [`FleetHandle::events_for_agent`]
//! exactly except for the `Store` index it filters through
//! (`events_for_run` instead of `events_for_agent`) and the resulting sort
//! direction (oldest-first, since a playback clock plays forward - see
//! `genaryx_core::store::Store::events_for_run`'s own doc comment). No new
//! Record, no new Object: a run's timeline is just another slice of the same
//! events table the graph and Agent 360 reads already slice.

pub mod cloud;
pub mod idryx;
pub mod wardryx;

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use chrono::{SecondsFormat, Utc};
use genaryx_core::store::{Store, StoredEvent};
use genaryx_core::{
    AgentSlice, ConsoleEvent, DelegationGraph, GraphEdge, GraphNode, IngestService, LayoutConfig,
    LayoutView, PositionedNode, demo, layout_view,
};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::TryRecvError;

uniffi::setup_scaffolding!();

/// Ingest poll cadence. 07 §3's default is 250ms; the spike ticks slightly
/// tighter so a pushed line reaches Swift well inside a second.
const POLL_INTERVAL: Duration = Duration::from_millis(150);

/// Feeder granularity: sleep this per tick and emit every
/// [`FEEDER_TICKS_PER_LINE`] ticks (~1s per line). Short ticks keep both the
/// stop latency and `Drop`'s join wait bounded by ~100ms, not a full second.
const FEEDER_TICK: Duration = Duration::from_millis(100);
const FEEDER_TICKS_PER_LINE: u32 = 10;

/// The six demo bus sources, exactly the files `demo::generate` writes
/// (`crates/core/src/demo.rs` `SOURCES`; that const is private to core, and
/// core is frozen for this spike, so the six names are mirrored here).
const DEMO_SOURCES: [&str; 6] = [
    "tokenfuse",
    "wardryx",
    "engram",
    "verdryx",
    "mockryx",
    "qryx",
];

/// Errors crossing the FFI boundary. Deliberately one flat, message-carrying
/// variant for the spike: Swift sees `FfiError.Core(msg:)` and can display or
/// log it; no caller on the shell side branches on finer kinds yet.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    #[error("core: {msg}")]
    Core { msg: String },
}

impl From<genaryx_core::Error> for FfiError {
    fn from(e: genaryx_core::Error) -> Self {
        Self::Core { msg: e.to_string() }
    }
}

impl From<std::io::Error> for FfiError {
    fn from(e: std::io::Error) -> Self {
        Self::Core { msg: e.to_string() }
    }
}

/// UI-facing mirror of the UI-relevant fields of [`StoredEvent`] (same subset
/// the hand-written `apps/macos` `UiEvent.swift` carries today; that file is
/// what this Record replaces in the follow-up wire-up). `data`, `env`,
/// `prev_hash`, `file`, and `off` are omitted until a view needs them; `raw`
/// already carries the full line verbatim.
///
/// Naming: the Rust field is `event_type` (not core's `type_`) because UniFFI
/// lower-camel-cases names for Swift, and `type_` would collide with Swift's
/// reserved `type`; `event_type` generates the unambiguous `eventType`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct UiEvent {
    /// `events.id` rowid for stored rows; 0 for live-pushed events, whose
    /// rowid is not carried on the broadcast path (`ConsoleEvent` predates
    /// insert). Shell view models should key rows on their own counter or on
    /// (`ts`, `agent_id`, `raw`), not on this field.
    pub id: i64,
    pub ts: String,
    pub source: String,
    pub event_type: String,
    pub agent_id: String,
    pub run_id: Option<String>,
    pub severity: Option<String>,
    pub schema: String,
    pub on_behalf_of: Vec<String>,
    pub raw: String,
}

impl From<StoredEvent> for UiEvent {
    fn from(e: StoredEvent) -> Self {
        Self {
            id: e.id,
            ts: e.ts,
            source: e.source,
            event_type: e.type_,
            agent_id: e.agent_id,
            run_id: e.run_id,
            severity: e.severity,
            schema: e.schema,
            on_behalf_of: e.on_behalf_of,
            raw: e.raw,
        }
    }
}

impl From<&ConsoleEvent> for UiEvent {
    fn from(ce: &ConsoleEvent) -> Self {
        let e = &ce.event;
        Self {
            id: 0, // see the field doc: rowid is unknown on the push path
            ts: e.ts.clone(),
            source: e.source.clone(),
            event_type: e.event_type.clone(),
            agent_id: e.agent_id.clone(),
            run_id: e.run_id.clone(),
            severity: e.severity.clone(),
            schema: e.schema.clone(),
            on_behalf_of: e.on_behalf_of.clone(),
            raw: ce.raw.clone(),
        }
    }
}

// ============================================================================
// Delegation graph + layout mirrors (PHASE3 W3). `genaryx_core::{NodeKind,
// GraphNode, GraphEdge, AgentSlice}` (`graph.rs`) and
// `genaryx_core::{PositionedNode, LayoutView}` (`layout.rs`) derive
// `Serialize`/`Deserialize` for the Tauri shell's IPC but are not themselves
// UniFFI types, so - exactly like `UiEvent` above - this crate defines its
// own mirrors and converts at the boundary. All conversions consume their
// input (never clone): every call site below already owns a freshly built
// `AgentSlice`/`LayoutView`, not a borrow.
// ============================================================================

/// Mirror of `genaryx_core::NodeKind`: what kind of principal a delegation
/// node is, inferred from its URI scheme. Swift sees `.user` / `.agent` /
/// `.other`. Left unsuffixed (unlike the `*Record` structs below), matching
/// this crate's existing enum convention (`wardryx::ApprovalVerdict`,
/// `idryx::IdryxEnvSource`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum NodeKind {
    /// `user://…` - a human principal (only ever a root, never an actor).
    User,
    /// `agent://…` - an agent principal.
    Agent,
    /// Anything else (kept, not dropped, so an unexpected scheme still renders).
    Other,
}

impl From<genaryx_core::NodeKind> for NodeKind {
    fn from(k: genaryx_core::NodeKind) -> Self {
        match k {
            genaryx_core::NodeKind::User => Self::User,
            genaryx_core::NodeKind::Agent => Self::Agent,
            genaryx_core::NodeKind::Other => Self::Other,
        }
    }
}

/// One delegation-graph node: mirrors [`GraphNode`], flattened for UniFFI.
#[derive(Debug, Clone, uniffi::Record)]
pub struct GraphNodeRecord {
    pub id: String,
    pub kind: NodeKind,
    /// Events where this node was the acting `agent_id`, deduped on the
    /// natural key. A pure delegator that never itself acted stays at 0.
    pub event_count: u64,
    /// The most recent `ts` this node was seen acting; `""` for a node only
    /// ever seen inside another's delegation chain.
    pub last_ts: String,
}

impl From<GraphNode> for GraphNodeRecord {
    fn from(n: GraphNode) -> Self {
        Self {
            id: n.id,
            kind: n.kind.into(),
            event_count: n.event_count,
            last_ts: n.last_ts,
        }
    }
}

/// One directed "delegates_to" edge: mirrors [`GraphEdge`].
#[derive(Debug, Clone, uniffi::Record)]
pub struct GraphEdgeRecord {
    pub from: String,
    pub to: String,
}

impl From<GraphEdge> for GraphEdgeRecord {
    fn from(e: GraphEdge) -> Self {
        Self {
            from: e.from,
            to: e.to,
        }
    }
}

/// One node with a computed layout position: mirrors [`PositionedNode`].
#[derive(Debug, Clone, uniffi::Record)]
pub struct PositionedNodeRecord {
    pub id: String,
    pub kind: NodeKind,
    pub event_count: u64,
    pub x: f64,
    pub y: f64,
}

impl From<PositionedNode> for PositionedNodeRecord {
    fn from(n: PositionedNode) -> Self {
        Self {
            id: n.id,
            kind: n.kind.into(),
            event_count: n.event_count,
            x: n.x,
            y: n.y,
        }
    }
}

/// The full laid-out delegation graph a shell's Canvas2D renderer consumes:
/// mirrors [`LayoutView`]. `width`/`height` are the canvas bounds every
/// `PositionedNodeRecord.x`/`.y` is clamped into (`LayoutConfig::default()`'s
/// 1000x1000 - see [`FleetHandle::agent_graph`]).
#[derive(Debug, Clone, uniffi::Record)]
pub struct LayoutViewRecord {
    pub nodes: Vec<PositionedNodeRecord>,
    pub edges: Vec<GraphEdgeRecord>,
    pub width: f64,
    pub height: f64,
}

impl From<LayoutView> for LayoutViewRecord {
    fn from(v: LayoutView) -> Self {
        Self {
            nodes: v
                .nodes
                .into_iter()
                .map(PositionedNodeRecord::from)
                .collect(),
            edges: v.edges.into_iter().map(GraphEdgeRecord::from).collect(),
            width: v.width,
            height: v.height,
        }
    }
}

/// One agent's immediate delegation neighborhood for its Agent 360 card's
/// Delegation section: mirrors [`AgentSlice`]. `node` is `None` for an agent
/// never seen on the bus - a normal, renderable "no delegation activity"
/// outcome, never an error (see [`FleetHandle::agent_slice`]).
#[derive(Debug, Clone, uniffi::Record)]
pub struct AgentSliceRecord {
    pub node: Option<GraphNodeRecord>,
    pub parents: Vec<GraphNodeRecord>,
    pub children: Vec<GraphNodeRecord>,
}

impl From<AgentSlice> for AgentSliceRecord {
    fn from(s: AgentSlice) -> Self {
        Self {
            node: s.node.map(GraphNodeRecord::from),
            parents: s.parents.into_iter().map(GraphNodeRecord::from).collect(),
            children: s.children.into_iter().map(GraphNodeRecord::from).collect(),
        }
    }
}

/// Live-push callback the shell implements (a Swift class conforming to the
/// generated `EventListener` protocol). Called from the Rust ingest thread,
/// never the main thread: the Swift side hops to `@MainActor` itself before
/// touching UI state.
#[uniffi::export(callback_interface)]
pub trait EventListener: Send + Sync {
    fn on_event(&self, event: UiEvent);
}

type Listeners = Arc<Mutex<Vec<Box<dyn EventListener>>>>;

/// Lock a poisoned-or-not mutex without panicking: a poisoned guard only
/// means some other thread died mid-hold, and every value guarded here stays
/// usable in that case. Never `unwrap` on the FFI-reachable path (06 §0.5).
fn relock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The spike's one exported Object: a self-contained live fleet view over a
/// throwaway demo world. Construction is synchronous and complete: when
/// `new()` returns, the demo campaign (~179 events) is already ingested and
/// queryable, and the live feeder is running.
#[derive(uniffi::Object)]
pub struct FleetHandle {
    /// Second WAL connection, reads only; the writer connection lives inside
    /// the ingest thread's `IngestService`. WAL is exactly the mode where one
    /// writer plus concurrent readers is the supported topology.
    reader: Mutex<Store>,
    listeners: Listeners,
    stop: Arc<AtomicBool>,
    threads: Mutex<Vec<JoinHandle<()>>>,
    /// Temp world root, removed on drop (best effort).
    dir: PathBuf,
}

#[uniffi::export]
impl FleetHandle {
    /// Build the demo world and start the ingest + feeder threads.
    ///
    /// Steps: temp dir; `demo::generate` writes the six NDJSON files; an
    /// `IngestService` (owning the writer Store) registers all six tails and
    /// runs one priming `poll_once` so the full campaign is stored before
    /// this returns; the primed broadcast backlog is drained so listeners
    /// registered later see only genuinely new events; then the two threads
    /// start and a reader Store opens on the same WAL file.
    #[uniffi::constructor]
    pub fn new() -> Result<Self, FfiError> {
        let dir = fresh_world_dir()?;
        let events_dir = dir.join("events");
        std::fs::create_dir_all(&events_dir)?;
        demo::generate(&events_dir)?;

        let db = dir.join("genaryx.sqlite3");
        let mut service = IngestService::new(Store::open(&db)?, "local")?;
        for source in DEMO_SOURCES {
            service.add_file_source(
                format!("filetail:{source}"),
                events_dir.join(format!("{source}.ndjson")),
            )?;
        }

        let mut rx = service.subscribe();
        service.poll_once()?;
        drain(&mut rx, &|_| {}); // primed history is not a live push

        let listeners: Listeners = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let ingest = spawn_ingest(service, rx, Arc::clone(&listeners), Arc::clone(&stop))?;
        let feeder = spawn_feeder(events_dir.join("tokenfuse.ndjson"), Arc::clone(&stop))?;

        Ok(Self {
            reader: Mutex::new(Store::open(&db)?),
            listeners,
            stop,
            threads: Mutex::new(vec![ingest, feeder]),
            dir,
        })
    }

    /// The most recent `limit` stored events, newest first (rowid order),
    /// via this handle's own read connection.
    pub fn recent_events(&self, limit: u32) -> Result<Vec<UiEvent>, FfiError> {
        let events = relock(&self.reader).recent_events(limit as usize)?;
        Ok(events.into_iter().map(UiEvent::from).collect())
    }

    /// Total events stored so far (demo campaign + everything the live
    /// feeder has appended and ingest has committed).
    pub fn event_count(&self) -> Result<u64, FfiError> {
        Ok(relock(&self.reader).event_count()?)
    }

    // ---- delegation graph + Agent 360 (PHASE3 W3) --------------------------
    // The live delegation graph is core's, built from the bus - NOT Idryx's
    // (PHASE3.md architecture position 1), so these read through the SAME
    // `reader` connection as `recent_events`/`event_count` above, never a
    // second handle. Each call rebuilds the graph fresh from the Store
    // (`DelegationGraph::from_store`): the demo/pilot event volume makes this
    // cheap, and it keeps these three reads trivially consistent with each
    // other and with whatever `recent_events` most recently showed, with no
    // separate live-updated graph state to keep in sync. Fail-closed
    // throughout: an absent/empty Store yields an empty result (0 nodes, a
    // `None` slice node), never a panic - `DelegationGraph::from_store` and
    // `agent_slice` are panic-free by construction (see `crates/core/src/
    // graph.rs`'s own module doc).

    /// The whole delegation graph, laid out for a Canvas2D renderer
    /// (PHASE3.md position 3: layout is computed once, here, in core; both
    /// shells only draw the result - never WebGL). The reader lock is
    /// released before the layout force-simulation runs (`drop(store)`
    /// below), so a slow layout over a large graph never blocks a concurrent
    /// `recent_events`/`event_count` call on the same connection.
    pub fn agent_graph(&self) -> Result<LayoutViewRecord, FfiError> {
        let store = relock(&self.reader);
        let graph = DelegationGraph::from_store(&store)?;
        drop(store);
        Ok(LayoutViewRecord::from(layout_view(
            &graph.view(),
            &LayoutConfig::default(),
        )))
    }

    /// One agent's immediate delegation neighborhood (Agent 360's
    /// Delegation section): the node itself, plus its direct parents
    /// (delegators) and children (delegatees). An agent never seen on the
    /// bus yields an all-empty slice (`node: None`), never an error - a
    /// normal "no delegation activity for this agent" outcome.
    pub fn agent_slice(&self, agent_id: String) -> Result<AgentSliceRecord, FfiError> {
        let store = relock(&self.reader);
        let graph = DelegationGraph::from_store(&store)?;
        drop(store);
        Ok(AgentSliceRecord::from(graph.agent_slice(&agent_id)))
    }

    /// This agent's most recent `limit` stored events, newest first (Agent
    /// 360's Events section), reusing [`UiEvent`] - the same Record
    /// [`FleetHandle::recent_events`] returns, just filtered server-side to
    /// one `agent_id` via `Store::events_for_agent`'s own index.
    pub fn events_for_agent(&self, agent_id: String, limit: u32) -> Result<Vec<UiEvent>, FfiError> {
        let events = relock(&self.reader).events_for_agent(&agent_id, limit as usize)?;
        Ok(events.into_iter().map(UiEvent::from).collect())
    }

    /// Every event of one run, OLDEST-first (Run Replay's timeline - PHASE3
    /// W4), reusing [`UiEvent`] exactly like
    /// [`FleetHandle::events_for_agent`] does, just filtered server-side to
    /// one `run_id` via `Store::events_for_run`'s own index. Oldest-first is
    /// the reverse of every other read on this handle
    /// (`recent_events`/`events_for_agent` are newest-first) because a
    /// playback clock plays a run forward in time, not backward - see
    /// `Store::events_for_run`'s own doc comment. A `run_id` this Store has
    /// never seen (e.g. one that only exists in a separate Cloud
    /// environment - PHASE3.md: "Cloud `/v1/replay/{run}` is a second
    /// source") yields a clean empty `Vec`, never an error: a normal,
    /// renderable "no events for this run" outcome for the Run Replay view's
    /// own honest empty state, exactly like `agent_slice`'s "unseen agent"
    /// contract above.
    ///
    /// NOTE for callers building a playback UI: this order is "oldest by
    /// insertion" (SQLite `id`), not a promise that `ts` is monotonic across
    /// SOURCES within the run - `IngestService::poll_once` batches one
    /// source's entire backlog before moving to the next
    /// (`crates/core/src/ingest.rs`), so a multi-source run's events land in
    /// the Store in source-registration order, not wall-clock order. A
    /// caller that wants a true wall-clock scrub should re-sort the returned
    /// `Vec` by `ts` itself (see `apps/macos`'s `RunReplayModel.swift`).
    pub fn events_for_run(&self, run_id: String, limit: u32) -> Result<Vec<UiEvent>, FfiError> {
        let events = relock(&self.reader).events_for_run(&run_id, limit as usize)?;
        Ok(events.into_iter().map(UiEvent::from).collect())
    }

    /// Register a live listener. Events ingested from this point on are
    /// pushed to it from the Rust ingest thread; events from before
    /// registration are not replayed (fetch history via
    /// [`FleetHandle::recent_events`]).
    pub fn subscribe(&self, listener: Box<dyn EventListener>) {
        relock(&self.listeners).push(listener);
    }
}

impl Drop for FleetHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let handles: Vec<JoinHandle<()>> = relock(&self.threads).drain(..).collect();
        for handle in handles {
            // Bounded wait: both loops re-check `stop` at least every 150ms.
            let _ = handle.join();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A unique, collision-proof temp directory for one handle's world:
/// pid + per-process counter + nanos.
fn fresh_world_dir() -> Result<PathBuf, FfiError> {
    static INSTANCE: AtomicU64 = AtomicU64::new(0);
    let n = INSTANCE.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("genaryx-ffi-{}-{n}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Drain everything currently buffered on the broadcast receiver, invoking
/// `each` per event. `Lagged` (receiver overrun) is survivable by design:
/// the skipped events are already in the Store, so history stays complete
/// even when a push is missed.
fn drain(rx: &mut broadcast::Receiver<ConsoleEvent>, each: &dyn Fn(&ConsoleEvent)) {
    loop {
        match rx.try_recv() {
            Ok(event) => each(&event),
            Err(TryRecvError::Lagged(skipped)) => {
                eprintln!("genaryx-ffi: broadcast receiver lagged, {skipped} pushes skipped");
            }
            Err(TryRecvError::Empty | TryRecvError::Closed) => break,
        }
    }
}

/// The ingest thread: sole owner of the `IngestService` (which is `Send` but
/// not `Sync`, so single-owner-thread is the intended shape, per its docs).
/// Each cycle: poll all sources, then synchronously forward every newly
/// broadcast event to the registered listeners. A poll error is logged and
/// the loop stays alive; it never panics and never goes silent.
fn spawn_ingest(
    mut service: IngestService,
    mut rx: broadcast::Receiver<ConsoleEvent>,
    listeners: Listeners,
    stop: Arc<AtomicBool>,
) -> Result<JoinHandle<()>, FfiError> {
    let handle = std::thread::Builder::new()
        .name("genaryx-ffi-ingest".into())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if let Err(e) = service.poll_once() {
                    eprintln!("genaryx-ffi: ingest poll error (loop kept alive): {e}");
                }
                drain(&mut rx, &|event| {
                    let ui = UiEvent::from(event);
                    for listener in relock(&listeners).iter() {
                        listener.on_event(ui.clone());
                    }
                });
                std::thread::sleep(POLL_INTERVAL);
            }
        })?;
    Ok(handle)
}

/// The feeder thread: appends one conforming v0.1 tokenfuse line to the
/// tailed file every ~1s, so the stream stays visibly live after the demo
/// campaign is ingested. Write errors are logged and the loop continues.
fn spawn_feeder(path: PathBuf, stop: Arc<AtomicBool>) -> Result<JoinHandle<()>, FfiError> {
    let handle = std::thread::Builder::new()
        .name("genaryx-ffi-feeder".into())
        .spawn(move || {
            let mut tick: u32 = 0;
            let mut line_no: u64 = 0;
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(FEEDER_TICK);
                tick = tick.wrapping_add(1);
                if !tick.is_multiple_of(FEEDER_TICKS_PER_LINE) {
                    continue;
                }
                line_no += 1;
                if let Err(e) = append_live_line(&path, line_no) {
                    eprintln!("genaryx-ffi: feeder append error (loop kept alive): {e}");
                }
            }
        })?;
    Ok(handle)
}

/// Append one live line. A single `write_all` of `line + "\n"` keeps the
/// tail parser's invariant simple: `FileTail` only consumes newline-complete
/// lines, so a torn read can only defer a line, never corrupt one.
fn append_live_line(path: &Path, line_no: u64) -> Result<(), FfiError> {
    let line = live_line(line_no)?;
    let mut file = OpenOptions::new().append(true).create(true).open(path)?;
    file.write_all(format!("{line}\n").as_bytes())?;
    Ok(())
}

/// One conforming agent-event v0.1 line (built through `serde_json`, never
/// string concatenation, mirroring `demo.rs`): source `tokenfuse` is in the
/// v0.1 closed source enum, `agent_id` matches the schema's
/// `^agent://[a-z0-9.-]+/[a-z0-9._/-]+$` pattern, `ts` is real RFC 3339 UTC.
fn live_line(line_no: u64) -> Result<String, FfiError> {
    let value = serde_json::json!({
        "schema": "taipanbox.dev/agent-event/v0.1",
        "ts": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        "source": "tokenfuse",
        "type": "spend_spike",
        "agent_id": "agent://taipanbox.dev/demo/live-feeder",
        "severity": "high",
        "run_id": format!("live-run-{line_no:03}"),
        "data": {
            "window_s": 60,
            "spend_usd": 1.0 + (line_no as f64) * 0.25,
            "baseline_usd": 0.8,
            "multiplier": 2.5,
            "live": true,
        },
    });
    serde_json::to_string(&value).map_err(|e| FfiError::Core { msg: e.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Rust-side stand-in for the Swift listener class; proves the callback
    /// path end to end without a foreign runtime (CI runs this on Linux).
    struct SinkListener(Arc<Mutex<Vec<UiEvent>>>);

    impl EventListener for SinkListener {
        fn on_event(&self, event: UiEvent) {
            relock(&self.0).push(event);
        }
    }

    #[test]
    fn constructs_serves_history_and_pushes_live_events() {
        let handle = FleetHandle::new().expect("construct demo world");

        // History: the priming poll stored the full demo campaign (~179
        // events) before the constructor returned.
        let recent = handle.recent_events(5).expect("recent_events");
        assert_eq!(recent.len(), 5, "expected 5 recent events");
        assert!(
            recent[0].id > recent[4].id,
            "recent_events must be newest-first by rowid"
        );
        let total = handle.event_count().expect("event_count");
        assert!(total >= 170, "expected the demo campaign, got {total}");

        // Live push: subscribe, then wait (bounded) for feeder lines to be
        // ingested and forwarded through the callback interface.
        let sink = Arc::new(Mutex::new(Vec::new()));
        handle.subscribe(Box::new(SinkListener(Arc::clone(&sink))));
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if relock(&sink).len() >= 2 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "no live events pushed within 15s"
            );
            std::thread::sleep(Duration::from_millis(100));
        }

        let events = relock(&sink);
        assert!(
            events
                .iter()
                .all(|e| e.agent_id == "agent://taipanbox.dev/demo/live-feeder"),
            "live pushes must be feeder lines"
        );
        assert!(
            events.iter().all(|e| e.source == "tokenfuse" && e.id == 0),
            "push-path events carry id 0 by contract"
        );
        drop(events);

        let dir = handle.dir.clone();
        drop(handle); // joins both threads
        assert!(!dir.exists(), "drop must remove the temp world");
    }

    /// PHASE3 W3: `agent_graph`/`agent_slice`/`events_for_agent` over the
    /// same demo campaign `crates/core/src/demo.rs` generates. `tier1-bot`
    /// is `AGENTS[0]`, acting on run index 0 - a block run (`i < 12`) whose
    /// index is a multiple of 4 and whose agent is not `orchestrator`, so
    /// per `demo::generate`'s own delegation rule it carries the fixed
    /// chain `user://taipanbox.dev/j.doe -> agent://.../orchestrator`,
    /// giving this test a real, known parent edge to assert on rather than
    /// a guessed one.
    #[test]
    fn agent_graph_agent_slice_and_events_for_agent_over_the_demo_campaign() {
        let handle = FleetHandle::new().expect("construct demo world");

        // agent_graph: a non-empty, internally coherent layout - every edge
        // endpoint is a known node, every position finite and in-bounds
        // (mirrors `layout::tests::every_node_positioned_in_bounds_and_finite`,
        // just through the FFI Record shape).
        let graph = handle.agent_graph().expect("agent_graph");
        assert!(
            !graph.nodes.is_empty(),
            "expected a non-empty delegation graph over the demo campaign"
        );
        assert!(
            !graph.edges.is_empty(),
            "expected delegation edges (a fraction of demo runs delegate through the orchestrator)"
        );
        let ids: std::collections::BTreeSet<&str> =
            graph.nodes.iter().map(|n| n.id.as_str()).collect();
        for edge in &graph.edges {
            assert!(
                ids.contains(edge.from.as_str()) && ids.contains(edge.to.as_str()),
                "edge {edge:?} must reference only known nodes"
            );
        }
        for node in &graph.nodes {
            assert!(
                node.x.is_finite() && node.y.is_finite(),
                "{} has a non-finite position",
                node.id
            );
            assert!(
                node.x >= 0.0 && node.x <= graph.width && node.y >= 0.0 && node.y <= graph.height,
                "{} position out of the {}x{} canvas bounds",
                node.id,
                graph.width,
                graph.height
            );
        }

        // agent_slice: tier1-bot has a real node (it acted) and delegates
        // through the orchestrator on at least one of its demo runs.
        const TIER1: &str = "agent://taipanbox.dev/demo/tier1-bot";
        const ORCHESTRATOR: &str = "agent://taipanbox.dev/demo/orchestrator";
        let slice = handle
            .agent_slice(TIER1.to_string())
            .expect("agent_slice for a demo agent");
        let node = slice.node.expect("tier1-bot must have acted on the bus");
        assert_eq!(node.id, TIER1);
        assert!(
            node.event_count > 0,
            "tier1-bot must have a positive event_count"
        );
        assert!(
            slice.parents.iter().any(|p| p.id == ORCHESTRATOR),
            "tier1-bot's demo runs delegate through the orchestrator, got parents {:?}",
            slice.parents
        );

        // The orchestrator itself: a delegator with real children, and its
        // own parent is the fixed demo user root - proves the neighborhood
        // is symmetric, not just readable from one side.
        let orch_slice = handle
            .agent_slice(ORCHESTRATOR.to_string())
            .expect("agent_slice for the orchestrator");
        assert!(orch_slice.node.is_some());
        assert!(
            orch_slice
                .parents
                .iter()
                .any(|p| p.id == "user://taipanbox.dev/j.doe"),
            "the orchestrator's own parent must be the demo user root"
        );
        assert!(
            !orch_slice.children.is_empty(),
            "the orchestrator must have delegatee children in the demo campaign"
        );

        // An agent never seen on the bus yields an all-empty slice, never an
        // error (`DelegationGraph::agent_slice`'s own fail-closed contract).
        let unknown = handle
            .agent_slice("agent://taipanbox.dev/demo/does-not-exist".to_string())
            .expect("agent_slice must succeed even for an unknown agent");
        assert!(
            unknown.node.is_none() && unknown.parents.is_empty() && unknown.children.is_empty(),
            "an unseen agent must yield an all-empty slice"
        );

        // events_for_agent: only this agent's own events, newest first.
        let events = handle
            .events_for_agent(TIER1.to_string(), 50)
            .expect("events_for_agent for a demo agent");
        assert!(!events.is_empty(), "tier1-bot must have stored events");
        assert!(
            events.iter().all(|e| e.agent_id == TIER1),
            "events_for_agent must return only this agent's own events"
        );
        if events.len() > 1 {
            assert!(
                events[0].id >= events[1].id,
                "events_for_agent must be newest-first by rowid"
            );
        }
    }

    /// PHASE3 W4: `events_for_run` over the same demo campaign
    /// `crates/core/src/demo.rs` generates. `demo-run-000` is run index 0 -
    /// one of the first `BLOCK_RUN_COUNT` "block" runs (`demo::run_calls`),
    /// so it is a real, known MULTI-SOURCE run (exactly three calls: wardryx
    /// `policy_allow`, tokenfuse `budget_exhausted`, engram
    /// `memory_written`, in that wall-clock order per
    /// `demo::block_run_calls`) rather than a guessed one - giving this test
    /// a chance to prove the subtlety called out in
    /// [`FleetHandle::events_for_run`]'s own doc comment: because
    /// `DEMO_SOURCES` registers tokenfuse before wardryx before engram, and
    /// `IngestService::poll_once` drains one source's whole backlog before
    /// the next, this run's events land in the Store id-ordered as
    /// (tokenfuse, wardryx, engram) - SOURCE order - even though wardryx's
    /// call is the one actually timestamped first. So "oldest-first" here
    /// is proven as an `id` (insertion) guarantee, not mistaken for a `ts`
    /// guarantee.
    #[test]
    fn events_for_run_over_the_demo_campaign_is_oldest_first_by_id() {
        let handle = FleetHandle::new().expect("construct demo world");

        const RUN: &str = "demo-run-000";
        let events = handle
            .events_for_run(RUN.to_string(), 50)
            .expect("events_for_run for a demo run");

        assert_eq!(
            events.len(),
            3,
            "demo-run-000 is a known 3-call block run: {events:?}"
        );
        assert!(
            events.iter().all(|e| e.run_id.as_deref() == Some(RUN)),
            "events_for_run must return only this run's own events: {events:?}"
        );
        let sources: Vec<&str> = events.iter().map(|e| e.source.as_str()).collect();
        assert_eq!(
            sources,
            vec!["tokenfuse", "wardryx", "engram"],
            "id order follows DEMO_SOURCES registration order (tokenfuse, wardryx, \
             engram), not wall-clock ts order - see this test's own doc comment"
        );
        assert!(
            events[0].id < events[1].id && events[1].id < events[2].id,
            "events_for_run must be oldest-first by id: {events:?}"
        );

        // A run_id this Store has never seen (e.g. a Money/Cloud-only run)
        // yields a clean empty Vec, never an error - the Run Replay view's
        // own honest empty state, mirroring `agent_slice`'s "an unseen
        // agent is a normal empty outcome" fail-closed contract.
        let unknown = handle
            .events_for_run("does-not-exist".to_string(), 50)
            .expect("events_for_run must succeed even for an unknown run_id");
        assert!(
            unknown.is_empty(),
            "an unknown run_id must yield an empty Vec, not an error"
        );
    }
}
