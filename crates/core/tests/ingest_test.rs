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
