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

pub mod cloud;

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use chrono::{SecondsFormat, Utc};
use genaryx_core::store::{Store, StoredEvent};
use genaryx_core::{ConsoleEvent, IngestService, demo};
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
}
