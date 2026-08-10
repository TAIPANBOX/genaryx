//! The live path: fills `genaryx-core`'s real `Store` at startup and keeps a
//! background thread forwarding new events to an [`EventSink`], so the Bus
//! Explorer updates without a reload (Phase-0 exit gate: "both shells show
//! the same live event stream from the shared core" - back when there were
//! two shells to keep in sync; today there is one). This module ran
//! identically for both shells while both existed: only the final delivery
//! differed, via whichever [`EventSink`] the caller handed to [`bootstrap`]
//! (the old desktop shell's was a Tauri window event; the web shell's, still
//! in use today, is an SSE broadcast to its subscribers).
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
//! Until 2026-07-21 only path 2 existed, unconditionally, in every shell this
//! project shipped at the time: the Phase-0 scaffold was never replaced, so
//! every console on every machine
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
//! one background thread for the rest of the process's life. The web shell's
//! own `recent_events` read path (an HTTP handler) never touches that
//! instance; it opens its own short-lived reader `Store` instead, which is
//! safe because `Store::open` always sets `journal_mode=WAL` (readers never
//! block on the writer).

use super::BusMode;
use crate::events::UiEvent;
use genaryx_core::demo;
use genaryx_core::event::ConsoleEvent;
use genaryx_core::ingest::IngestService;
use genaryx_core::store::Store;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::broadcast;

/// A destination for one [`UiEvent`] at a time, so the feeder loop below
/// never needs to know the sink's own delivery mechanism - see this module's
/// header for what that is today. `emit` takes no `Result`: a delivery
/// failure (say, a dropped SSE subscriber) is each sink's own concern to log,
/// not this generic loop's job.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: crate::events::UiEvent);
}

/// What [`bootstrap`] resolved, handed back to the calling shell's own
/// startup path (the desktop's `setup` hook in `lib.rs`, the web shell's own
/// equivalent).
pub struct BusBootstrap {
    /// The directory holding `console.sqlite`. In demo mode that is the
    /// generated fixture directory; in live mode it is a console-owned
    /// scratch directory, NOT the environment's own `events.dir` (the console
    /// must never write its database into a directory `taipan up` owns).
    /// Where the console's own SQLite store lives: a fresh directory per
    /// launch (see this module's doc on why history is not durable yet).
    pub events_dir: PathBuf,
    /// Where the PRODUCTS write their NDJSON, i.e. the directory this bus
    /// tails. Distinct from `events_dir` above, and the distinction is
    /// load-bearing: a `console_command` written into the store directory is
    /// written somewhere nothing tails and nothing keeps, so the action
    /// vanishes from the bus and from every evidence pack built afterwards.
    pub source_events_dir: PathBuf,
    pub mode: BusMode,
}

/// Cadence of the background tick, for both the live tailer and the demo
/// feeder (spec: "every ~2s").
const FEEDER_INTERVAL: Duration = Duration::from_secs(2);

/// Open the console's bus and start the background thread that feeds it.
///
/// Live if an environment resolves, demo if none does, and never a mixture:
/// see this module's header for why a resolved-but-empty environment must not
/// fall back to fixtures. The caller degrades to mock data on `Err` rather
/// than failing app startup. Generic over `S: EventSink` so the caller can
/// hand in its own way of delivering a [`UiEvent`] without this module
/// knowing, or needing to know, the concrete mechanism (the module header
/// covers what that has meant across shells).
pub fn bootstrap<S: EventSink>(sink: S) -> genaryx_core::Result<BusBootstrap> {
    match genaryx_core::bus::discover() {
        Some(resolved) => bootstrap_live(sink, resolved),
        None => bootstrap_demo(sink),
    }
}

/// Tail a real environment's event files, into a store that OUTLIVES the
/// process.
///
/// This used to be per-process scratch, and the note here used to explain why:
/// re-ingestion. `stack-up` truncates its event files on every start,
/// `FileTail` correctly resets to offset 0 when it sees that, and with no
/// dedupe key the whole file landed a second time. Both halves of the answer
/// now exist in `genaryx-core`'s store, so the limit is lifted:
///
/// - every row carries a `dedupe` key over (env, file, offset, raw), so a
///   replay of bytes already held is a no-op rather than a second copy;
/// - the offset journal records the file's inode, so a file replaced while the
///   console was down is re-read from the top instead of resumed at an offset
///   that belongs to a file that is gone.
///
/// The store is per ENVIRONMENT, under the same `TAIPAN_HOME` the rest of the
/// install honours. Two environments would otherwise share one history and
/// answer each other's questions.
///
/// Demo mode stays scratch, deliberately. Its events are regenerated on every
/// launch, so persisting them would accumulate fixture data that no product
/// ever emitted and call it history, in the one product whose whole claim is
/// that what you see is what happened.
fn bootstrap_live<S: EventSink>(
    sink: S,
    resolved: genaryx_core::bus::ResolvedBus,
) -> genaryx_core::Result<BusBootstrap> {
    let store_dir = durable_store_dir(&resolved.env_name).unwrap_or_else(|| {
        eprintln!(
            "genaryx: neither TAIPAN_HOME nor HOME is set, so this session keeps no history; \
             events are kept for this process only"
        );
        unique_events_dir()
    });
    std::fs::create_dir_all(&store_dir)?;

    let db_path = store_dir.join("console.sqlite");
    let store = Store::open(&db_path)?;
    prune_history(&store);
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
    std::thread::spawn(move || run_tailer(ingest, receiver, events_dir, tailed, sink));

    Ok(BusBootstrap {
        events_dir: store_dir,
        source_events_dir: resolved.events_dir.clone(),
        mode: BusMode::Live {
            env: resolved.env_name,
            dir: resolved.events_dir.display().to_string(),
        },
    })
}

/// No environment: generate fixtures and run the synthetic feeder.
fn bootstrap_demo<S: EventSink>(sink: S) -> genaryx_core::Result<BusBootstrap> {
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

    let feeder_dir = events_dir.clone();
    std::thread::spawn(move || run_feeder(ingest, receiver, feeder_dir, files, sink));

    Ok(BusBootstrap {
        events_dir: events_dir.clone(),
        // Demo mode seeds and tails the SAME directory, so the two are one
        // path here. Only the live path splits them.
        source_events_dir: events_dir.clone(),
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

/// Tail any `*.ndjson` file in `dir` that is not tailed yet, extending
/// `tailed` with each one taken up.
///
/// Both loops call this every tick, and both need it for the same reason: a
/// source file on this bus is created lazily, by whichever tool first has
/// something to say. There is no `qryx.ndjson` until something scans, no
/// `mockryx.ndjson` until a drill fires, and no `console.ndjson` until the
/// operator issues their first privileged command. A directory listing taken
/// once at startup misses precisely the sources somebody is waiting for, and
/// keeps missing them until the console is restarted.
///
/// A file that cannot be registered is logged and retried on the next tick
/// rather than ending the loop: a bus that stops reading is much worse than
/// a bus that skips a cycle.
fn pick_up_new_sources(ingest: &mut IngestService, dir: &Path, tailed: &mut Vec<PathBuf>) {
    for path in collect_ndjson_files(dir).unwrap_or_default() {
        if tailed.contains(&path) {
            continue;
        }
        match add_source(ingest, &path) {
            Ok(()) => {
                eprintln!("genaryx: bus picked up a new source: {}", path.display());
                tailed.push(path);
            }
            Err(e) => eprintln!("genaryx: bus could not tail {}: {e}", path.display()),
        }
    }
}

/// A fresh, distinct-per-process directory so a restart never reuses another
/// run's `console.sqlite`. Reusing one would double-count events on every
/// restart: `demo::generate` deterministically rewrites the NDJSON files back
/// to the same ~179-event baseline, but the offset journal left over from a
/// previous, feeder-extended run would then look like the file had been
/// truncated, and the whole baseline would be re-ingested as new rows. Plain
/// ephemeral scratch space; never cleaned up here (Phase-0 demo storage).
/// Where one environment's durable history lives: `<TAIPAN_HOME>/genaryx/<env>`.
///
/// Under the same root the rest of the install already uses, and keyed by
/// environment, because two environments hold different estates and a single
/// shared history would let one answer questions about the other.
///
/// The environment name is sanitized to one path segment: it comes from a
/// descriptor file, and a name carrying `..` or a slash must not be able to
/// choose where the console writes.
fn durable_store_dir(env: &str) -> Option<PathBuf> {
    let root = genaryx_core::taipan_home::environments_dir()?
        .parent()?
        .join("genaryx");
    let safe: String = env
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let safe = if safe.is_empty() {
        "default".to_string()
    } else {
        safe
    };
    Some(root.join(safe))
}

/// How long a durable store keeps events, in days.
///
/// Ninety is a quarter, which is the shortest window an operator is plausibly
/// asked about after the fact ("what did this agent do last quarter"). It is
/// deliberately a plain number of days rather than a size cap: a size cap
/// deletes the busy weeks first, which are the ones somebody comes back for.
///
/// `GENARYX_HISTORY_DAYS=0` keeps everything, for a box where an auditor owns
/// the retention decision rather than this default.
const DEFAULT_HISTORY_DAYS: i64 = 90;

/// Drop anything past the retention horizon, and SAY how much went.
///
/// Announced rather than silent: a console that quietly deletes an operator's
/// evidence is a worse failure than one that keeps too much, and the line in
/// the log is the only place the trade-off is visible on a running box.
fn prune_history(store: &Store) {
    let days = std::env::var("GENARYX_HISTORY_DAYS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(DEFAULT_HISTORY_DAYS);
    if days <= 0 {
        eprintln!(
            "genaryx: history retention is off (GENARYX_HISTORY_DAYS={days}), keeping everything"
        );
        return;
    }
    let cutoff = chrono::Utc::now().timestamp_millis() - days * 86_400_000;
    match store.prune_before(cutoff) {
        Ok((0, 0)) => {}
        Ok((events, quarantined)) => eprintln!(
            "genaryx: history retention dropped {events} event(s) and {quarantined} quarantined \
             line(s) older than {days} days"
        ),
        // A store that cannot be pruned is still a usable store. Say so and
        // carry on rather than refusing to start over housekeeping.
        Err(e) => eprintln!("genaryx: history retention could not run: {e}"),
    }
}

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
/// the last tick ([`pick_up_new_sources`]), poll every tailed file, and
/// forward whatever arrived. It writes nothing, ever: the only source of
/// events here is the products themselves.
///
/// Failures are logged and retried on the next tick rather than ending the
/// loop, mirroring the demo feeder: a bus that stops reading is much worse
/// than a bus that skips one cycle.
fn run_tailer<S: EventSink>(
    mut ingest: IngestService,
    mut receiver: broadcast::Receiver<ConsoleEvent>,
    events_dir: PathBuf,
    mut tailed: Vec<PathBuf>,
    sink: S,
) {
    loop {
        std::thread::sleep(FEEDER_INTERVAL);

        pick_up_new_sources(&mut ingest, &events_dir, &mut tailed);

        if let Err(e) = ingest.poll_once() {
            eprintln!("genaryx: bus poll_once failed: {e}");
            continue;
        }

        drain_and_emit(&mut receiver, &sink);
    }
}

/// Demo mode's loop, and demo mode's only. Owns the `IngestService` (and its
/// `Store`) for the rest of the process: every ~2s, pick up any event file
/// that appeared since the last tick ([`pick_up_new_sources`]), append one
/// conforming demo-shaped line to one of the seeded NDJSON files, poll it
/// into the Store like any other ingest cycle, then forward whatever that
/// poll just broadcast to the sink. Never panics on a transient failure; a
/// bad tick is logged and the loop just tries again next tick (fail-closed:
/// the live feed degrading is never the whole app crashing).
///
/// The rescan is here for the console's own file and nothing else: the
/// demo fixtures are all written before this loop starts, but
/// `console.ndjson` is created by the operator's first privileged command,
/// which can happen at any point afterwards. `files` stays the SEEDED list,
/// because it decides where a synthetic line is appended and the feeder must
/// never write into the console's chain.
///
/// This function fabricates events. It must never run for a resolved
/// environment, which is why it is reachable only from [`bootstrap_demo`].
fn run_feeder<S: EventSink>(
    mut ingest: IngestService,
    mut receiver: broadcast::Receiver<ConsoleEvent>,
    events_dir: PathBuf,
    files: Vec<PathBuf>,
    sink: S,
) {
    let mut tailed = files.clone();
    let mut tick: u64 = 0;
    loop {
        std::thread::sleep(FEEDER_INTERVAL);
        tick += 1;

        pick_up_new_sources(&mut ingest, &events_dir, &mut tailed);

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

        drain_and_emit(&mut receiver, &sink);
    }
}

/// Drain every event the poll just broadcast (normally exactly one, since the
/// feeder appends exactly one line per tick) and forward each to `sink`.
/// `try_recv` never blocks and never needs an async runtime (the broadcast
/// channel's `send`/`try_recv` pair is plain synchronous state, see
/// `genaryx_core::ingest`'s own `run_blocking` doc comment); a lagged
/// receiver just logs the gap rather than panicking, though at one
/// subscriber and a 2s cadence it is not expected to happen. `EventSink::emit`
/// takes no `Result`, so a delivery failure (say, a dropped SSE subscriber)
/// is the sink's own concern to log, not this generic loop's.
fn drain_and_emit<S: EventSink>(receiver: &mut broadcast::Receiver<ConsoleEvent>, sink: &S) {
    loop {
        match receiver.try_recv() {
            Ok(ce) => {
                let ui_event = UiEvent::from(ce);
                sink.emit(ui_event);
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
    //! Sanity check for the seeding half of [`bootstrap`]'s generic path
    //! (everything up to the point an event would reach an `EventSink`,
    //! which the web shell verifies against its own sink instead - each
    //! former desktop shell did the same against its own): this crate
    //! has no Tauri dependency, or any other shell's dependency, at all, so
    //! this exercises `demo::generate` -> `Store` ->
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

    /// A source file that appears AFTER startup still reaches the bus.
    ///
    /// The live tailer already re-lists the directory every tick, because
    /// wave-2 tools create their file lazily on first run. Demo mode listed
    /// once and never again, which did not matter while the console appended
    /// its `console_command` lines into a file `demo::generate` had already
    /// created. Now that the console writes its own file, created by the
    /// first privileged action, "listed once" means an operator's own kill
    /// never appears in the Bus Explorer.
    #[test]
    fn a_source_file_created_after_startup_is_picked_up() {
        let dir = std::env::temp_dir().join(format!(
            "genaryx-late-source-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let generated = demo::generate(&dir).expect("demo::generate");

        let store = Store::open(&dir.join("console.sqlite")).expect("Store::open");
        let mut ingest = IngestService::new(store, "demo").expect("IngestService::new");
        let mut tailed = collect_ndjson_files(&dir).expect("collect_ndjson_files");
        for path in &tailed {
            add_source(&mut ingest, path).expect("add_source");
        }
        ingest.poll_once().expect("seed poll");
        assert_eq!(
            ingest.store().event_count().expect("event_count"),
            generated as u64
        );

        // The console's first privileged action creates its file.
        let console = dir.join("console.ndjson");
        std::fs::write(
            &console,
            "{\"schema\":\"taipanbox.dev/agent-event/v0.2\",\
             \"ts\":\"2026-08-06T10:00:00.000Z\",\"source\":\"console\",\
             \"type\":\"console_command\",\
             \"agent_id\":\"agent://acme.example/console/box\",\
             \"data\":{\"action\":\"console.kill_run\",\"target\":\"run-1\",\
             \"decision\":\"allow\",\"sig_alg\":\"es256\",\
             \"sig_fpr\":\"software-signed\",\"http_status\":200,\
             \"verify_result\":\"killed:true\"}}\n",
        )
        .expect("write the console events file");

        pick_up_new_sources(&mut ingest, &dir, &mut tailed);
        ingest.poll_once().expect("poll after rescan");

        assert!(
            tailed.contains(&console),
            "the console's file must be tailed once it exists: {tailed:?}"
        );
        assert_eq!(
            ingest.store().event_count().expect("event_count"),
            generated as u64 + 1,
            "the console_command must reach the store"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
