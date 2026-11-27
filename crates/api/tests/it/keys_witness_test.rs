//! The Keys panel's answers that are not a clean read: a witness or custody
//! that answers with another status or a body of the wrong shape, and a
//! setting refused at startup, each said as what it is and never as empty.

use crate::cloud_stub::spawn_cloud;
use genaryx_api::keys::{
    CUSTODY_ENV, CustodyPart, KeysConfig, WITNESS_ENV, WitnessPart, keys_custody,
};

fn cfg(custody: Option<&str>, witness: Option<&str>) -> KeysConfig {
    KeysConfig {
        custody: custody.map(str::to_string),
        witness: witness.map(str::to_string),
    }
}

#[tokio::test]
async fn a_witness_that_answers_another_status_is_unreachable_with_that_status() {
    let base = spawn_cloud(vec![(
        "/v1/witness/head",
        "500 Internal Server Error",
        r#"{"error":"log unreadable"}"#.to_string(),
    )]);
    let panel = keys_custody(&cfg(None, Some(&base)), &[]).await;
    assert_eq!(
        panel.witness,
        WitnessPart::Unreachable {
            url: base,
            reason: "answered 500".to_string(),
        }
    );
    assert_eq!(
        panel.custody,
        CustodyPart::NotConfigured {
            variable: CUSTODY_ENV.to_string()
        }
    );
}

#[tokio::test]
async fn a_witness_head_of_the_wrong_shape_is_not_read_as_a_head() {
    let base = spawn_cloud(vec![(
        "/v1/witness/head",
        "200 OK",
        r#"{"size":"big","root":7}"#.to_string(),
    )]);
    let panel = keys_custody(&cfg(None, Some(&base)), &[]).await;
    match panel.witness {
        WitnessPart::Unreachable { url, reason } => {
            assert_eq!(url, base);
            assert!(reason.starts_with("not the expected answer: "), "{reason}");
        }
        other => panic!("expected Unreachable, got {other:?}"),
    }
}

#[tokio::test]
async fn a_head_without_signatures_reads_with_no_kids() {
    let base = spawn_cloud(vec![(
        "/v1/witness/head",
        "200 OK",
        r#"{"size":3,"root":"ab","at":17}"#.to_string(),
    )]);
    let panel = keys_custody(&cfg(None, Some(&base)), &[]).await;
    assert_eq!(
        panel.witness,
        WitnessPart::Read {
            url: base,
            size: 3,
            root: "ab".to_string(),
            at: 17,
            es384_kid: None,
            ml_dsa_65_kid: None,
        }
    );
}

#[tokio::test]
async fn a_custody_state_that_answers_another_status_or_shape_is_unreachable() {
    let refused = spawn_cloud(vec![(
        "/v1/state",
        "403 Forbidden",
        r#"{"error":"no"}"#.to_string(),
    )]);
    let panel = keys_custody(&cfg(Some(&refused), None), &[]).await;
    assert_eq!(
        panel.custody,
        CustodyPart::Unreachable {
            url: refused,
            reason: "answered 403".to_string(),
        }
    );
    assert_eq!(
        panel.witness,
        WitnessPart::NotConfigured {
            variable: WITNESS_ENV.to_string()
        }
    );

    let garbled = spawn_cloud(vec![("/v1/state", "200 OK", "[1,2,3]".to_string())]);
    match keys_custody(&cfg(Some(&garbled), None), &[]).await.custody {
        CustodyPart::Unreachable { reason, .. } => {
            assert!(reason.starts_with("not the expected answer: "), "{reason}");
        }
        other => panic!("expected Unreachable, got {other:?}"),
    }
}

#[tokio::test]
async fn a_refused_setting_wins_over_a_configured_one_and_reads_nothing() {
    let base = spawn_cloud(vec![]);
    let refused = [
        (CUSTODY_ENV.to_string(), "not a URL".to_string()),
        (WITNESS_ENV.to_string(), "has a path".to_string()),
    ];
    let panel = keys_custody(&cfg(Some(&base), Some(&base)), &refused).await;
    assert_eq!(
        panel.custody,
        CustodyPart::Refused {
            variable: CUSTODY_ENV.to_string(),
            reason: "not a URL".to_string(),
        }
    );
    assert_eq!(
        panel.witness,
        WitnessPart::Refused {
            variable: WITNESS_ENV.to_string(),
            reason: "has a path".to_string(),
        }
    );
}
