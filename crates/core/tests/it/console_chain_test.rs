//! The console's own hash chain, under the two conditions that actually
//! occur on a live box: another product appending to the same file, and two
//! privileged console actions landing at once.
//!
//! Both were broken in the same way. `command::record` used to derive its
//! `prev_hash` by re-reading the last line of the file immediately before
//! appending, so ANY line that arrived in between (a product's, or another
//! console command's) became the link target. A console chain is the one
//! thing an auditor most needs to trust, and it forked every time.
//!
//! The fix is the shape the estate already implements twice (TokenFuse's
//! `Exporter`, heraldyx's `internal/record`): one writer per file, a
//! process-wide sink seeded from the file tail ONCE at open, advancing its
//! next hash in memory, and framing the line with its newline in a single
//! write.

use genaryx_core::command::chain_hash_of_line;
use genaryx_core::store::Store;
use genaryx_core::{CommandRecord, record};
use serde_json::json;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A fresh directory of this test's own. Cargo runs these on parallel
/// threads in one process, and the sink registry is process-wide and keyed
/// by path, so two tests sharing a directory would share a sink.
fn unique_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-console-chain-{tag}-{}-{n}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn kill(target: &str) -> CommandRecord {
    CommandRecord {
        operator: "user://acme.example/alice".to_string(),
        env: "test".to_string(),
        action: "console.kill_run".to_string(),
        target: target.to_string(),
        params: json!({}),
        decision: "allow".to_string(),
        sig_alg: "es256".to_string(),
        sig_fpr: "software-signed".to_string(),
        http_status: 200,
        verify_result: "killed:true".to_string(),
    }
}

/// One product line, written the way TokenFuse's exporter writes: the line
/// and its newline in a single `write_all`, through an O_APPEND handle.
fn foreign_append(path: &Path, n: u64) {
    let line = format!(
        r#"{{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-08-06T10:00:00.000Z","source":"tokenfuse","type":"tool_call","severity":"low","agent_id":"agent://acme.example/bot/{n}","data":{{}}}}"#
    );
    let mut framed = line.into_bytes();
    framed.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open the events file for append");
    file.write_all(&framed).expect("foreign append");
}

/// Every `source:"console"` line in `path`, in file order.
fn console_lines(path: &Path) -> Vec<String> {
    let body = std::fs::read_to_string(path).expect("read the events file");
    body.lines()
        .filter(|l| !l.trim().is_empty())
        .filter(|l| {
            serde_json::from_str::<serde_json::Value>(l)
                .ok()
                .and_then(|v| {
                    v.get("source")
                        .and_then(|s| s.as_str())
                        .map(|s| s == "console")
                })
                .unwrap_or(false)
        })
        .map(str::to_string)
        .collect()
}

/// Assert that `lines` form one unbroken chain: each line's `prev_hash` is
/// the chain hash of the line before it.
fn assert_one_chain(lines: &[String]) {
    for i in 1..lines.len() {
        let carried: Option<String> = serde_json::from_str::<serde_json::Value>(&lines[i])
            .expect("a console line must parse")
            .get("prev_hash")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let expected = chain_hash_of_line(&lines[i - 1]);
        assert_eq!(
            carried,
            expected,
            "console line {i} must link to console line {}, not to whatever else \
             happened to be written in between.\n  line {}: {}\n  line {i}: {}",
            i - 1,
            i - 1,
            lines[i - 1],
            lines[i]
        );
    }
}

/// The deterministic case, and the reason this is not filed as a race: one
/// foreign line appended between two console commands is enough. The second
/// console command must still link to the first console command.
#[test]
fn a_foreign_append_between_two_console_commands_does_not_fork_the_console_chain() {
    let dir = unique_dir("foreign-between");
    let events = dir.join("console.ndjson");
    let store = Store::open(&dir.join("console.sqlite")).expect("open store");

    record(&store, &events, "acme.example", "host", &kill("run-1")).expect("first console command");
    foreign_append(&events, 1);
    record(&store, &events, "acme.example", "host", &kill("run-2"))
        .expect("second console command");

    let lines = console_lines(&events);
    assert_eq!(lines.len(), 2, "two console commands were recorded");
    assert_one_chain(&lines);

    let _ = std::fs::remove_dir_all(&dir);
}

/// The console's chain survives a product writing into the same file
/// throughout, and two console commands landing at once.
#[test]
fn concurrent_console_commands_and_a_foreign_writer_leave_one_console_chain() {
    const CONSOLE_THREADS: usize = 4;
    const PER_THREAD: usize = 25;
    const FOREIGN_THREADS: usize = 4;
    const FOREIGN_WRITES: u64 = 400;

    let dir = unique_dir("concurrent");
    let events = dir.join("console.ndjson");
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let foreign: Vec<_> = (0..FOREIGN_THREADS)
        .map(|_| {
            let events = events.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                for n in 0..FOREIGN_WRITES {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    foreign_append(&events, n);
                }
            })
        })
        .collect();

    let writers: Vec<_> = (0..CONSOLE_THREADS)
        .map(|t| {
            let events = events.clone();
            let dir = dir.clone();
            std::thread::spawn(move || {
                // A `Store` wraps a rusqlite `Connection`, which is `Send` but
                // not `Sync`, so each thread owns its own. The events file is
                // what this test is about; the journal database is not.
                let store =
                    Store::open(&dir.join(format!("console-{t}.sqlite"))).expect("open store");
                for i in 0..PER_THREAD {
                    record(
                        &store,
                        &events,
                        "acme.example",
                        "host",
                        &kill(&format!("run-{t}-{i}")),
                    )
                    .expect("record must not fail under concurrency");
                }
            })
        })
        .collect();

    for w in writers {
        w.join().expect("console writer thread");
    }
    stop.store(true, Ordering::Relaxed);
    for f in foreign {
        f.join().expect("foreign writer thread");
    }

    let lines = console_lines(&events);
    assert_eq!(
        lines.len(),
        CONSOLE_THREADS * PER_THREAD,
        "every console command must appear exactly once"
    );
    assert_one_chain(&lines);

    let _ = std::fs::remove_dir_all(&dir);
}

/// A console line and its newline reach the file together. Appending them as
/// two writes lets a concurrent O_APPEND write land in between, which
/// produces one line that is two half-events and parses as neither.
#[test]
fn a_console_line_and_its_newline_are_never_split_by_another_writer() {
    const CONSOLE_THREADS: usize = 4;
    const PER_THREAD: usize = 25;
    const FOREIGN_THREADS: usize = 4;
    const FOREIGN_WRITES: u64 = 400;

    let dir = unique_dir("framing");
    let events = dir.join("console.ndjson");
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let foreign: Vec<_> = (0..FOREIGN_THREADS)
        .map(|_| {
            let events = events.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                for n in 0..FOREIGN_WRITES {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    foreign_append(&events, n);
                }
            })
        })
        .collect();

    let writers: Vec<_> = (0..CONSOLE_THREADS)
        .map(|t| {
            let events = events.clone();
            let dir = dir.clone();
            std::thread::spawn(move || {
                let store =
                    Store::open(&dir.join(format!("console-{t}.sqlite"))).expect("open store");
                for i in 0..PER_THREAD {
                    record(
                        &store,
                        &events,
                        "acme.example",
                        "host",
                        &kill(&format!("run-{t}-{i}")),
                    )
                    .expect("record must not fail under concurrency");
                }
            })
        })
        .collect();

    for w in writers {
        w.join().expect("console writer thread");
    }
    stop.store(true, Ordering::Relaxed);
    for f in foreign {
        f.join().expect("foreign writer thread");
    }

    let body = std::fs::read_to_string(&events).expect("read the events file");
    let malformed: Vec<&str> = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter(|l| serde_json::from_str::<serde_json::Value>(l).is_err())
        .collect();
    assert!(
        malformed.is_empty(),
        "{} line(s) in the events file are not valid JSON, which means a write \
         was interleaved with another writer's. First: {}",
        malformed.len(),
        malformed[0]
    );

    let _ = std::fs::remove_dir_all(&dir);
}
