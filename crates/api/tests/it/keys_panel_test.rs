//! Invariant 53: the Keys panel reads Keyryx custody and its witness over
//! their public GET routes, on a stub that answers as keyryx v0.1.5 does
//! (`internal/api/api.go`, `internal/wlog/wlog.go`).

use crate::cloud_stub::{spawn_cloud, spawn_recording_cloud};
use genaryx_api::keys::{
    CUSTODY_ENV, CustodyPart, KeysConfig, WITNESS_ENV, WitnessPart, keys_custody,
};

const STATE: &str = r#"{"role":"custody","version":"v0.1.5","root_key":"loaded","keys":[{"name":"wardryx.approval","alg":"ES384","mode":"held","active_kid":"kid-2","active_since":1763890000,"next_kid":"kid-3","next_activates_at":1763990000}],"alarms":[{"kind":"state_mac","name":"trailryx.seal","kid":"kid-9","reason":"the state MAC does not verify"}]}"#;
const KEYS: &str = r#"{"keys":[{"name":"wardryx.approval","alg":"ES384","kid":"kid-1","mode":"held","state":"retired","created":1760000000,"retired":1763890000},{"name":"wardryx.approval","alg":"ES384","kid":"kid-2","mode":"held","state":"active","created":1763800000,"activate_at":1763890000},{"name":"wardryx.approval","alg":"ES384","kid":"kid-3","mode":"held","state":"next","created":1763980000,"activate_at":1763990000}]}"#;
const HEAD: &str = r#"{"size":4242,"root":"9f2c","at":1763891000,"es384":{"kid":"w-es","sig":"AAA"},"ml_dsa_65":{"kid":"w-ml","sig":"BBB"}}"#;

fn cfg(custody: Option<&str>, witness: Option<&str>) -> KeysConfig {
    KeysConfig {
        custody: custody.map(str::to_string),
        witness: witness.map(str::to_string),
    }
}

#[tokio::test]
async fn custody_and_the_witness_are_read_and_the_last_rotation_is_named() {
    let base = spawn_cloud(vec![
        ("/v1/state", "200 OK", STATE.to_string()),
        ("/v1/keys", "200 OK", KEYS.to_string()),
        ("/v1/witness/head", "200 OK", HEAD.to_string()),
    ]);
    let panel = keys_custody(&cfg(Some(&base), Some(&base)), &[]).await;
    let CustodyPart::Read {
        version,
        names,
        alarms,
        entries_total,
        inventory_error,
        ..
    } = panel.custody
    else {
        panic!("expected custody read, got {:?}", panel.custody);
    };
    assert_eq!(version, "v0.1.5");
    assert_eq!(entries_total, 3);
    assert_eq!(inventory_error, None);
    assert_eq!(names[0].active_kid.as_deref(), Some("kid-2"));
    assert_eq!(names[0].next_kid.as_deref(), Some("kid-3"));
    assert_eq!(names[0].last_rotation, Some(1_763_980_000));
    assert_eq!(alarms[0].reason, "the state MAC does not verify");
    let WitnessPart::Read {
        size,
        root,
        at,
        es384_kid,
        ml_dsa_65_kid,
        ..
    } = panel.witness
    else {
        panic!("expected witness read, got {:?}", panel.witness);
    };
    assert_eq!((size, root.as_str(), at), (4242, "9f2c", 1_763_891_000));
    assert_eq!(es384_kid.as_deref(), Some("w-es"));
    assert_eq!(ml_dsa_65_kid.as_deref(), Some("w-ml"));
}

#[tokio::test]
async fn the_panel_sends_only_the_three_reads_and_no_credential() {
    let (base, seen) = spawn_recording_cloud(vec![
        ("/v1/state", "200 OK", STATE.to_string()),
        ("/v1/keys", "200 OK", KEYS.to_string()),
        ("/v1/witness/head", "200 OK", HEAD.to_string()),
    ]);
    let _ = keys_custody(&cfg(Some(&base), Some(&base)), &[]).await;
    let seen = seen.lock().expect("log").clone();
    let mut paths: Vec<String> = seen
        .iter()
        .map(|s| format!("{} {}", s.method, s.path))
        .collect();
    paths.sort();
    assert_eq!(
        paths,
        vec!["GET /v1/keys", "GET /v1/state", "GET /v1/witness/head"]
    );
    assert!(seen.iter().all(|s| s.header("authorization").is_none()));
}

#[tokio::test]
async fn nothing_named_is_not_configured_and_names_each_setting() {
    let panel = keys_custody(&cfg(None, None), &[]).await;
    assert_eq!(
        panel.custody,
        CustodyPart::NotConfigured {
            variable: CUSTODY_ENV.to_string()
        }
    );
    assert_eq!(
        panel.witness,
        WitnessPart::NotConfigured {
            variable: WITNESS_ENV.to_string()
        }
    );
}

#[tokio::test]
async fn a_setting_that_is_not_a_url_is_refused_by_name() {
    let refused = vec![(
        CUSTODY_ENV.to_string(),
        "must start with http:// or https://".to_string(),
    )];
    let panel = keys_custody(&cfg(None, None), &refused).await;
    assert!(
        matches!(panel.custody, CustodyPart::Refused { ref variable, .. } if variable == CUSTODY_ENV)
    );
}

#[tokio::test]
async fn a_custody_that_does_not_answer_is_unreachable_not_empty() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let closed = format!("http://{}", listener.local_addr().expect("addr"));
    drop(listener);
    let panel = keys_custody(&cfg(Some(&closed), Some(&closed)), &[]).await;
    assert!(
        matches!(panel.custody, CustodyPart::Unreachable { .. }),
        "{:?}",
        panel.custody
    );
    assert!(
        matches!(panel.witness, WitnessPart::Unreachable { .. }),
        "{:?}",
        panel.witness
    );
}

#[tokio::test]
async fn a_witness_with_no_head_yet_says_so() {
    let base = spawn_cloud(vec![]);
    let panel = keys_custody(&cfg(None, Some(&base)), &[]).await;
    assert!(
        matches!(panel.witness, WitnessPart::NoHeadYet { .. }),
        "{:?}",
        panel.witness
    );
}

#[tokio::test]
async fn an_inventory_that_fails_while_the_state_answers_is_said() {
    let base = spawn_cloud(vec![
        ("/v1/state", "200 OK", STATE.to_string()),
        ("/v1/keys", "500 Internal Server Error", "{}".to_string()),
    ]);
    let panel = keys_custody(&cfg(Some(&base), None), &[]).await;
    let CustodyPart::Read {
        inventory_error,
        names,
        ..
    } = panel.custody
    else {
        panic!("expected a read state");
    };
    assert_eq!(inventory_error.as_deref(), Some("answered 500"));
    assert_eq!(names[0].last_rotation, None);
}

#[tokio::test]
async fn a_redirect_is_not_followed_and_an_oversized_answer_is_not_read() {
    let big = format!("{{\"keys\":[], \"pad\":\"{}\"}}", "x".repeat(2 << 20));
    let base = spawn_cloud(vec![
        ("/v1/state", "302 Found", String::new()),
        ("/v1/witness/head", "200 OK", big),
    ]);
    let panel = keys_custody(&cfg(Some(&base), Some(&base)), &[]).await;
    match panel.custody {
        CustodyPart::Unreachable { reason, .. } => assert!(reason.contains("302"), "{reason}"),
        other => panic!("expected unreachable, got {other:?}"),
    }
    match panel.witness {
        WitnessPart::Unreachable { reason, .. } => {
            assert!(reason.contains("longer than"), "{reason}")
        }
        other => panic!("expected unreachable, got {other:?}"),
    }
}
