//! Invariant 51. The fixture `testdata/heraldyx-sent.ndjson` was written by
//! heraldyx's own `internal/record` journal (origin/main, 2026-11-23): four
//! messages, one refused by the transport, and `heraldyx --journal` over it
//! printed "records: 4 (alert 2, digest 1, pulse 1)", "outcome: 3 accepted, 1
//! refused" and "chain: verifies (3 chained, 1 head(s))".

use super::*;

const REAL: &str = include_str!("testdata/heraldyx-sent.ndjson");

fn scratch(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!(
        "genaryx-notifications-{}-{name}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn write(name: &str, body: &str) -> PathBuf {
    let path = scratch(name).join("sent.ndjson");
    std::fs::write(&path, body).expect("write journal");
    path
}

fn read(body: &str) -> NotificationsPanel {
    notifications_sent(Some(&write("read", body)))
}

struct Read {
    records: Vec<SentRecord>,
    total: usize,
    accepted: usize,
    refused: usize,
    by_kind: BTreeMap<String, usize>,
    chain: ChainVerdict,
    window_truncated: bool,
    skipped: Vec<SkippedLine>,
    skipped_total: usize,
}

fn unwrap_read(p: NotificationsPanel) -> Read {
    match p {
        NotificationsPanel::Read {
            records,
            total,
            accepted,
            refused,
            by_kind,
            chain,
            window_truncated,
            skipped,
            skipped_total,
            ..
        } => Read {
            records,
            total,
            accepted,
            refused,
            by_kind,
            chain,
            window_truncated,
            skipped,
            skipped_total,
        },
        other => panic!("expected a read journal, got {other:?}"),
    }
}

#[test]
fn the_journal_setting_comes_from_its_own_variable() {
    assert_eq!(SENT_ENV, "GENARYX_HERALDYX_SENT");
    assert_eq!(sent_path_from(""), None);
    assert_eq!(sent_path_from("  \n"), None);
    assert_eq!(
        sent_path_from(" /var/lib/stack/heraldyx/sent.ndjson \n"),
        Some(PathBuf::from("/var/lib/stack/heraldyx/sent.ndjson"))
    );
}

#[test]
fn a_box_with_no_journal_named_says_so_and_names_the_setting() {
    assert_eq!(
        notifications_sent(None),
        NotificationsPanel::NotConfigured {
            variable: "GENARYX_HERALDYX_SENT".to_string()
        }
    );
}

#[test]
fn a_named_journal_that_is_not_there_is_unreadable_not_empty() {
    let path = scratch("absent").join("sent.ndjson");
    match notifications_sent(Some(&path)) {
        NotificationsPanel::Unreadable {
            path: shown,
            reason,
        } => {
            assert_eq!(shown, path.display().to_string());
            assert!(reason.contains("no file there"), "{reason}");
        }
        other => panic!("expected unreadable, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn a_journal_path_that_is_a_link_or_a_directory_is_not_read() {
    let real = write("target", REAL);
    let link = scratch("link").join("sent.ndjson");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");
    for path in [link, scratch("dir")] {
        match notifications_sent(Some(&path)) {
            NotificationsPanel::Unreadable { reason, .. } => {
                assert!(reason.contains("not a regular file"), "{reason}")
            }
            other => panic!("expected unreadable for {}, got {other:?}", path.display()),
        }
    }
}

#[test]
fn a_real_heraldyx_journal_reads_as_heraldyx_reads_it() {
    let r = unwrap_read(read(REAL));
    assert_eq!(r.total, 4);
    assert_eq!((r.accepted, r.refused), (3, 1));
    assert_eq!(r.by_kind.get("alert"), Some(&2));
    assert_eq!(r.by_kind.get("digest"), Some(&1));
    assert_eq!(r.by_kind.get("pulse"), Some(&1));
    assert_eq!(
        r.chain,
        ChainVerdict::Verifies {
            chained: 3,
            heads: 1,
            unverifiable: 0
        }
    );
    assert!(!r.window_truncated);
    assert_eq!(r.skipped_total, 0);
}

#[test]
fn records_come_newest_first_with_who_was_written_to_and_the_outcome() {
    let r = unwrap_read(read(REAL));
    assert_eq!(r.records[0].about, "pulse:2026-11-23");
    assert_eq!(r.records[0].transport, "file");
    let refused = r
        .records
        .iter()
        .find(|x| x.outcome == "refused")
        .expect("a refused record");
    assert_eq!(refused.about, "kill_switch:run-0002");
    assert_eq!(
        refused.to,
        vec!["ops@example.invalid", "oncall@example.invalid"]
    );
    assert_eq!(refused.to_total, 2);
    assert_eq!(refused.error.as_deref(), Some("550 5.7.1 relay denied"));
    assert_eq!(refused.run_id.as_deref(), Some("run-0002"));
    assert_eq!(refused.agent_id, "agent://example.invalid/sre/rca-copilot");
    assert_eq!(r.records[0].run_id, None);
}

#[test]
fn an_edited_record_breaks_the_chain_at_the_line_after_it() {
    // The third record's recipient changed after it was written: line 4 names
    // the hash of the third line as written, which no longer matches.
    let edited = REAL.replacen(
        "\"about\":\"digest:2026-11-23\",\"kind\":\"digest\",\"outcome\":\"accepted\",\"to\":[\"ops@example.invalid\"]",
        "\"about\":\"digest:2026-11-23\",\"kind\":\"digest\",\"outcome\":\"accepted\",\"to\":[\"someone-else@example.invalid\"]",
        1,
    );
    assert_ne!(edited, REAL, "the edit applied");
    let r = unwrap_read(read(&edited));
    assert_eq!(
        r.chain,
        ChainVerdict::Broken {
            first_line: 4,
            breaks: 1
        }
    );
}

#[test]
fn a_journal_of_one_record_is_not_called_verified() {
    let first = REAL.lines().next().expect("a line");
    let r = unwrap_read(read(&format!("{first}\n")));
    assert_eq!(r.total, 1);
    assert_eq!(r.chain, ChainVerdict::NothingBound);
    let r = unwrap_read(read(""));
    assert_eq!(r.total, 0);
    assert_eq!(r.chain, ChainVerdict::NothingBound);
}

#[test]
fn a_malformed_line_is_named_and_makes_the_next_unverifiable_not_broken() {
    let lines: Vec<&str> = REAL.lines().collect();
    let body = format!("{}\n{{not json\n{}\n", lines[0], lines[1]);
    let r = unwrap_read(read(&body));
    assert_eq!(r.total, 2);
    assert_eq!(r.skipped_total, 1);
    assert_eq!(r.skipped[0].line, 2);
    assert_eq!(r.skipped[0].reason, "not a JSON object");
    assert_eq!(
        r.chain,
        ChainVerdict::Verifies {
            chained: 0,
            heads: 1,
            unverifiable: 1
        }
    );
}

#[test]
fn a_line_from_another_writer_is_named_and_is_not_a_record() {
    let other = r#"{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-11-23T09:00:00Z","source":"tokenfuse","type":"alert_sent","agent_id":"agent://example.invalid/x/y","severity":"info","data":{"to":["a@example.invalid"],"outcome":"accepted"}}"#;
    let r = unwrap_read(read(&format!("{other}\n")));
    assert_eq!(r.total, 0);
    assert_eq!(r.skipped[0].reason, "not written by heraldyx");
}

#[test]
fn a_journal_longer_than_the_window_is_read_from_its_tail_and_says_so() {
    let line = REAL.lines().next().expect("a line");
    let mut body = String::new();
    while (body.len() as u64) < MAX_READ_BYTES + 4096 {
        body.push_str(line);
        body.push('\n');
    }
    let r = unwrap_read(read(&body));
    assert!(r.window_truncated);
    // The partial first line of the window is dropped, never read as malformed.
    assert_eq!(r.skipped_total, 0);
    assert_eq!(r.records.len(), MAX_RECORDS);
    assert!(r.total > MAX_RECORDS);
    assert!((r.total as u64) * (line.len() as u64 + 1) <= MAX_READ_BYTES);
}

#[test]
fn long_text_and_long_recipient_lists_are_bounded() {
    let to: Vec<String> = (0..80).map(|i| format!("r{i}@example.invalid")).collect();
    let line = serde_json::json!({
        "schema": "taipanbox.dev/agent-event/v0.2",
        "ts": "2026-11-23T09:00:00Z",
        "source": "heraldyx",
        "type": "alert_sent",
        "agent_id": "agent://example.invalid/x/y",
        "severity": "info",
        "data": {"about": "x".repeat(5000), "to": to, "outcome": "refused", "error": "e".repeat(5000), "kind": "alert", "transport": "smtp"}
    });
    let r = unwrap_read(read(&format!("{line}\n")));
    let rec = &r.records[0];
    assert_eq!(rec.to.len(), MAX_RECIPIENTS);
    assert_eq!(rec.to_total, 80);
    assert!(rec.about.chars().count() <= MAX_FIELD_CHARS + 3);
    assert!(rec.error.as_deref().unwrap_or_default().chars().count() <= MAX_FIELD_CHARS + 3);
}

#[test]
fn the_chain_hash_is_the_one_heraldyx_writes() {
    let lines: Vec<&str> = REAL.lines().collect();
    for pair in lines.windows(2) {
        let obj: serde_json::Map<String, Value> = serde_json::from_str(pair[0]).expect("object");
        let next: Value = serde_json::from_str(pair[1]).expect("object");
        assert_eq!(chain_hash(&obj).as_deref(), next["prev_hash"].as_str());
    }
}

#[test]
fn two_hundred_seeds_of_hostile_journals_never_panic_and_always_add_up() {
    let alphabet: Vec<char> = "{}[]\":,\\ \nabc0123\u{e9}\u{0}\u{202e}".chars().collect();
    let pieces: Vec<&str> = REAL.lines().collect();
    for seed in 0..200u64 {
        let mut x = seed.wrapping_add(17);
        let mut next = || {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (x >> 33) as usize
        };
        let mut body = String::new();
        for _ in 0..(next() % 12) {
            match next() % 3 {
                0 => body.push_str(pieces[next() % pieces.len()]),
                1 => {
                    let n = next() % 60;
                    for _ in 0..n {
                        body.push(alphabet[next() % alphabet.len()]);
                    }
                }
                _ => {
                    // A real record with one byte flipped.
                    let mut s: Vec<char> = pieces[next() % pieces.len()].chars().collect();
                    let i = next() % s.len();
                    s[i] = alphabet[next() % alphabet.len()];
                    body.extend(s);
                }
            }
            body.push('\n');
        }
        let r = unwrap_read(read(&body));
        let other = r
            .records
            .iter()
            .filter(|x| x.outcome != "accepted" && x.outcome != "refused")
            .count();
        assert_eq!(r.accepted + r.refused + other, r.total, "seed {seed}");
        assert_eq!(r.by_kind.values().sum::<usize>(), r.total, "seed {seed}");
        assert!(r.skipped.len() <= MAX_SKIPPED_NAMED, "seed {seed}");
        assert!(r.skipped.len() <= r.skipped_total, "seed {seed}");
    }
}
