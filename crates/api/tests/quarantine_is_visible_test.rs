//! A producer with a broken envelope must be findable, not merely quiet.
//!
//! # THE FAULT THIS HOLDS AGAINST
//!
//! Refused lines have always been kept, with their file, offset, raw bytes and
//! the validator's own reason, on the principle that a malformed line must
//! never silently vanish. Nothing ever read them back. `quarantine_count`
//! existed with no caller, and the only report anywhere was one `eprintln!` at
//! startup, to stderr, once.
//!
//! So the line did not vanish and the operator could not tell the difference.
//! The console did not go blank either: it kept showing the rest of the bus,
//! correctly, while the broken producer's agents just looked idle.
//!
//! This drives the REAL captured campaign through the REAL ingest path rather
//! than a hand-written bad line, because the fault it records is not "a
//! validator rejects a malformed string". It is that twelve events from a live
//! benchmark run reached this console and left no trace an operator could see.

use genaryx_core::ingest::IngestService;
use genaryx_core::store::Store;

/// The real `aws-comparable-176` output, non-conforming exactly as captured on
/// 2026-07-16. Also lives in `genaryx-core`'s own conformance suite, which
/// asserts each of the twelve is refused; this asserts what the CONSOLE then
/// does about it.
const CAMPAIGN: &str = include_str!("../../core/tests/fixtures/campaign-aws-176.ndjson");

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-quarantine-{tag}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create_dir_all");
    dir
}

#[test]
fn a_campaign_the_envelope_refuses_is_reported_rather_than_looking_quiet() {
    let source = temp_dir("src");
    let store_dir = temp_dir("store");
    let ndjson = source.join("tokenfuse.ndjson");
    std::fs::write(&ndjson, CAMPAIGN).expect("write fixture");

    let store = Store::open(&store_dir.join("console.sqlite")).expect("Store::open");
    let mut ingest = IngestService::new(store, "local").expect("IngestService::new");
    ingest
        .add_file_source("tokenfuse", &ndjson)
        .expect("add_file_source");
    let stats = ingest.poll_once().expect("poll_once");

    assert_eq!(
        stats.inserted, 0,
        "not one of these conforms, so not one belongs on the bus"
    );
    assert_eq!(stats.quarantined, 12, "and all twelve must be kept");

    // What an operator can now actually see.
    let state = genaryx_api::bus::AppState {
        events_dir: Some(store_dir.clone()),
        source_events_dir: Some(source.clone()),
        mode: genaryx_api::bus::BusMode::Live {
            env: "local".into(),
            dir: source.display().to_string(),
        },
    };
    let panel = genaryx_api::bus::quarantine(&state);

    assert!(panel.measured, "the store was there and was read");
    assert_eq!(panel.total, 12);

    let note = panel.note.expect("a measured panel still explains itself");
    assert!(
        note.contains("quieter") && note.contains("Fix the producer"),
        "the note must carry the consequence and the action, got: {note}"
    );
    assert!(
        !note.contains("12"),
        "and must NOT restate the count: it is a field, and repeating it puts the same \
         number on screen twice in two wordings. Got: {note}"
    );

    // One producer, one fault, one row. Twelve copies of the same sentence
    // would be a log, and this is meant to be read.
    assert_eq!(
        panel.reasons.len(),
        1,
        "twelve lines with one fault are one reason, got {:?}",
        panel.reasons.iter().map(|r| &r.reason).collect::<Vec<_>>()
    );
    let r = &panel.reasons[0];
    assert_eq!(r.count, 12);
    assert!(
        r.reason.contains("agent://"),
        "the reason must be the validator's own words, so it names the grammar that failed: {}",
        r.reason
    );

    // The three things that make it fixable rather than merely known.
    assert!(
        r.example_file
            .as_deref()
            .is_some_and(|f| f.ends_with("tokenfuse.ndjson")),
        "which file, got {:?}",
        r.example_file
    );
    assert!(r.example_offset.is_some(), "and where in it");
    assert!(
        r.raw_excerpt
            .as_deref()
            .is_some_and(|e| e.contains("aws-comparable-agent")),
        "and enough of the line to recognize the producer, got {:?}",
        r.raw_excerpt
    );

    let _ = std::fs::remove_dir_all(&source);
    let _ = std::fs::remove_dir_all(&store_dir);
}

/// The other half, and the one that decides whether the panel is worth
/// trusting: a clean bus must say every line was accepted, and must say it in
/// words that cannot be read as "we did not look".
#[test]
fn a_clean_bus_says_it_checked_rather_than_saying_nothing() {
    let store_dir = temp_dir("clean");
    let _ = Store::open(&store_dir.join("console.sqlite")).expect("Store::open");

    let state = genaryx_api::bus::AppState {
        events_dir: Some(store_dir.clone()),
        source_events_dir: None,
        mode: genaryx_api::bus::BusMode::Unavailable {
            reason: "test".into(),
        },
    };
    let panel = genaryx_api::bus::quarantine(&state);
    assert!(panel.measured);
    assert_eq!(panel.total, 0);
    assert!(panel.reasons.is_empty());
    let note = panel.note.expect("even a clean panel says what it checked");
    assert!(
        note.contains("conformed"),
        "an empty panel must claim the check, not just render blank: {note}"
    );

    let _ = std::fs::remove_dir_all(&store_dir);
}

/// And the failure mode this whole module exists against: with no store to
/// read, the panel must refuse to report a clean bus.
#[test]
fn with_no_store_it_does_not_report_a_clean_bus() {
    let state = genaryx_api::bus::AppState {
        events_dir: None,
        source_events_dir: None,
        mode: genaryx_api::bus::BusMode::Unavailable {
            reason: "test".into(),
        },
    };
    let panel = genaryx_api::bus::quarantine(&state);
    assert!(!panel.measured);
    let note = panel.note.expect("an unmeasured panel must say why");
    assert!(
        note.contains("not a report that every line your producers sent was accepted"),
        "the note must refuse the wrong reading explicitly, got: {note}"
    );
}
