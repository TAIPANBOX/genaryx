//! The live path: seeds `genaryx-core`'s real `Store` at startup from the demo
//! NDJSON fixtures, then keeps a single background thread feeding one new
//! conforming event into that same bus every ~2s and forwarding it to the
//! frontend, so the Bus Explorer updates without a reload (Phase-0 exit gate:
//! "both shells show the same live event stream from the shared core").
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

/// Tauri-managed state: where the seeded demo store lives, if startup
/// succeeded. `None` means [`crate::recent_events`] falls back to
/// `events::mock_events` (fail-closed: a seeding failure degrades the Bus
/// Explorer to mock data; it never crashes the app or traps the UI).
pub struct AppState {
    pub events_dir: Option<PathBuf>,
}

/// Tauri event name the frontend `listen()`s for; payload is one [`UiEvent`].
pub const LIVE_EVENT: &str = "bus:event";

/// Cadence of the background feeder tick (spec: "every ~2s").
const FEEDER_INTERVAL: Duration = Duration::from_secs(2);

/// Seed the real store from the demo fixtures and start the live feeder.
/// Returns the events directory (holding `console.sqlite` plus the six
/// `<source>.ndjson` files) on success; the caller (`lib.rs`'s `setup` hook)
/// degrades to mock data on error rather than failing app startup.
pub fn bootstrap(app_handle: AppHandle) -> genaryx_core::Result<PathBuf> {
    let events_dir = unique_events_dir();
    let generated = demo::generate(&events_dir)?;

    let db_path = events_dir.join("console.sqlite");
    let store = Store::open(&db_path)?;
    let mut ingest = IngestService::new(store, "local")?;

    let files = collect_ndjson_files(&events_dir)?;
    for path in &files {
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        ingest.add_file_source(format!("filetail:{id}"), path)?;
    }

    // Subscribe before the first poll so no batch is missed (per the
    // `IngestService::subscribe` doc comment). In practice the initial seed
    // batch is never forwarded live anyway, since `recent_events` reads it
    // straight from the Store; this only matters so the channel exists
    // before `poll_once` below could otherwise race a subscriber.
    let receiver = ingest.subscribe();
    let stats = ingest.poll_once()?;
    eprintln!(
        "genaryx: seeded demo store at {} ({generated} generated, {} inserted, {} quarantined)",
        events_dir.display(),
        stats.inserted,
        stats.quarantined
    );

    std::thread::spawn(move || run_feeder(ingest, receiver, files, app_handle));

    Ok(events_dir)
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

/// Every `*.ndjson` file `demo::generate` just wrote, so the feeder can
/// register a `FileTail` per source without hardcoding `demo`'s private
/// source list here (mirroring that private list, the way `events.rs`'s mock
/// data mirrors `demo`'s topic/eval/scenario lists, would drift silently if
/// `demo` ever added a source; reading the directory back cannot drift).
fn collect_ndjson_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("ndjson"))
        .collect();
    files.sort();
    Ok(files)
}

/// Owns the `IngestService` (and its `Store`) for the rest of the process:
/// every ~2s, append one conforming demo-shaped line to one of the seeded
/// NDJSON files, poll it into the Store like any other ingest cycle, then
/// forward whatever that poll just broadcast to the frontend. Never panics on
/// a transient failure; a bad tick is logged and the loop just tries again
/// next tick (fail-closed: the live feed degrading is never the whole app
/// crashing).
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
