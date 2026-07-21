//! The live path: fills `genaryx-core`'s real `Store` at startup and keeps a
//! background thread forwarding new events to the frontend, so the Bus
//! Explorer updates without a reload (Phase-0 exit gate: "both shells show
//! the same live event stream from the shared core").
//!
//! There are two ways that stream can be obtained, and which one is in use is
//! never hidden from the operator (see [`BusMode`]):
//!
//! 1. **Live.** An environment resolved through [`genaryx_core::bus`], so the
//!    `*.ndjson` files under its `events.dir` are tailed. Real events, real
//!    timestamps, and an honest empty bus when nothing has happened yet.
//! 2. **Demo.** No environment exists on this machine at all, so the fixtures
//!    are generated and a synthetic feeder appends one conforming event every
//!    ~2s. Useful for a first look and for screenshots, and labelled as demo
//!    everywhere it surfaces.
//!
//! Until 2026-07-21 only path 2 existed, unconditionally, in both shells: the
//! Phase-0 scaffold was never replaced, so every console on every machine
//! showed a fabricated stream while the descriptor sitting in
//! `~/.taipan/environments/` already carried the real `events.dir`. The rule
//! that follows from fixing it is worth stating, because it is the whole
//! point: **a resolved environment never falls back to demo**, not even when
//! it has produced nothing yet. An empty real bus is information. A
//! fabricated one presented in its place is not.
//!
//! Ownership: `Store`/`IngestService` wrap a `rusqlite::Connection`, which is
//! `Send` but not `Sync` (see `genaryx_core::ingest` module docs). Rather than
//! share one behind a lock, this module hands the `IngestService` to exactly
//! one background thread for the rest of the process's life. The
//! `recent_events` Tauri command (in `lib.rs`) never touches that instance;
//! it opens its own short-lived reader `Store` instead, which is safe because
//! `Store::open` always sets `journal_mode=WAL` (readers never block on the
//! writer).

use crate::events::UiEvent;
use genaryx_core::demo;
use genaryx_core::event::ConsoleEvent;
use genaryx_core::ingest::IngestService;
use genaryx_core::store::Store;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::broadcast;

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

/// Tauri-managed state: where the console's store lives (if startup
/// succeeded) and how it is being fed. `events_dir: None` means
/// [`crate::recent_events`] falls back to `events::mock_events` (fail-closed:
/// a startup failure degrades the Bus Explorer; it never crashes the app or
/// traps the UI).
pub struct AppState {
    pub events_dir: Option<PathBuf>,
    pub mode: BusMode,
}

/// What [`bootstrap`] resolved, handed back to `lib.rs`'s `setup` hook.
pub struct BusBootstrap {
    /// The directory holding `console.sqlite`. In demo mode that is the
    /// generated fixture directory; in live mode it is a console-owned
    /// scratch directory, NOT the environment's own `events.dir` (the console
    /// must never write its database into a directory `taipan up` owns).
    pub events_dir: PathBuf,
    pub mode: BusMode,
}

/// Tauri event name the frontend `listen()`s for; payload is one [`UiEvent`].
pub const LIVE_EVENT: &str = "bus:event";

/// Cadence of the background tick, for both the live tailer and the demo
/// feeder (spec: "every ~2s").
const FEEDER_INTERVAL: Duration = Duration::from_secs(2);

/// Open the console's bus and start the background thread that feeds it.
///
/// Live if an environment resolves, demo if none does, and never a mixture:
/// see this module's header for why a resolved-but-empty environment must not
/// fall back to fixtures. The caller degrades to mock data on `Err` rather
/// than failing app startup.
pub fn bootstrap(app_handle: AppHandle) -> genaryx_core::Result<BusBootstrap> {
    match genaryx_core::bus::discover() {
        Some(resolved) => bootstrap_live(app_handle, resolved),
        None => bootstrap_demo(app_handle),
    }
}

/// Tail a real environment's event files.
///
/// The store is per-process scratch, exactly as in demo mode, and that is a
/// deliberate limit of this slice rather than an oversight. A store that
/// persisted across restarts is what turns this bus into history, but it also
/// needs an answer for re-ingestion: `stack-up` truncates its event files on
/// every start, `FileTail` correctly resets to offset 0 when it sees that,
/// and with no dedupe key the whole file would then be inserted a second
/// time. Durable history is its own piece of work (the `runs` table and
/// retention); until then a fresh store per launch is the honest option,
/// because it can only ever show what the files really contain.
fn bootstrap_live(
    app_handle: AppHandle,
    resolved: genaryx_core::bus::ResolvedBus,
) -> genaryx_core::Result<BusBootstrap> {
    let store_dir = unique_events_dir();
    std::fs::create_dir_all(&store_dir)?;

    let db_path = store_dir.join("console.sqlite");
    let store = Store::open(&db_path)?;
    let mut ingest = IngestService::new(store, resolved.env_name.as_str())?;

    // An events dir that does not exist yet is not an error: the environment
    // is configured, the products that write into it simply have not run.
    // `collect_ndjson_files` reports none, the tailer rescans every tick, and
    // the Bus Explorer shows an honest empty bus until something arrives.
    let mut tailed = Vec::new();
    for path in collect_ndjson_files(&resolved.events_dir).unwrap_or_default() {
        add_source(&mut ingest, &path)?;
        tailed.push(path);
    }

    let receiver = ingest.subscribe();
    let stats = ingest.poll_once()?;
    eprintln!(
        "genaryx: bus LIVE on environment {:?} at {} ({} file(s), {} event(s) ingested, {} quarantined)",
        resolved.env_name,
        resolved.events_dir.display(),
        tailed.len(),
        stats.inserted,
        stats.quarantined
    );

    let events_dir = resolved.events_dir.clone();
    std::thread::spawn(move || run_tailer(ingest, receiver, events_dir, tailed, app_handle));

    Ok(BusBootstrap {
        events_dir: store_dir,
        mode: BusMode::Live {
            env: resolved.env_name,
            dir: resolved.events_dir.display().to_string(),
        },
    })
}

/// No environment: generate fixtures and run the synthetic feeder.
fn bootstrap_demo(app_handle: AppHandle) -> genaryx_core::Result<BusBootstrap> {
    let events_dir = unique_events_dir();
    let generated = demo::generate(&events_dir)?;

    let db_path = events_dir.join("console.sqlite");
    let store = Store::open(&db_path)?;
    let mut ingest = IngestService::new(store, "demo")?;

    let files = collect_ndjson_files(&events_dir)?;
    for path in &files {
        add_source(&mut ingest, path)?;
    }

    // Subscribe before the first poll so no batch is missed (per the
    // `IngestService::subscribe` doc comment). In practice the initial seed
    // batch is never forwarded live anyway, since `recent_events` reads it
    // straight from the Store; this only matters so the channel exists
    // before `poll_once` below could otherwise race a subscriber.
    let receiver = ingest.subscribe();
    let stats = ingest.poll_once()?;
    eprintln!(
        "genaryx: bus DEMO (no environment found under ~/.taipan/environments) at {} ({generated} generated, {} inserted, {} quarantined)",
        events_dir.display(),
        stats.inserted,
        stats.quarantined
    );

    std::thread::spawn(move || run_feeder(ingest, receiver, files, app_handle));

    Ok(BusBootstrap {
        events_dir: events_dir.clone(),
        mode: BusMode::Demo {
            dir: events_dir.display().to_string(),
        },
    })
}

/// Register one `*.ndjson` file as a tailed source, keyed by its file stem so
/// the id is stable across restarts and readable in the offset journal.
fn add_source(ingest: &mut IngestService, path: &Path) -> genaryx_core::Result<()> {
    let id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    ingest.add_file_source(format!("filetail:{id}"), path)
}

/// A fresh, distinct-per-process directory so a restart never reuses another
/// run's `console.sqlite`. Reusing one would double-count events on every
/// restart: `demo::generate` deterministically rewrites the NDJSON files back
/// to the same ~179-event baseline, but the offset journal left over from a
/// previous, feeder-extended run would then look like the file had been
/// truncated, and the whole baseline would be re-ingested as new rows. Plain
/// ephemeral scratch space; never cleaned up here (Phase-0 demo storage).
fn unique_events_dir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "genaryx-desktop-events-{}-{nanos}",
        std::process::id()
    ))
}

/// Every `*.ndjson` file in `dir`, sorted.
///
/// In demo mode that is whatever `demo::generate` just wrote, so the feeder
/// registers a `FileTail` per source without hardcoding `demo`'s private
/// source list here (mirroring that private list, the way `events.rs`'s mock
/// data mirrors `demo`'s topic/eval/scenario lists, would drift silently if
/// `demo` ever added a source; reading the directory back cannot drift). In
/// live mode it is whichever products have written into the environment's
/// `events.dir` so far, which is why the tailer calls this every tick and not
/// only at startup.
fn collect_ndjson_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("ndjson"))
        .collect();
    files.sort();
    Ok(files)
}

/// Live mode's loop: every ~2s, pick up any event file that appeared since
/// the last tick, poll every tailed file, and forward whatever arrived. It
/// writes nothing, ever: the only source of events here is the products
/// themselves.
///
/// The rescan matters more than it looks. Wave-2 tools create their event
/// file lazily, on their first run: there is no `qryx.ndjson` until something
/// scans, no `mockryx.ndjson` until a drill fires. A directory listing taken
/// once at startup would miss precisely the sources an operator is most
/// likely to be waiting for, and would keep missing them until the console
/// was restarted. Re-listing one small directory every two seconds is a
/// rounding error next to that.
///
/// Failures are logged and retried on the next tick rather than ending the
/// loop, mirroring the demo feeder: a bus that stops reading is much worse
/// than a bus that skips one cycle.
fn run_tailer(
    mut ingest: IngestService,
    mut receiver: broadcast::Receiver<ConsoleEvent>,
    events_dir: PathBuf,
    mut tailed: Vec<PathBuf>,
    app_handle: AppHandle,
) {
    loop {
        std::thread::sleep(FEEDER_INTERVAL);

        for path in collect_ndjson_files(&events_dir).unwrap_or_default() {
            if tailed.contains(&path) {
                continue;
            }
            match add_source(&mut ingest, &path) {
                Ok(()) => {
                    eprintln!("genaryx: bus picked up a new source: {}", path.display());
                    tailed.push(path);
                }
                Err(e) => eprintln!("genaryx: bus could not tail {}: {e}", path.display()),
            }
        }

        if let Err(e) = ingest.poll_once() {
            eprintln!("genaryx: bus poll_once failed: {e}");
            continue;
        }

        drain_and_emit(&mut receiver, &app_handle);
    }
}

/// Demo mode's loop, and demo mode's only. Owns the `IngestService` (and its
/// `Store`) for the rest of the process: every ~2s, append one conforming
/// demo-shaped line to one of the seeded NDJSON files, poll it into the Store
/// like any other ingest cycle, then forward whatever that poll just
/// broadcast to the frontend. Never panics on a transient failure; a bad tick
/// is logged and the loop just tries again next tick (fail-closed: the live
/// feed degrading is never the whole app crashing).
///
/// This function fabricates events. It must never run for a resolved
/// environment, which is why it is reachable only from [`bootstrap_demo`].
fn run_feeder(
    mut ingest: IngestService,
    mut receiver: broadcast::Receiver<ConsoleEvent>,
    files: Vec<PathBuf>,
    app_handle: AppHandle,
) {
    let mut tick: u64 = 0;
    loop {
        std::thread::sleep(FEEDER_INTERVAL);
        tick += 1;

        let (source, line) = feeder_line(tick);
        match pick_file(&files, source) {
            Some(path) => {
                if let Err(e) = append_line(path, &line) {
                    eprintln!(
                        "genaryx: live feeder could not append to {}: {e}",
                        path.display()
                    );
                    continue;
                }
            }
            None => {
                eprintln!("genaryx: live feeder found no ndjson file for source {source:?}");
                continue;
            }
        }

        if let Err(e) = ingest.poll_once() {
            eprintln!("genaryx: live feeder poll_once failed: {e}");
            continue;
        }

        drain_and_emit(&mut receiver, &app_handle);
    }
}

/// Drain every event the poll just broadcast (normally exactly one, since the
/// feeder appends exactly one line per tick) and forward each to the
/// frontend. `try_recv` never blocks and never needs an async runtime (the
/// broadcast channel's `send`/`try_recv` pair is plain synchronous state, see
/// `genaryx_core::ingest`'s own `run_blocking` doc comment); a lagged
/// receiver just logs the gap rather than panicking, though at one
/// subscriber and a 2s cadence it is not expected to happen.
fn drain_and_emit(receiver: &mut broadcast::Receiver<ConsoleEvent>, app_handle: &AppHandle) {
    loop {
        match receiver.try_recv() {
            Ok(ce) => {
                let ui_event = UiEvent::from(ce);
                if let Err(e) = app_handle.emit(LIVE_EVENT, ui_event) {
                    eprintln!("genaryx: failed to emit live event: {e}");
                }
            }
            Err(broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed) => {
                break;
            }
            Err(broadcast::error::TryRecvError::Lagged(n)) => {
                eprintln!("genaryx: live feeder receiver lagged by {n} events");
            }
        }
    }
}

/// The seeded ndjson file matching `source`'s file stem, falling back to the
/// first available file so a feeder tick can never simply have nowhere to
/// write (fail-closed over an exact-match requirement).
fn pick_file<'a>(files: &'a [PathBuf], source: &str) -> Option<&'a PathBuf> {
    files
        .iter()
        .find(|p| p.file_stem().and_then(|s| s.to_str()) == Some(source))
        .or_else(|| files.first())
}

fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
    writeln!(file, "{line}")
}

/// Build one conforming demo-shaped NDJSON line for feeder tick `tick`,
/// cycling through a small set of the same (source, type, severity, data)
/// combinations `genaryx_core::demo` uses (08 §5 conventions), so a live row
/// looks like a seeded one. `agent_id`/`run_id` mark it as specifically the
/// live feed (rather than reusing one of the seeded demo agents), so it is
/// easy to tell "seeded at startup" from "arrived live" while sanity-checking
/// the wiring end to end.
fn feeder_line(tick: u64) -> (&'static str, String) {
    use genaryx_core::event::SchemaVersion;

    const VARIANTS: [(&str, SchemaVersion, &str, &str); 4] = [
        ("wardryx", SchemaVersion::V0_2, "policy_allow", "info"),
        ("verdryx", SchemaVersion::V0_2, "quality_score", "info"),
        ("mockryx", SchemaVersion::V0_2, "sim_run", "info"),
        ("engram", SchemaVersion::V0_1, "memory_written", "info"),
    ];
    let (source, schema, event_type, severity) = VARIANTS[(tick as usize) % VARIANTS.len()];

    let data = match event_type {
        "policy_allow" => {
            serde_json::json!({"policy": "default-allow", "reason": "within policy"})
        }
        "quality_score" => {
            serde_json::json!({"eval_suite": "live-feed-qa", "current_score": 0.95})
        }
        "sim_run" => {
            serde_json::json!({"scenario": "live-feed-heartbeat", "status": "completed"})
        }
        _ => serde_json::json!({
            "memory_id": format!("mem-live-{tick:06}"),
            "topic": "live_feed_heartbeat",
        }),
    };

    let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let line = serde_json::json!({
        "schema": schema.as_str(),
        "ts": ts,
        "source": source,
        "type": event_type,
        "agent_id": "agent://taipanbox.dev/demo/live-feed",
        "severity": severity,
        "run_id": "live-feed",
        "data": data,
    });
    (source, line.to_string())
}

#[cfg(test)]
mod tests {
    //! Sanity check for the seeding half of [`bootstrap`] (everything except
    //! the Tauri-specific parts: no `AppHandle`/`emit` available in a plain
    //! `cargo test`, so this exercises `demo::generate` -> `Store` ->
    //! `IngestService::add_file_source` -> `poll_once` directly and asserts
    //! the resulting event count, the same seeding path `bootstrap` runs at
    //! startup.
    use super::*;

    #[test]
    fn seeds_the_store_from_demo_fixtures() {
        let dir = std::env::temp_dir().join(format!(
            "genaryx-desktop-events-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));

        let generated = demo::generate(&dir).expect("demo::generate");

        let db_path = dir.join("console.sqlite");
        let store = Store::open(&db_path).expect("Store::open");
        let mut ingest = IngestService::new(store, "local").expect("IngestService::new");

        let files = collect_ndjson_files(&dir).expect("collect_ndjson_files");
        assert_eq!(
            files.len(),
            6,
            "expected one ndjson file per emitting source"
        );
        for path in &files {
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            ingest
                .add_file_source(format!("filetail:{id}"), path)
                .expect("add_file_source");
        }

        let stats = ingest.poll_once().expect("poll_once");
        eprintln!(
            "seeds_the_store_from_demo_fixtures: demo::generate={generated} inserted={} quarantined={} event_count={}",
            stats.inserted,
            stats.quarantined,
            ingest.store().event_count().expect("event_count")
        );

        assert_eq!(stats.quarantined, 0, "every demo line must conform");
        assert_eq!(
            stats.inserted, generated,
            "every generated line should have been inserted"
        );
        assert_eq!(
            ingest.store().event_count().expect("event_count"),
            generated as u64,
            "store should hold exactly the generated events after one poll"
        );

        let _ = std::fs::remove_dir_all(&dir); // best-effort cleanup; not load-bearing
    }
}
