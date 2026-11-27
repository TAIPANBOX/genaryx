//! `FileTail` and `IngestService` at their edges: a line is emitted only once
//! its newline arrives, a missing file is "nothing yet" while a file that
//! cannot be read is an error, a stopped loop polls nothing, and a pinned
//! stream refuses a line that claims another source.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use genaryx_core::ingest::PINNED_SOURCE_REFUSED;
use genaryx_core::store::Store;
use genaryx_core::{EventSource, FileTail, IngestService};

const LINE_A: &str = r#"{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-11-01T00:00:00Z","source":"wardryx","type":"policy_deny","agent_id":"agent://acme.example/bot"}"#;
const LINE_B: &str = r#"{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-11-01T00:00:01Z","source":"tokenfuse","type":"budget_exhausted","agent_id":"agent://acme.example/bot"}"#;

fn scratch(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-ingest-edges-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn a_partial_line_is_withheld_until_its_newline_arrives() {
    let dir = scratch("partial");
    let path = dir.join("wardryx.ndjson");
    std::fs::write(&path, format!("{LINE_A}\n{}", &LINE_B[..20])).expect("write");
    let mut tail = FileTail::new("filetail:wardryx", &path, 0);
    assert_eq!(tail.id(), "filetail:wardryx");

    let first = tail.poll().expect("poll");
    assert_eq!(first.len(), 1, "only the complete line");
    assert_eq!(first[0].raw, LINE_A);
    assert_eq!(first[0].offset, Some(0));
    let key = path.to_string_lossy().into_owned();
    assert_eq!(first[0].file.as_deref(), Some(key.as_str()));
    assert!(tail.poll().expect("poll").is_empty(), "the half line waits");

    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open");
    f.write_all(format!("{}\n", &LINE_B[20..]).as_bytes())
        .expect("append");
    let second = tail.poll().expect("poll");
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].raw, LINE_B, "the line is whole once completed");
    assert_eq!(second[0].offset, Some(LINE_A.len() as u64 + 1));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_missing_file_is_nothing_yet_but_an_unreadable_one_is_an_error() {
    let dir = scratch("missing");
    let mut missing = FileTail::new("t", dir.join("not-yet.ndjson"), 0);
    assert!(missing.poll().expect("not an error").is_empty());

    // A directory opens on some platforms and fails on read, on others fails
    // to open; either way it is never read as an empty stream. It holds a
    // file so no filesystem reports its size as zero.
    std::fs::write(dir.join("present.ndjson"), "x\n").expect("write");
    let mut dir_tail = FileTail::new("t", &dir, 0);
    assert!(
        dir_tail.poll().is_err(),
        "a directory is not an event stream"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_stopped_loop_returns_without_polling() {
    let dir = scratch("stopped");
    let path = dir.join("wardryx.ndjson");
    std::fs::write(&path, format!("{LINE_A}\n")).expect("write");
    let mut svc =
        IngestService::new(Store::open_in_memory().expect("store"), "local").expect("service");
    svc.add_file_source("filetail:wardryx", &path)
        .expect("source");
    let stop = Arc::new(AtomicBool::new(true));
    svc.run_blocking(Duration::from_millis(1), stop)
        .expect("a stopped loop is not an error");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_loop_ingests_until_it_is_told_to_stop() {
    let dir = scratch("loop");
    let path = dir.join("wardryx.ndjson");
    std::fs::write(&path, format!("{LINE_A}\n")).expect("write");
    let db = dir.join("console.sqlite");
    let mut svc = IngestService::new(Store::open(&db).expect("store"), "local").expect("service");
    svc.add_file_source("filetail:wardryx", &path)
        .expect("source");
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let reader = Store::open(&db).expect("a second reader");
    // The watcher stops the loop once the line is stored, or after about two
    // seconds whatever happened, so a loop that stores nothing fails below
    // rather than hanging.
    let watcher = std::thread::spawn(move || {
        let mut stored = 0;
        for _ in 0..400 {
            stored = reader.event_count().unwrap_or(0);
            if stored > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        flag.store(true, Ordering::Relaxed);
        stored
    });
    svc.run_blocking(Duration::from_millis(5), stop)
        .expect("the loop ends cleanly");
    let stored = watcher.join().expect("watcher thread");
    assert_eq!(stored, 1, "the running loop stored the line");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_pinned_stream_quarantines_a_line_claiming_another_source() {
    let dir = scratch("pinned");
    let path = dir.join("wardryx.ndjson");
    std::fs::write(&path, format!("{LINE_A}\n{LINE_B}\n")).expect("write");
    let mut svc =
        IngestService::new(Store::open_in_memory().expect("store"), "local").expect("service");
    svc.add_pinned_file_source("filetail:wardryx", &path, "wardryx")
        .expect("source");
    let stats = svc.poll_once().expect("poll");
    assert_eq!((stats.inserted, stats.quarantined), (1, 1));
    let reasons = svc.store().quarantine_by_reason(10).expect("reasons");
    assert_eq!(reasons.len(), 1);
    assert_eq!(reasons[0].reason, PINNED_SOURCE_REFUSED);
    std::fs::remove_dir_all(&dir).ok();
}
