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
fn events_for_agent_filters_by_agent_id() {
    let store = Store::open_in_memory().expect("open in-memory store");
    store
        .insert_batch(&canonical_events())
        .expect("insert_batch");

    // Pick an agent_id that actually appears in the fixture, from a full read,
    // rather than hard-coding one that could drift with the fixture.
    let all = store.recent_events(100).expect("recent_events");
    let target = all[0].agent_id.clone();
    let expected = all.iter().filter(|e| e.agent_id == target).count();
    assert!(expected >= 1);

    let scoped = store
        .events_for_agent(&target, 100)
        .expect("events_for_agent");
    assert_eq!(
        scoped.len(),
        expected,
        "must return exactly this agent's events"
    );
    assert!(
        scoped.iter().all(|e| e.agent_id == target),
        "only the target agent's events"
    );
    // newest-first by id, like recent_events
    assert!(scoped.windows(2).all(|w| w[0].id > w[1].id));

    // an unknown agent is a clean empty vec, never an error
    let none = store
        .events_for_agent("agent://nobody.local/x", 100)
        .expect("events_for_agent unknown");
    assert!(none.is_empty());
}

#[test]
fn events_for_run_is_chronological_and_scoped() {
    let store = Store::open_in_memory().expect("open in-memory store");
    store
        .insert_batch(&canonical_events())
        .expect("insert_batch");

    let all = store.recent_events(100).expect("recent_events");
    let target = all
        .iter()
        .find_map(|e| e.run_id.clone())
        .expect("at least one fixture event carries a run_id");
    let expected = all
        .iter()
        .filter(|e| e.run_id.as_deref() == Some(target.as_str()))
        .count();

    let run = store.events_for_run(&target, 100).expect("events_for_run");
    assert_eq!(run.len(), expected, "must return exactly this run's events");
    assert!(
        run.iter()
            .all(|e| e.run_id.as_deref() == Some(target.as_str())),
        "only the target run's events"
    );
    // OLDEST-first (the reverse of recent_events), so replay plays forward.
    assert!(run.windows(2).all(|w| w[0].id < w[1].id));

    // an unknown run is a clean empty vec, never an error
    assert!(
        store
            .events_for_run("run-does-not-exist", 100)
            .expect("events_for_run unknown")
            .is_empty()
    );
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

// ---------------------------------------------------------------------------
// Durable history: the three properties a store that outlives its process has
// to have, and the one it must NOT have.
// ---------------------------------------------------------------------------

/// The property that makes a durable store possible at all. `stack-up`
/// truncates its event files on every start, `FileTail` resets to offset 0 when
/// it sees that, and the same lines arrive a second time. Against a store that
/// survives the process, counting them twice would double every number the
/// console reports after a restart.
#[test]
fn re_ingesting_the_same_lines_stores_them_once_and_says_so() {
    let store = Store::open_in_memory().expect("open in-memory store");
    let events = canonical_events();

    let first = store.insert_batch(&events).expect("first insert");
    assert_eq!(first, events.len(), "a fresh store takes every line");

    let second = store.insert_batch(&events).expect("replay");
    assert_eq!(second, 0, "a replay of bytes already held writes nothing");

    assert_eq!(
        store.event_count().expect("event_count") as usize,
        events.len(),
        "and the store still holds exactly one copy"
    );
}

/// Two events can legitimately be byte-identical. What makes them one event is
/// coming from the same place in the same file, so the key includes the offset
/// and identical lines at different offsets must both land.
#[test]
fn two_identical_lines_at_different_offsets_are_two_events() {
    let store = Store::open_in_memory().expect("open in-memory store");
    let mut a = canonical_events()[0].clone();
    let mut b = a.clone();
    a.provenance.offset = Some(0);
    b.provenance.offset = Some(4096);

    assert_eq!(
        store.insert_batch(&[a, b]).expect("insert"),
        2,
        "same bytes, two positions, two events"
    );
}

/// The same bytes in two ENVIRONMENTS are two events. One console can be
/// pointed at two estates, and they are not each other's history.
#[test]
fn the_same_line_in_two_environments_is_two_events() {
    let store = Store::open_in_memory().expect("open in-memory store");
    let mut a = canonical_events()[0].clone();
    let mut b = a.clone();
    a.provenance.env = "prod".into();
    b.provenance.env = "staging".into();

    assert_eq!(store.insert_batch(&[a, b]).expect("insert"), 2);
}

/// A window is on the EVENT's clock. The canonical fixture is stamped in 2026,
/// so a window that starts after it must be empty and one that starts before it
/// must hold it - and the store must never fall back to "when I read the line",
/// which would put every historical event in today.
#[test]
fn a_window_selects_on_the_events_own_timestamp() {
    let store = Store::open_in_memory().expect("open in-memory store");
    let events = canonical_events();
    store.insert_batch(&events).expect("insert");

    let (oldest, newest) = store
        .ts_span()
        .expect("ts_span")
        .expect("the fixture is dated, so the store has a span");
    assert!(oldest <= newest);

    let all = store.events_since(oldest, 100).expect("events_since");
    assert_eq!(all.len(), events.len(), "a window at the oldest holds all");

    let none = store.events_since(newest + 1, 100).expect("events_since");
    assert!(
        none.is_empty(),
        "a window starting after the newest event holds nothing"
    );

    let newest_only = store.events_since(newest, 100).expect("events_since");
    assert!(!newest_only.is_empty() && newest_only.len() < events.len());
}

/// An event this build cannot place in time is still stored, is absent from
/// every window, and is COUNTED so a caller can say so. Silently returning a
/// smaller number is the failure this pair of behaviours exists to prevent.
#[test]
fn an_undated_event_is_kept_countable_and_out_of_windows() {
    let store = Store::open_in_memory().expect("open in-memory store");
    let mut broken = canonical_events()[0].clone();
    broken.event.ts = "not a timestamp".into();
    broken.provenance.offset = Some(999_999);

    store.insert_batch(&[broken]).expect("insert");
    assert_eq!(store.event_count().expect("count"), 1, "it is stored");
    assert_eq!(store.undated_count().expect("undated"), 1, "and counted");
    assert!(
        store.events_since(0, 100).expect("since").is_empty(),
        "it has no place on a timeline, so no window claims it"
    );
    assert!(
        store.ts_span().expect("span").is_none(),
        "a store with nothing dated has no span to report"
    );
}

/// Retention drops what is past the horizon and reports how much. An undated
/// event is never dropped by age, because there is no age to compare it to:
/// deleting it would be deleting on a guess.
#[test]
fn retention_drops_the_old_and_never_the_undated() {
    let store = Store::open_in_memory().expect("open in-memory store");
    let events = canonical_events();
    let mut undated = events[0].clone();
    undated.event.ts = "not a timestamp".into();
    undated.provenance.offset = Some(999_999);

    store.insert_batch(&events).expect("insert dated");
    store.insert_batch(&[undated]).expect("insert undated");

    let (_, newest) = store.ts_span().expect("span").expect("dated events exist");
    let (dropped, _) = store.prune_before(newest).expect("prune");
    assert!(dropped > 0, "everything before the newest event goes");

    assert_eq!(
        store.undated_count().expect("undated"),
        1,
        "the undated event survives a prune it cannot be compared against"
    );
}

/// The offset journal remembers which FILE the offset belongs to. Without it a
/// durable offset is a number with no subject, and a rotated file gets resumed
/// at a position that belongs to a file that no longer exists.
#[test]
fn the_offset_journal_remembers_the_inode() {
    let store = Store::open_in_memory().expect("open in-memory store");
    store
        .set_offset("tokenfuse.ndjson", 4096, Some(31337))
        .expect("set_offset");

    let state = store
        .get_source_state("tokenfuse.ndjson")
        .expect("get_source_state")
        .expect("the file has been seen");
    assert_eq!(state.offset, 4096);
    assert_eq!(state.inode, Some(31337));

    assert!(
        store
            .get_source_state("never-seen.ndjson")
            .expect("get_source_state")
            .is_none()
    );
}
