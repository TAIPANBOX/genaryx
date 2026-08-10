//! Ingest tests: `FileTail` -> conform -> `Store` -> broadcast, end to end
//! through `IngestService`. Covers the offset journal (only newly appended
//! bytes are reprocessed), quarantine of malformed/non-conforming lines, the
//! live broadcast channel, and resilience to truncation (07 §3).

use genaryx_core::IngestService;
use genaryx_core::store::Store;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// 5 valid canonical-style lines: the closed v0.1 source enum for the first
/// 4, the open v0.2 source string for the 5th (mirrors `fixtures/canonical.ndjson`).
const VALID_5: &str = r#"{"schema":"taipanbox.dev/agent-event/v0.1","ts":"2026-07-16T00:00:00Z","source":"tokenfuse","type":"budget_exhausted","agent_id":"agent://acme.example/support/bot"}
{"schema":"taipanbox.dev/agent-event/v0.1","ts":"2026-07-16T00:00:01Z","source":"engram","type":"contradiction_found","agent_id":"agent://acme.example/support/bot"}
{"schema":"taipanbox.dev/agent-event/v0.1","ts":"2026-07-16T00:00:02Z","source":"idryx","type":"attestation_missing","agent_id":"agent://acme.example/eng/ci-fixer"}
{"schema":"taipanbox.dev/agent-event/v0.1","ts":"2026-07-16T00:00:03Z","source":"qryx","type":"evidence_signed","agent_id":"agent://acme.example/support/bot"}
{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-07-16T00:00:04Z","source":"wardryx","type":"policy_deny","agent_id":"agent://acme.example/eng/ci-fixer"}
"#;

/// 2 bad lines: malformed JSON, then valid JSON with a non-conforming
/// `agent_id` (no `agent://` prefix, uppercase).
const BAD_2: &str = r#"{nope
{"schema":"taipanbox.dev/agent-event/v0.1","ts":"2026-07-16T00:00:05Z","source":"tokenfuse","type":"budget_exhausted","agent_id":"NOPE"}
"#;

/// 3 more valid lines, appended after the first poll to prove the offset
/// journal only reprocesses genuinely new bytes.
const APPEND_3: &str = r#"{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-07-16T00:00:06Z","source":"verdryx","type":"quality_drift","agent_id":"agent://acme.example/support/bot"}
{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-07-16T00:00:07Z","source":"mockryx","type":"blast_radius_measured","agent_id":"agent://acme.example/eng/ci-fixer"}
{"schema":"taipanbox.dev/agent-event/v0.1","ts":"2026-07-16T00:00:08Z","source":"tokenfuse","type":"budget_exhausted","agent_id":"agent://acme.example/support/bot"}
"#;

/// 8 lines that share NOTHING with `VALID_5`, and together are longer than it.
///
/// Used for the rotation test, and the "share nothing" is the whole point: a
/// replacement that happens to begin with the same bytes is indistinguishable
/// from an append, so a test built on one proves nothing about detecting the
/// replacement.
const REPLACEMENT_8: &str = r#"{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-07-17T00:00:00Z","source":"wardryx","type":"policy_deny","agent_id":"agent://acme.example/ops/rotator-a"}
{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-07-17T00:00:01Z","source":"wardryx","type":"policy_deny","agent_id":"agent://acme.example/ops/rotator-b"}
{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-07-17T00:00:02Z","source":"wardryx","type":"policy_deny","agent_id":"agent://acme.example/ops/rotator-c"}
{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-07-17T00:00:03Z","source":"wardryx","type":"policy_deny","agent_id":"agent://acme.example/ops/rotator-d"}
{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-07-17T00:00:04Z","source":"wardryx","type":"policy_deny","agent_id":"agent://acme.example/ops/rotator-e"}
{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-07-17T00:00:05Z","source":"wardryx","type":"policy_deny","agent_id":"agent://acme.example/ops/rotator-f"}
{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-07-17T00:00:06Z","source":"wardryx","type":"policy_deny","agent_id":"agent://acme.example/ops/rotator-g"}
{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-07-17T00:00:07Z","source":"wardryx","type":"policy_deny","agent_id":"agent://acme.example/ops/rotator-h"}
"#;

/// A single valid line: used to overwrite the test file with something
/// shorter than the previously journaled offset (truncation resilience).
const SHORT_VALID_LINE: &str = r#"{"schema":"taipanbox.dev/agent-event/v0.1","ts":"2026-07-16T00:00:09Z","source":"tokenfuse","type":"budget_exhausted","agent_id":"agent://acme.example/support/bot"}
"#;

/// A fresh, uniquely-named NDJSON path under the system temp dir, so
/// concurrently-running tests never collide (a static counter plus the
/// process id disambiguates every call).
fn unique_ndjson_path(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-ingest-test-{tag}-{}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp test dir");
    dir.join("events.ndjson")
}

/// The inode of `path`, so a test can assert it did NOT change and prove the
/// content fingerprint is what detected the rewrite.
#[cfg(unix)]
fn inode_of(path: &std::path::Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).expect("stat").ino()
}

#[cfg(not(unix))]
fn inode_of(_path: &std::path::Path) -> u64 {
    0
}

fn write_new(path: &std::path::Path, content: &str) {
    std::fs::write(path, content).expect("write test ndjson file");
}

fn append(path: &std::path::Path, content: &str) {
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open test ndjson file for append");
    f.write_all(content.as_bytes())
        .expect("append to test ndjson file");
}

#[test]
fn poll_once_conforms_quarantines_broadcasts_and_journals_offset() {
    let path = unique_ndjson_path("lifecycle");
    write_new(&path, &format!("{VALID_5}{BAD_2}"));

    let store = Store::open_in_memory().expect("open in-memory store");
    let mut svc = IngestService::new(store, "test").expect("new IngestService");

    // Subscribe before the first poll, so the live broadcast of that poll's
    // batch is not missed.
    let mut rx = svc.subscribe();

    svc.add_file_source("filetail:test", &path)
        .expect("add_file_source");

    let stats = svc.poll_once().expect("poll_once");
    assert_eq!(stats.inserted, 5, "5 of the 7 lines conform");
    assert_eq!(
        stats.quarantined, 2,
        "the malformed-json line and the bad-agent_id line"
    );
    assert_eq!(svc.store().event_count().expect("event_count"), 5);
    assert_eq!(svc.store().quarantine_count().expect("quarantine_count"), 2);

    let mut received = 0usize;
    while rx.try_recv().is_ok() {
        received += 1;
    }
    assert_eq!(received, 5, "every valid event is broadcast live");

    // Append 3 more valid lines to the same file; only the new lines should
    // be processed this cycle, proving the offset journal skips what was
    // already ingested rather than reprocessing the whole file.
    append(&path, APPEND_3);
    let stats = svc.poll_once().expect("poll_once after append");
    assert_eq!(stats.inserted, 3, "only the 3 newly appended lines");
    assert_eq!(stats.quarantined, 0);
    assert_eq!(svc.store().event_count().expect("event_count"), 8);
}

#[test]
fn poll_once_reingests_from_top_after_truncation() {
    let path = unique_ndjson_path("truncation");
    write_new(&path, &format!("{VALID_5}{BAD_2}"));

    let store = Store::open_in_memory().expect("open in-memory store");
    let mut svc = IngestService::new(store, "test").expect("new IngestService");
    svc.add_file_source("filetail:test", &path)
        .expect("add_file_source");

    let stats = svc.poll_once().expect("initial poll_once");
    assert_eq!(stats.inserted, 5);
    assert_eq!(stats.quarantined, 2);

    // Overwrite with a file shorter than the journaled offset: this must be
    // treated as rotation/truncation (07 §3) and re-read from the top, not
    // skipped and not treated as an error.
    write_new(&path, SHORT_VALID_LINE);
    let stats = svc.poll_once().expect("poll_once after truncation");
    assert_eq!(
        stats.inserted, 1,
        "re-ingested the single line from offset 0"
    );
    assert_eq!(stats.quarantined, 0);
    assert_eq!(
        svc.store().event_count().expect("event_count"),
        6,
        "the 5 events from before truncation plus the 1 re-ingested"
    );
}

// ---------------------------------------------------------------------------
// Durable history: what a store that outlives its process has to survive.
// ---------------------------------------------------------------------------

/// A console restarted against the same store resumes where it left off and
/// does not re-read what it already holds.
///
/// This is the property the whole durable store rests on. Before it, the store
/// died with the process, so resuming was never tested by anything and the
/// offset journal was write-only in practice.
#[test]
fn a_restart_against_the_same_store_resumes_instead_of_re_reading() {
    let path = unique_ndjson_path("restart-resume");
    let db = path.with_extension("sqlite");
    write_new(&path, VALID_5);

    {
        let store = Store::open(&db).expect("open store");
        let mut svc = IngestService::new(store, "test").expect("new IngestService");
        svc.add_file_source("filetail:test", &path)
            .expect("add_file_source");
        assert_eq!(svc.poll_once().expect("first run").inserted, 5);
    }

    // The console stops and starts. Same file, same store, nothing new written.
    {
        let store = Store::open(&db).expect("reopen store");
        let mut svc = IngestService::new(store, "test").expect("new IngestService");
        svc.add_file_source("filetail:test", &path)
            .expect("add_file_source");
        assert_eq!(
            svc.poll_once().expect("second run").inserted,
            0,
            "a restart with nothing new to read learns nothing new"
        );
        assert_eq!(
            svc.store().event_count().expect("event_count"),
            5,
            "and the history is still one copy of each event"
        );
    }

    // Now something IS appended while the console is up again.
    {
        append(&path, APPEND_3);
        let store = Store::open(&db).expect("reopen store");
        let mut svc = IngestService::new(store, "test").expect("new IngestService");
        svc.add_file_source("filetail:test", &path)
            .expect("add_file_source");
        assert_eq!(svc.poll_once().expect("third run").inserted, 3);
        assert_eq!(svc.store().event_count().expect("event_count"), 8);
    }

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&db);
}

/// `stack-up` truncates its event files on every start. Against a durable
/// store the tail re-reads from the top, and the dedupe key is what stops that
/// becoming a second copy of every line the file still holds.
///
/// This is the exact scenario the old scratch-store note named as the reason
/// durable history could not be turned on.
#[test]
fn a_truncation_that_rewrites_the_same_lines_does_not_duplicate_them() {
    let path = unique_ndjson_path("truncate-rewrite");
    let db = path.with_extension("sqlite");
    write_new(&path, &format!("{VALID_5}{APPEND_3}"));

    {
        let store = Store::open(&db).expect("open store");
        let mut svc = IngestService::new(store, "test").expect("new IngestService");
        svc.add_file_source("filetail:test", &path)
            .expect("add_file_source");
        assert_eq!(svc.poll_once().expect("first run").inserted, 8);
    }

    // Truncated back to its first five lines, byte for byte, exactly as a
    // restarted producer replaying its own state would leave it.
    write_new(&path, VALID_5);

    {
        let store = Store::open(&db).expect("reopen store");
        let mut svc = IngestService::new(store, "test").expect("new IngestService");
        svc.add_file_source("filetail:test", &path)
            .expect("add_file_source");
        assert_eq!(
            svc.poll_once().expect("after truncation").inserted,
            0,
            "the same bytes at the same offsets are the same events"
        );
        assert_eq!(
            svc.store().event_count().expect("event_count"),
            8,
            "history keeps the three lines the truncation dropped, and no duplicates"
        );
    }

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&db);
}

/// A file whose CONTENT was replaced while the console was down gets re-read
/// from the top, even when the replacement is longer than the journaled offset
/// and even when it is the same file on disk.
///
/// Three things have to line up for this to be missed, and they do line up in
/// the ordinary `stack-up` restart: the tail's own shorter-than-my-offset check
/// does not fire (the new content is LONGER), the inode is unchanged (the file
/// was rewritten in place, and on Linux even a delete-and-recreate usually
/// reuses the number), and so a naive resume seeks into the middle of content
/// it has never read. Everything before that point is then lost, permanently
/// and silently.
///
/// Rewriting in place is deliberate here rather than `remove` + `create`: it
/// pins the inode identical on every platform, so the test proves the
/// fingerprint is doing the work instead of depending on how the filesystem
/// happens to allocate.
#[test]
fn a_file_rewritten_while_down_is_re_read_from_the_top() {
    let path = unique_ndjson_path("replaced");
    let db = path.with_extension("sqlite");
    write_new(&path, VALID_5);

    {
        let store = Store::open(&db).expect("open store");
        let mut svc = IngestService::new(store, "test").expect("new IngestService");
        svc.add_file_source("filetail:test", &path)
            .expect("add_file_source");
        assert_eq!(svc.poll_once().expect("first run").inserted, 5);
    }

    // Rewritten IN PLACE: same inode, longer than before, sharing none of the
    // old bytes. Sharing no bytes is what makes this a test at all; a
    // replacement that began with the same content would be read correctly by
    // a plain resume and would prove nothing.
    let inode_before = inode_of(&path);
    write_new(&path, REPLACEMENT_8);
    assert_eq!(
        inode_of(&path),
        inode_before,
        "rewriting in place must keep the inode, so this test cannot pass by \
         accident on a filesystem that hands out a fresh one"
    );
    assert!(
        REPLACEMENT_8.len() > VALID_5.len(),
        "the replacement must be LONGER, which is the case FileTail's own \
         shorter-than-my-offset check cannot see"
    );

    {
        let store = Store::open(&db).expect("reopen store");
        let mut svc = IngestService::new(store, "test").expect("new IngestService");
        svc.add_file_source("filetail:test", &path)
            .expect("add_file_source");
        let stats = svc.poll_once().expect("after replacement");
        assert_eq!(
            stats.inserted, 8,
            "every line of the new file, not just the bytes past the old offset"
        );
        assert_eq!(
            stats.quarantined, 0,
            "and no partial line, which is what resuming mid-file would produce"
        );
        assert_eq!(svc.store().event_count().expect("event_count"), 13);
    }

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&db);
}
