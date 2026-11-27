//! The bus's three reads over the console's store: recent events, what was
//! refused, and how the stream is fed. Each one is driven over a real store
//! filled through the real ingest path, and over the two ways the store can
//! be missing: never configured, and configured but unopenable.

use genaryx_api::bus::{AppState, BusMode, bus_status, quarantine, recent_events};
use genaryx_api::events::mock_events;
use genaryx_core::ingest::IngestService;
use genaryx_core::store::Store;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const V02: &str = "taipanbox.dev/agent-event/v0.2";

fn scratch(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-bus-state-itest-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn state_at(events_dir: Option<PathBuf>) -> AppState {
    AppState {
        events_dir,
        source_events_dir: None,
        mode: BusMode::Unavailable {
            reason: "test".to_string(),
        },
    }
}

/// A path whose `console.sqlite` can never be opened: its parent is a file.
fn unopenable(root: &Path) -> PathBuf {
    let file = root.join("not-a-directory");
    std::fs::write(&file, "a file where a directory should be").expect("write");
    file
}

fn line(source: &str, event_type: &str, ts: &str, data: Value) -> String {
    format!(
        "{}\n",
        json!({
            "schema": V02,
            "ts": ts,
            "source": source,
            "type": event_type,
            "agent_id": "agent://acme.example/agents/a",
            "run_id": "run-1",
            "data": data,
        })
    )
}

/// Write each `(stream, body)` as `<stream>.ndjson` under `dir` and ingest all
/// of them, in order, into the store the bus reads.
fn ingest(dir: &Path, streams: &[(&str, String)]) -> (usize, usize) {
    let store = Store::open(&dir.join("console.sqlite")).expect("open the store");
    let mut ingest = IngestService::new(store, "single").expect("ingest service");
    for (name, body) in streams {
        let path = dir.join(format!("{name}.ndjson"));
        std::fs::write(&path, body).expect("write the stream");
        ingest
            .add_file_source(format!("filetail:{name}"), &path)
            .expect("tail the stream");
    }
    let stats = ingest.poll_once().expect("poll");
    (stats.inserted, stats.quarantined)
}

#[test]
fn with_no_store_the_explorer_falls_back_to_the_labelled_mock_timeline() {
    let got: Vec<i64> = recent_events(10, &state_at(None))
        .iter()
        .map(|e| e.id)
        .collect();
    let mock: Vec<i64> = mock_events(10).iter().map(|e| e.id).collect();
    assert_eq!(got, mock);
    assert_eq!(got.len(), 10);
}

#[test]
fn a_store_that_cannot_be_opened_falls_back_rather_than_failing() {
    let root = scratch("unopenable-recent");
    let rows = recent_events(3, &state_at(Some(unopenable(&root))));
    let ids: Vec<i64> = rows.iter().map(|e| e.id).collect();
    assert_eq!(
        ids,
        vec![41, 40, 39],
        "the mock timeline, capped at the limit"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_real_store_is_read_newest_first_and_capped_at_the_limit() {
    let root = scratch("real-store");
    let body = [
        line(
            "wardryx",
            "policy_allow",
            "2026-10-20T09:00:00.000Z",
            json!({}),
        ),
        line(
            "wardryx",
            "policy_deny",
            "2026-10-20T09:00:01.000Z",
            json!({}),
        ),
        line(
            "wardryx",
            "approval_requested",
            "2026-10-20T09:00:02.000Z",
            json!({}),
        ),
    ]
    .concat();
    assert_eq!(ingest(&root, &[("wardryx", body)]), (3, 0));

    let state = state_at(Some(root.clone()));
    let rows = recent_events(10, &state);
    let types: Vec<&str> = rows.iter().map(|e| e.event_type.as_str()).collect();
    assert_eq!(
        types,
        vec!["approval_requested", "policy_deny", "policy_allow"],
        "newest first"
    );
    assert!(
        rows.iter().all(|e| e.id >= 1),
        "real rows carry store ids, never the mock's or the live feed's"
    );
    assert!(rows.iter().all(|e| e.env == "single"));
    assert!(
        rows[0]
            .file
            .as_deref()
            .is_some_and(|f| f.ends_with("wardryx.ndjson"))
    );
    assert!(rows[0].raw.contains("approval_requested"));
    assert_eq!(recent_events(2, &state).len(), 2);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn with_no_store_the_quarantine_refuses_to_report_a_clean_bus() {
    let panel = quarantine(&state_at(None));
    assert!(!panel.measured);
    assert_eq!(panel.total, 0);
    assert!(panel.reasons.is_empty());
    let note = panel.note.expect("an unmeasured panel says why");
    assert!(note.contains("no event store"), "{note}");
    assert!(note.contains("This is not a report"), "{note}");
}

#[test]
fn an_unopenable_store_is_named_as_unopenable_not_as_clean() {
    let root = scratch("unopenable-quarantine");
    let panel = quarantine(&state_at(Some(unopenable(&root))));
    assert!(!panel.measured);
    let note = panel.note.expect("a note");
    assert!(note.contains("could not be opened"), "{note}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_clean_bus_claims_the_check_rather_than_rendering_blank() {
    let root = scratch("clean");
    assert_eq!(
        ingest(
            &root,
            &[(
                "wardryx",
                line(
                    "wardryx",
                    "policy_allow",
                    "2026-10-20T09:00:00.000Z",
                    json!({})
                )
            )]
        ),
        (1, 0)
    );
    let panel = quarantine(&state_at(Some(root.clone())));
    assert!(panel.measured);
    assert_eq!(panel.total, 0);
    assert!(panel.reasons.is_empty());
    let note = panel.note.expect("a note");
    assert!(note.contains("conformed to the envelope"), "{note}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn refused_lines_are_counted_with_their_reasons_and_where_to_find_them() {
    let root = scratch("refused");
    let body = format!(
        "not json at all\n{}\n{}",
        r#"{"schema":"taipanbox.dev/agent-event/v9.9","ts":"2026-10-20T09:00:00.000Z"}"#,
        line(
            "wardryx",
            "policy_allow",
            "2026-10-20T09:00:00.000Z",
            json!({})
        )
    );
    assert_eq!(ingest(&root, &[("wardryx", body)]), (1, 2));

    let panel = quarantine(&state_at(Some(root.clone())));
    assert!(panel.measured);
    assert_eq!(panel.total, 2);
    assert_eq!(panel.reasons.len(), 2, "two different faults, two rows");
    assert_eq!(
        panel.reasons.iter().map(|r| r.count).sum::<u64>(),
        panel.total,
        "every refused line is under some reason"
    );
    for r in &panel.reasons {
        assert!(
            r.example_file
                .as_deref()
                .is_some_and(|f| f.ends_with("wardryx.ndjson")),
            "each reason points at the producer's file: {r:?}"
        );
        assert!(r.example_offset.is_some(), "{r:?}");
    }
    assert!(
        panel
            .reasons
            .iter()
            .any(|r| r.reason.contains("malformed json")),
        "the validator's own words: {:?}",
        panel.reasons
    );
    let note = panel.note.expect("a note");
    assert!(note.contains("Fix the producer"), "{note}");
    assert!(
        !note.contains("Showing the"),
        "every reason is shown, so the note does not say otherwise: {note}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn more_reasons_than_the_panel_shows_are_said_to_be_partial() {
    let root = scratch("many-reasons");
    {
        let store = Store::open(&root.join("console.sqlite")).expect("store");
        for i in 0..14 {
            store
                .quarantine(
                    "single",
                    Some("/var/lib/events/wardryx.ndjson"),
                    Some(i * 100),
                    "{}",
                    &format!("reason number {i:02}"),
                    "2026-10-20T09:00:00Z",
                )
                .expect("quarantine a line");
        }
    }
    let panel = quarantine(&state_at(Some(root.clone())));
    assert!(panel.measured);
    assert_eq!(panel.total, 14);
    assert_eq!(panel.reasons.len(), 12, "capped at the twelve worst");
    let note = panel.note.expect("a note");
    assert!(
        note.contains("Showing the 12 most common reasons, covering 12 of them."),
        "a partial list says it is partial: {note}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_bus_status_is_the_mode_the_bus_was_started_in() {
    let modes = [
        BusMode::Live {
            env: "single".to_string(),
            dir: "/var/lib/stack/events".to_string(),
        },
        BusMode::Demo {
            dir: "/tmp/demo".to_string(),
        },
        BusMode::Unavailable {
            reason: "store failed".to_string(),
        },
    ];
    for mode in modes {
        let state = AppState {
            events_dir: None,
            source_events_dir: None,
            mode: mode.clone(),
        };
        assert_eq!(bus_status(&state), mode);
    }
    let wire = serde_json::to_value(BusMode::Demo {
        dir: "/tmp/demo".to_string(),
    })
    .expect("serializes");
    assert_eq!(wire, json!({"kind": "demo", "dir": "/tmp/demo"}));
}
