//! The egress panel over a real store: what agents fetched, what was refused,
//! and the two counts an operator reads first (fetches governed only at the
//! navigation, and fetches whose subresources nobody could report). A store
//! that cannot be read is said to be unread, never a panel of zeros.

use genaryx_api::bus::{AppState, BusMode};
use genaryx_api::egress::egress_recent;
use genaryx_core::ingest::IngestService;
use genaryx_core::store::Store;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

fn scratch(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-egress-itest-{tag}-{}-{}",
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

fn line(source: &str, event_type: &str, second: u32, data: Value) -> String {
    format!(
        "{}\n",
        json!({
            "schema": "taipanbox.dev/agent-event/v0.2",
            "ts": format!("2026-10-20T09:00:{second:02}.000Z"),
            "source": source,
            "type": event_type,
            "agent_id": "agent://acme.example/agents/browser",
            "run_id": "run-web",
            "data": data,
        })
    )
}

/// A wardryx line first, then seven scopyx lines in time order: three fetches,
/// two refusals with a verdict, a type this build does not know, and a refusal
/// with no verdict at all.
fn seed(dir: &Path) {
    let store = Store::open(&dir.join("console.sqlite")).expect("store");
    let mut ingest = IngestService::new(store, "single").expect("ingest");
    let wardryx = line(
        "wardryx",
        "web_fetch",
        0,
        json!({"origin": "https://not-scopyx.example"}),
    );
    let scopyx = [
        line(
            "scopyx",
            "web_fetch",
            1,
            json!({
                "origin": "https://docs.example", "url_sha384": "sha384:aa11",
                "backend": "chromium", "enforcement": "navigation_only",
                "content_bytes": 2048, "subresources_requested": null,
            }),
        ),
        line(
            "scopyx",
            "web_fetch",
            2,
            json!({
                "origin": "https://api.example", "backend": "passthrough",
                "enforcement": "per_request", "content_bytes": 512,
                "subresources_requested": 0,
            }),
        ),
        line(
            "scopyx",
            "web_fetch",
            3,
            json!({
                "origin": "https://x.example", "backend": "chromium",
                "enforcement": "navigation_only",
            }),
        ),
        line(
            "scopyx",
            "web_blocked",
            4,
            json!({
                "origin": "https://evil.example", "verdict": "denied_host",
                "reason": "not on the allow list",
            }),
        ),
        line(
            "scopyx",
            "web_blocked",
            5,
            json!({"origin": "https://evil2.example", "verdict": "denied_host"}),
        ),
        line(
            "scopyx",
            "web_cache_note",
            6,
            json!({"origin": "https://z.example"}),
        ),
        line(
            "scopyx",
            "web_blocked",
            7,
            json!({"origin": "https://y.example"}),
        ),
    ]
    .concat();
    for (name, body) in [("wardryx", wardryx), ("scopyx", scopyx)] {
        let path = dir.join(format!("{name}.ndjson"));
        std::fs::write(&path, body).expect("write");
        ingest
            .add_file_source(format!("filetail:{name}"), &path)
            .expect("tail");
    }
    let stats = ingest.poll_once().expect("poll");
    assert_eq!((stats.inserted, stats.quarantined), (8, 0));
}

#[test]
fn an_unopenable_store_is_unmeasured_and_says_so() {
    let root = scratch("unopenable");
    let file = root.join("not-a-directory");
    std::fs::write(&file, "x").expect("write");
    let panel = egress_recent(50, &state_at(Some(file)));
    assert!(!panel.measured);
    assert!(panel.rows.is_empty());
    let note = panel.note.expect("an unmeasured panel says why");
    assert!(note.contains("could not be opened"), "{note}");
    assert!(
        note.contains("not a report that your agents made no web requests"),
        "{note}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_panel_counts_fetches_refusals_and_what_nobody_could_report() {
    let root = scratch("full");
    seed(&root);
    let panel = egress_recent(50, &state_at(Some(root.clone())));
    assert!(panel.measured);

    let t = &panel.totals;
    assert_eq!(t.fetched, 3);
    assert_eq!(t.blocked, 3);
    assert_eq!(
        t.navigation_only, 2,
        "two fetches were governed only at the navigation"
    );
    assert_eq!(
        t.subresources_unknown, 2,
        "null and absent are both unknown; a reported zero is not"
    );
    assert_eq!(t.by_verdict.get("denied_host"), Some(&2));
    assert_eq!(
        t.by_verdict.get("unrecorded"),
        Some(&1),
        "a refusal with no verdict is counted as unrecorded, never dropped"
    );
    assert_eq!(t.by_verdict.len(), 2);

    let origins: Vec<&str> = panel.rows.iter().map(|r| r.origin.as_str()).collect();
    assert_eq!(
        origins,
        vec![
            "https://y.example",
            "https://evil2.example",
            "https://evil.example",
            "https://x.example",
            "https://api.example",
            "https://docs.example",
        ],
        "scopyx only, the unknown type skipped, newest first"
    );

    let newest = &panel.rows[0];
    assert_eq!(newest.outcome, "blocked");
    assert_eq!(newest.verdict, None);
    assert_eq!(newest.backend, None);
    assert_eq!(newest.run_id.as_deref(), Some("run-web"));
    assert_eq!(newest.agent_id, "agent://acme.example/agents/browser");
    assert_eq!(newest.ts, "2026-10-20T09:00:07.000Z");

    let refused = &panel.rows[2];
    assert_eq!(refused.verdict.as_deref(), Some("denied_host"));
    assert_eq!(refused.reason.as_deref(), Some("not on the allow list"));

    let oldest = &panel.rows[5];
    assert_eq!(oldest.outcome, "fetched");
    assert_eq!(oldest.backend.as_deref(), Some("chromium"));
    assert_eq!(oldest.enforcement.as_deref(), Some("navigation_only"));
    assert_eq!(oldest.content_bytes, Some(2048));
    assert_eq!(oldest.url_sha384.as_deref(), Some("sha384:aa11"));
    assert_eq!(oldest.verdict, None);

    let note = panel.note.expect("a measured panel names its window");
    assert!(
        note.contains("Read from the 8 most recent events"),
        "the window counts every event read, scopyx or not: {note}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_limit_caps_the_rows_returned_and_the_totals_follow_them() {
    let root = scratch("limited");
    seed(&root);
    let panel = egress_recent(2, &state_at(Some(root.clone())));
    assert!(panel.measured);
    let origins: Vec<&str> = panel.rows.iter().map(|r| r.origin.as_str()).collect();
    assert_eq!(origins, vec!["https://y.example", "https://evil2.example"]);
    assert_eq!(panel.totals.blocked, 2);
    assert_eq!(panel.totals.fetched, 0);
    let note = panel.note.expect("a note");
    assert!(
        note.contains("Read from the 8 most recent events"),
        "the read is wider than the rows kept: {note}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
