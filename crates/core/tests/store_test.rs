//! Store tests: round-trip the 7 canonical events end to end (fixture -> conform
//! -> `ConsoleEvent` -> `insert_batch` -> `recent_events`), plus quarantine and
//! the source-offset upsert.

use genaryx_core::store::Store;
use genaryx_core::{Conformer, ConsoleEvent, Provenance};

const CANONICAL: &str = include_str!("fixtures/canonical.ndjson");

/// Parse+conform every non-empty line of `canonical.ndjson` into a
/// `ConsoleEvent` with a synthetic `Provenance`, offset by line index.
fn canonical_events() -> Vec<ConsoleEvent> {
    let conformer = Conformer::new().expect("embedded schemas must compile");
    CANONICAL
        .lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, line)| {
            let event = conformer
                .parse_valid(line)
                .unwrap_or_else(|report| panic!("fixture line {i}: must conform: {report:?}"));
            let schema_version = event
                .schema_version()
                .expect("fixture schema must be recognized");
            ConsoleEvent {
                event,
                provenance: Provenance {
                    env: "test".into(),
                    connector: "fixture".into(),
                    file: Some("canonical.ndjson".into()),
                    offset: Some(i as u64),
                    endpoint: None,
                    received_ts: "2026-07-16T00:00:00Z".into(),
                },
                raw: line.to_string(),
                schema_version,
            }
        })
        .collect()
}

#[test]
fn insert_batch_and_recent_events_round_trip() {
    let store = Store::open_in_memory().expect("open in-memory store");
    let events = canonical_events();
    assert_eq!(events.len(), 7, "canonical fixture should hold 7 events");

    let inserted = store.insert_batch(&events).expect("insert_batch");
    assert_eq!(inserted, 7);
    assert_eq!(store.event_count().expect("event_count"), 7);

    let recent = store.recent_events(3).expect("recent_events");
    assert_eq!(recent.len(), 3);
    // Newest first by id: the last three canonical lines are mockryx, verdryx,
    // wardryx (in that reverse-insertion order).
    assert_eq!(recent[0].source, "mockryx");
    assert_eq!(recent[1].source, "verdryx");
    assert_eq!(recent[2].source, "wardryx");
    assert!(recent[0].id > recent[1].id);
    assert!(recent[1].id > recent[2].id);
}

#[test]
fn on_behalf_of_and_data_round_trip() {
    let store = Store::open_in_memory().expect("open in-memory store");
    let events = canonical_events();
    store.insert_batch(&events).expect("insert_batch");

    let recent = store.recent_events(7).expect("recent_events");

    // The idryx `attestation_missing` event (line 3) carries a delegation chain
    // and a `data` object; both must round-trip through the JSON-text columns.
    let idryx = recent
        .iter()
        .find(|e| e.source == "idryx")
        .expect("idryx event present");
    assert_eq!(
        idryx.on_behalf_of,
        vec!["agent://acme-bank.example/eng/ci-orchestrator".to_string()]
    );
    let data = idryx.data.as_ref().expect("data present");
    assert_eq!(data.get("privileged").and_then(|v| v.as_bool()), Some(true));
    let scopes: Vec<&str> = data
        .get("scopes")
        .and_then(|v| v.as_array())
        .expect("scopes array present")
        .iter()
        .map(|v| v.as_str().expect("scope is a string"))
        .collect();
    assert_eq!(scopes, vec!["repo:write", "deploy:prod"]);

    // The engram event (line 2) has no delegation chain: on_behalf_of must
    // round-trip as an empty vec, not a stray null-turned-entry.
    let engram = recent
        .iter()
        .find(|e| e.source == "engram")
        .expect("engram event present");
    assert!(engram.on_behalf_of.is_empty());
}

#[test]
fn quarantine_records_malformed_lines() {
    let store = Store::open_in_memory().expect("open in-memory store");
    assert_eq!(store.quarantine_count().expect("quarantine_count"), 0);

    store
        .quarantine(
            "test",
            Some("bad.ndjson"),
            Some(42),
            "{not json",
            "malformed json",
            "2026-07-16T00:00:00Z",
        )
        .expect("quarantine");

    assert_eq!(store.quarantine_count().expect("quarantine_count"), 1);
}

#[test]
fn offset_upsert_overwrites() {
    let store = Store::open_in_memory().expect("open in-memory store");
    assert_eq!(
        store.get_offset("tokenfuse.ndjson").expect("get_offset"),
        None
    );

    store
        .set_offset("tokenfuse.ndjson", 100, Some(7))
        .expect("set_offset");
    assert_eq!(
        store.get_offset("tokenfuse.ndjson").expect("get_offset"),
        Some(100)
    );

    store
        .set_offset("tokenfuse.ndjson", 250, Some(7))
        .expect("set_offset (upsert)");
    assert_eq!(
        store.get_offset("tokenfuse.ndjson").expect("get_offset"),
        Some(250)
    );
}
