//! Store tests: round-trip the 7 canonical events end to end (fixture -> conform
//! -> `ConsoleEvent` -> `insert_batch` -> `recent_events`), plus quarantine and
//! the source-offset upsert.

use genaryx_core::store::Store;
use genaryx_core::{Conformer, ConsoleEvent, Provenance};

const CANONICAL: &str = include_str!("../fixtures/canonical.ndjson");

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
        .set_offset("tokenfuse.ndjson", 100, Some(7), None)
        .expect("set_offset");
    assert_eq!(
        store.get_offset("tokenfuse.ndjson").expect("get_offset"),
        Some(100)
    );

    store
        .set_offset("tokenfuse.ndjson", 250, Some(7), None)
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
        .set_offset("tokenfuse.ndjson", 4096, Some(31337), Some("deadbeef"))
        .expect("set_offset");

    let state = store
        .get_source_state("tokenfuse.ndjson")
        .expect("get_source_state")
        .expect("the file has been seen");
    assert_eq!(state.offset, 4096);
    assert_eq!(state.inode, Some(31337));
    assert_eq!(
        state.head_sha.as_deref(),
        Some("deadbeef"),
        "the fingerprint of the consumed bytes is what survives inode reuse"
    );

    assert!(
        store
            .get_source_state("never-seen.ndjson")
            .expect("get_source_state")
            .is_none()
    );
}

/// Every table this store creates has a writer.
///
/// # THE ONE THAT DID NOT, AND WHAT IT COST
///
/// `rollup_spend_1m` was created by the very first migration with the comment
/// "populated later by a reducer task". No reducer was ever written. It sat
/// there empty from Phase 0 until 2026-08-11, and nothing noticed, because an
/// empty table is indistinguishable from a quiet one and nothing ever looked.
///
/// It was not harmless. When the Statistics tab needed per-agent history, the
/// obvious move was a rollup table filled by a reducer, and the argument
/// against it was hard to make from first principles. This table WAS the
/// argument: a cache that can drift from the events will drift, and the
/// evidence that a speculative one never gets built was sitting in the schema.
/// The decision to go without one was then confirmed by measurement rather than
/// by taste (`crates/api/tests/it/stats_scale.rs`: a per-agent profile stays
/// between 1 and 5 ms across a hundredfold increase in rows).
///
/// This test is the marker that keeps it gone. Adding a table here is fine;
/// adding one with nothing that writes to it is what this refuses.
#[test]
fn the_schema_holds_no_table_that_nothing_writes() {
    let store = Store::open_in_memory().expect("open in-memory store");
    let tables = store.table_names().expect("table_names");

    assert!(
        !tables.contains(&"rollup_spend_1m".to_string()),
        "rollup_spend_1m had no writer for the whole life of this project and was \
         dropped on 2026-08-11; re-adding it needs a reducer in the same commit. \
         Tables now: {tables:?}"
    );

    // The four that remain, each named so that adding a fifth is a deliberate
    // act rather than a diff nobody reads.
    for expected in [
        "events",
        "source_offsets",
        "event_quarantine",
        "commands_journal",
    ] {
        assert!(
            tables.contains(&expected.to_string()),
            "{expected} must exist; tables: {tables:?}"
        );
    }
    assert_eq!(
        tables.len(),
        4,
        "a new table needs a writer and a line in this test, got {tables:?}"
    );
}

/// One stored event of `type_` about `agent`, carrying `data`. Built from the
/// canonical fixture's first line so every envelope field a producer must set
/// is set, and with `raw` unique per event, because the store's dedupe key
/// hashes it.
fn event_about(
    agent: &str,
    type_: &str,
    data: serde_json::Value,
    ts: &str,
    n: u64,
) -> ConsoleEvent {
    let mut e = canonical_events()[0].clone();
    e.event.event_type = type_.to_string();
    e.event.agent_id = agent.to_string();
    e.event.ts = ts.to_string();
    e.event.data = Some(data.clone());
    e.provenance.offset = Some(10_000 + n);
    e.raw =
        serde_json::json!({ "n": n, "type": type_, "agent_id": agent, "data": data }).to_string();
    e
}

/// TokenFuse's `identity_mismatch` names the agent its caller CLAIMED on the
/// envelope and the credential that actually called in `data.key_id`. The
/// aggregate and the per-agent profile read must file it under that key, never
/// under the impersonated agent, whatever shape the key arrives in.
#[test]
fn an_identity_refusal_is_grouped_under_its_key_in_the_aggregate() {
    let store = Store::open_in_memory().expect("open in-memory store");
    let flint = "agent://taipanbox.dev/routers/flint";
    let ts = "2026-10-07T10:00:00Z";
    store
        .insert_batch(&[
            // flint's own refusal, which IS about flint.
            event_about(flint, "policy_deny", serde_json::json!({}), ts, 1),
            // Two attempts by somebody else's key, claiming to be flint.
            event_about(
                flint,
                "identity_mismatch",
                serde_json::json!({ "key_id": "forge-imposter", "agent_id": flint, "reason": "agent_id_not_allowed" }),
                ts,
                2,
            ),
            event_about(
                flint,
                "identity_mismatch",
                serde_json::json!({ "key_id": "forge-imposter", "agent_id": flint, "reason": "agent_id_not_allowed" }),
                "2026-10-07T10:01:00Z",
                3,
            ),
            // No client keys on the gateway: no key to file it under.
            event_about(flint, "identity_mismatch", serde_json::json!({ "key_id": null }), ts, 4),
            event_about(flint, "identity_mismatch", serde_json::json!({ "key_id": "" }), ts, 5),
            event_about(flint, "identity_mismatch", serde_json::json!({ "key_id": 42 }), ts, 6),
            event_about(flint, "identity_mismatch", serde_json::json!({}), ts, 7),
            // A key named like SQL, and one named like an agent.
            event_about(flint, "identity_mismatch", serde_json::json!({ "key_id": "x' OR 1=1 --" }), ts, 8),
            event_about(flint, "identity_mismatch", serde_json::json!({ "key_id": flint }), ts, 9),
        ])
        .expect("insert");

    let counts = store.type_counts_since(None).expect("type_counts_since");
    let count = |subject: &str, t: &str| {
        counts
            .iter()
            .filter(|c| c.agent_id == subject && c.type_ == t)
            .map(|c| c.count)
            .sum::<u64>()
    };
    assert_eq!(
        count(flint, "identity_mismatch"),
        0,
        "an identity refusal was counted under the agent it claimed: {counts:?}"
    );
    assert_eq!(
        count(flint, "policy_deny"),
        1,
        "flint's own stop stays flint's"
    );
    assert_eq!(count("key:forge-imposter", "identity_mismatch"), 2);
    assert_eq!(
        count("key:(none)", "identity_mismatch"),
        4,
        "null, empty, a number and an absent key name no key"
    );
    assert_eq!(count("key:x' OR 1=1 --", "identity_mismatch"), 1);
    assert_eq!(
        count(&format!("key:{flint}"), "identity_mismatch"),
        1,
        "a key named like an agent is still a key, and not that agent"
    );

    // The per-agent profile reads the same rule.
    let day = store
        .daily_type_counts(flint, 0)
        .expect("daily_type_counts");
    assert!(
        day.iter().all(|d| d.type_ != "identity_mismatch"),
        "an identity refusal reached the claimed agent's profile: {day:?}"
    );
    let key_day = store
        .daily_type_counts("key:forge-imposter", 0)
        .expect("daily_type_counts");
    assert_eq!(
        key_day.iter().map(|d| d.count).sum::<u64>(),
        2,
        "the key's own profile holds its two attempts"
    );
}

/// The rule has two spellings, `attribution::filed_under` in Rust and
/// `attribution::FILED_UNDER_SQL` in the store's queries. A sweep of 200
/// seeded `key_id` shapes (strings with quotes, percent signs, backslashes,
/// multi-byte text, numbers, nulls, objects, absent) goes through both,
/// and every row must come back filed under the same subject by each, and by
/// the aggregate.
#[test]
fn an_identity_refusal_is_filed_the_same_way_in_sql_and_in_rust() {
    use genaryx_core::attribution::filed_under;

    let store = Store::open_in_memory().expect("open in-memory store");
    let flint = "agent://taipanbox.dev/routers/flint";
    let alphabet: Vec<char> = "ab'\"%_\\-:/ ()é漢😀0".chars().collect();
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        state >> 33
    };
    let mut events = Vec::new();
    for seed in 0..200u64 {
        let key = match next() % 6 {
            0 => serde_json::Value::Null,
            1 => serde_json::json!(next() as i64),
            2 => serde_json::json!({ "k": "v" }),
            3 => serde_json::json!(""),
            _ => {
                let len = (next() % 40) as usize;
                let s: String = (0..len)
                    .map(|_| alphabet[(next() as usize) % alphabet.len()])
                    .collect();
                serde_json::json!(s)
            }
        };
        let data = if seed % 17 == 0 {
            serde_json::json!({ "agent_id": flint })
        } else {
            serde_json::json!({ "key_id": key, "agent_id": flint })
        };
        let t = if seed % 5 == 0 {
            "policy_deny"
        } else {
            "identity_mismatch"
        };
        events.push(event_about(
            flint,
            t,
            data,
            "2026-10-07T10:00:00Z",
            20_000 + seed,
        ));
    }
    store.insert_batch(&events).expect("insert");

    let rows = store
        .events_of_types_since(&["identity_mismatch", "policy_deny"], &[], None, 10_000)
        .expect("events_of_types_since");
    assert_eq!(
        rows.len(),
        200,
        "every planted event was stored and read back"
    );
    let mut expected: std::collections::BTreeMap<(String, String), u64> = Default::default();
    for r in &rows {
        let rust = filed_under(&r.type_, &r.agent_id, r.data.as_ref());
        assert_eq!(
            r.filed_under, rust,
            "SQL and Rust disagree on data {:?}",
            r.data
        );
        if r.type_ == "identity_mismatch" {
            assert_ne!(
                rust, flint,
                "a refusal fell back to the claimed agent: {:?}",
                r.data
            );
        }
        *expected.entry((rust, r.type_.clone())).or_insert(0) += 1;
    }
    let counted: std::collections::BTreeMap<(String, String), u64> = store
        .type_counts_since(None)
        .expect("type_counts_since")
        .into_iter()
        .map(|c| ((c.agent_id, c.type_), c.count))
        .collect();
    assert_eq!(
        counted, expected,
        "the aggregate files a row somewhere the row read does not"
    );
}
