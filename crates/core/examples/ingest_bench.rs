//! Phase-0 spike #3 (06 §7, docs/PHASE0.md spike row 3): ingest throughput
//! bench for the conform + Store insert path. Target: >= 50,000 NDJSON lines
//! per minute, sustained, on this box.
//!
//! Run: `cargo run --release --example ingest_bench -p genaryx-core`
//!
//! Builds a large corpus by repeating `demo::generate`'s own output (already
//! proven conforming line-by-line in `tests/demo_test.rs`) up to a fixed
//! target line count, then times two stages with `std::time::Instant`:
//!
//!   1. conform-only: `Conformer::parse_valid` over every line, nothing else.
//!   2. end-to-end ingest: conform, build a `ConsoleEvent`, then
//!      `Store::insert_batch` in ~1000-line batches against a fresh
//!      in-memory `Store`. This replays the same conform/insert sequence
//!      `IngestService::poll_once` runs (see `src/ingest.rs`), directly
//!      against the in-memory corpus rather than through `IngestService`
//!      itself: the spike is specifically an SQLite ingest bench (docs/PHASE0.md
//!      row 3 title), so file-tailing IO is deliberately kept out of the
//!      timed path.
//!
//! No `criterion`, no new dependencies, no `rand`. The corpus is a
//! deterministic repetition of `demo::generate`'s deterministic output, and a
//! fixed `received_ts` stands in for the real path's `Utc::now()` read, so
//! the timed loop has no wall-clock read in it and a rerun on this box
//! exercises the same bytes again.
//!
//! `Store::insert_batch` has no uniqueness constraint on event content, so
//! repeating identical lines triggers no dedup path in this schema. Nothing
//! here varies the repeated lines: being honest about what is measured
//! matters more than guarding against a dedup effect that does not exist in
//! this code.

use genaryx_core::event::{AgentEvent, ConsoleEvent, Provenance};
use genaryx_core::store::Store;
use genaryx_core::{Conformer, demo};
use std::path::Path;
use std::time::{Duration, Instant};

/// Minimum corpus size (spec: "cycle them to reach >= 200,000 lines").
const TARGET_LINES: usize = 200_000;

/// Lines per `Store::insert_batch` call in the end-to-end stage (spec:
/// "batches of ~1000").
const INSERT_BATCH_SIZE: usize = 1_000;

/// Lines run once through conform plus a throwaway store before either timed
/// stage starts, so first-touch allocation, page faults, and one-time lazy
/// init (the compiled jsonschema validators, SQLite's own setup) are not
/// charged to the measured run.
const WARMUP_LINES: usize = 5_000;

/// Phase-0 spike #3 target (06 §7): 50,000 NDJSON lines per minute.
const TARGET_LINES_PER_MINUTE: f64 = 50_000.0;

/// The six emitting services `demo::generate` writes (mirrors the private
/// `SOURCES` list in `src/demo.rs`; idryx is deliberately absent there, see
/// that module's docs).
const SOURCES: [&str; 6] = [
    "tokenfuse",
    "wardryx",
    "engram",
    "verdryx",
    "mockryx",
    "qryx",
];

fn main() {
    let events_dir =
        std::env::temp_dir().join(format!("genaryx-ingest-bench-{}", std::process::id()));
    let demo_total = demo::generate(&events_dir)
        .unwrap_or_else(|e| panic!("demo::generate into {}: {e}", events_dir.display()));

    let base_lines = read_all_lines(&events_dir);
    assert_eq!(
        base_lines.len(),
        demo_total,
        "lines read back from disk must match demo::generate's own returned count"
    );
    println!(
        "base corpus: {} conforming lines from demo::generate, across {} files",
        base_lines.len(),
        SOURCES.len()
    );

    let corpus = cycle_to_at_least(&base_lines, TARGET_LINES);
    println!(
        "bench corpus: {} lines (target >= {TARGET_LINES}, exact whole repeats of the base corpus)",
        corpus.len()
    );

    let conformer = Conformer::new().unwrap_or_else(|e| panic!("Conformer::new: {e}"));

    let warmup_n = WARMUP_LINES.min(corpus.len());
    warm_up(&conformer, &corpus[..warmup_n]);
    println!("warm-up: {warmup_n} lines through conform + a throwaway store (not timed)");

    let conform_elapsed = bench_conform_only(&conformer, &corpus);
    let ingest_elapsed = bench_end_to_end(&conformer, &corpus);

    let _ = std::fs::remove_dir_all(&events_dir);

    println!();
    report("conform-only", corpus.len(), conform_elapsed);
    let ingest_lines_per_min = report("end-to-end ingest", corpus.len(), ingest_elapsed);

    let verdict = if ingest_lines_per_min >= TARGET_LINES_PER_MINUTE {
        "PASS"
    } else {
        "FAIL"
    };
    println!(
        "\nOverall verdict ({verdict}): end-to-end ingest {ingest_lines_per_min:.0} lines/min \
         vs {TARGET_LINES_PER_MINUTE:.0} lines/min target (the SQLite insert path is the \
         bottleneck this spike is asking about)."
    );
}

/// Read every non-empty line from every `{source}.ndjson` file
/// `demo::generate` wrote into `dir`, in a fixed file order, so the base
/// corpus itself is deterministic.
fn read_all_lines(dir: &Path) -> Vec<String> {
    let mut lines = Vec::new();
    for source in SOURCES {
        let path = dir.join(format!("{source}.ndjson"));
        let body = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        lines.extend(
            body.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.to_string()),
        );
    }
    lines
}

/// Repeat `base` a whole copy at a time until the result has at least
/// `target` lines. The last copy is never truncated, so every line stays a
/// complete, independently-valid NDJSON record; the final count can land
/// slightly above `target`, which is printed, never silently rounded down.
fn cycle_to_at_least(base: &[String], target: usize) -> Vec<String> {
    assert!(!base.is_empty(), "base corpus must be non-empty");
    let mut out = Vec::with_capacity(target + base.len());
    while out.len() < target {
        out.extend(base.iter().cloned());
    }
    out
}

/// Run `lines` once through conform + `Store::insert_batch` on a throwaway
/// in-memory store, discarding the store afterward. Not timed; see
/// `WARMUP_LINES`.
fn warm_up(conformer: &Conformer, lines: &[String]) {
    let store = Store::open_in_memory().unwrap_or_else(|e| panic!("warm-up store: {e}"));
    let mut batch: Vec<ConsoleEvent> = Vec::with_capacity(INSERT_BATCH_SIZE);
    for line in lines {
        let event = conformer.parse_valid(line).unwrap_or_else(|report| {
            panic!(
                "warm-up: expected a conforming line, got: {:?}",
                report.errors
            )
        });
        batch.push(to_console_event(event, line));
        if batch.len() >= INSERT_BATCH_SIZE {
            store
                .insert_batch(&batch)
                .unwrap_or_else(|e| panic!("warm-up insert_batch: {e}"));
            batch.clear();
        }
    }
    if !batch.is_empty() {
        store
            .insert_batch(&batch)
            .unwrap_or_else(|e| panic!("warm-up insert_batch (final partial batch): {e}"));
    }
}

/// Stage 1: `Conformer` alone, over every line in `corpus`. Every line must
/// parse valid: the corpus is entirely repeated `demo::generate` output,
/// already proven conforming by `tests/demo_test.rs`. A failure here would
/// mean the corpus itself regressed, so it is a hard panic, not a skipped
/// line.
fn bench_conform_only(conformer: &Conformer, corpus: &[String]) -> Duration {
    let start = Instant::now();
    let mut valid = 0usize;
    for line in corpus {
        match conformer.parse_valid(line) {
            Ok(_) => valid += 1,
            Err(report) => panic!("expected a conforming demo line, got: {:?}", report.errors),
        }
    }
    let elapsed = start.elapsed();
    assert_eq!(valid, corpus.len(), "every corpus line must conform");
    elapsed
}

/// Stage 2: the real path, conform, build a `ConsoleEvent`, then
/// `Store::insert_batch`, in `INSERT_BATCH_SIZE` chunks against a fresh
/// in-memory store. Mirrors the conform/insert sequence
/// `IngestService::poll_once` runs (`src/ingest.rs`), replayed directly
/// against the corpus rather than through a file tail.
fn bench_end_to_end(conformer: &Conformer, corpus: &[String]) -> Duration {
    let store = Store::open_in_memory().unwrap_or_else(|e| panic!("bench store: {e}"));
    let start = Instant::now();

    let mut batch: Vec<ConsoleEvent> = Vec::with_capacity(INSERT_BATCH_SIZE);
    let mut inserted = 0usize;
    for line in corpus {
        let event = conformer.parse_valid(line).unwrap_or_else(|report| {
            panic!("expected a conforming demo line, got: {:?}", report.errors)
        });
        batch.push(to_console_event(event, line));
        if batch.len() >= INSERT_BATCH_SIZE {
            inserted += store
                .insert_batch(&batch)
                .unwrap_or_else(|e| panic!("insert_batch: {e}"));
            batch.clear();
        }
    }
    if !batch.is_empty() {
        inserted += store
            .insert_batch(&batch)
            .unwrap_or_else(|e| panic!("insert_batch (final partial batch): {e}"));
    }

    let elapsed = start.elapsed();
    assert_eq!(
        inserted,
        corpus.len(),
        "every corpus line must have been inserted"
    );
    let stored = store
        .event_count()
        .unwrap_or_else(|e| panic!("event_count: {e}"));
    assert_eq!(stored, corpus.len() as u64, "Store's own count must match");
    elapsed
}

/// Wrap a conformed `AgentEvent` in the `ConsoleEvent` envelope the real
/// ingest path builds (see `IngestService::poll_once`). Provenance is
/// bench-labeled; `received_ts` is a fixed constant rather than `Utc::now()`,
/// so the timed loop has no wall-clock read in it.
fn to_console_event(event: AgentEvent, raw: &str) -> ConsoleEvent {
    let schema_version = match event.schema_version() {
        Some(v) => v,
        None => panic!(
            "a conforming event must resolve a schema version, got schema: {:?}",
            event.schema
        ),
    };
    ConsoleEvent {
        event,
        provenance: Provenance {
            env: "bench".to_string(),
            connector: "ingest_bench".to_string(),
            file: None,
            offset: None,
            endpoint: None,
            received_ts: "2026-07-16T00:00:00.000Z".to_string(),
        },
        raw: raw.to_string(),
        schema_version,
    }
}

/// Print one stage's throughput report and return its lines/minute.
fn report(label: &str, n: usize, elapsed: Duration) -> f64 {
    let secs = elapsed.as_secs_f64();
    let lines_per_sec = n as f64 / secs;
    let lines_per_min = lines_per_sec * 60.0;
    let verdict = if lines_per_min >= TARGET_LINES_PER_MINUTE {
        "PASS"
    } else {
        "FAIL"
    };
    println!(
        "{label}: {n} lines in {secs:.3}s => {lines_per_sec:.0} lines/sec, \
         {lines_per_min:.0} lines/min ({verdict} vs {TARGET_LINES_PER_MINUTE:.0}/min target)"
    );
    lines_per_min
}
