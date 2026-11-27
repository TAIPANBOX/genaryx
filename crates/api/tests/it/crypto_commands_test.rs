//! The Crypto view's commands. A scan and a verification both run a qryx
//! process; a qryx that cannot be started must surface as an error, never as
//! an empty scan and never as a verdict. In particular `crypto_verify_evidence`
//! must not answer `verified` for a file nothing checked.
//!
//! No real qryx is spawned here: the binary path is inside a directory that
//! does not exist, so the spawn itself fails.

use genaryx_api::crypto::commands::{
    CryptoError, CryptoStatusDto, crypto_scan_cbom, crypto_scan_evidence, crypto_scan_ncsc,
    crypto_status, crypto_verify_evidence,
};
use genaryx_api::crypto::state::{CryptoClient, CryptoInner, CryptoState};
use genaryx_connectors::QryxClient;
use std::path::PathBuf;

async fn state_of(inner: CryptoInner) -> CryptoState {
    let state = CryptoState::pending();
    *state.inner.lock().await = inner;
    state
}

/// A qryx path that cannot exist: its directory was never created.
fn absent_qryx() -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "genaryx-crypto-itest-absent-{}",
            std::process::id()
        ))
        .join("qryx")
}

fn ready_with(bin: PathBuf) -> CryptoInner {
    CryptoInner::Ready(CryptoClient {
        client: QryxClient::new(bin.clone()),
        qryx_bin: bin,
        default_target: PathBuf::from("/srv/repo"),
    })
}

fn assert_spawn_error<T>(what: &str, r: Result<T, CryptoError>, bin: &std::path::Path) {
    match r {
        Err(CryptoError::Qryx { message }) => {
            assert!(
                message.starts_with("qryx spawn"),
                "{what}: a qryx that cannot start says so: {message}"
            );
            assert!(
                message.contains(&bin.display().to_string()),
                "{what}: names the binary it tried: {message}"
            );
        }
        Err(other) => panic!("{what}: expected a qryx error, got {other:?}"),
        Ok(_) => panic!("{what}: a qryx that never ran produced a result"),
    }
}

#[tokio::test]
async fn the_status_names_each_state_and_the_resolved_paths() {
    assert!(matches!(
        crypto_status(&CryptoState::pending()).await,
        Ok(CryptoStatusDto::Bootstrapping)
    ));
    assert!(matches!(
        crypto_status(&state_of(CryptoInner::NoEnvironment).await).await,
        Ok(CryptoStatusDto::NoEnvironment)
    ));
    let bin = absent_qryx();
    match crypto_status(&state_of(ready_with(bin.clone())).await).await {
        Ok(CryptoStatusDto::Ready {
            qryx_bin,
            default_target,
        }) => {
            assert_eq!(qryx_bin, bin.display().to_string());
            assert_eq!(default_target, "/srv/repo");
        }
        other => panic!("expected Ready, got {other:?}"),
    }
}

#[tokio::test]
async fn every_command_answers_the_panels_state_while_it_is_not_ready() {
    let pending = CryptoState::pending();
    assert!(matches!(
        crypto_scan_ncsc("/srv".to_string(), &pending).await,
        Err(CryptoError::Bootstrapping)
    ));
    assert!(matches!(
        crypto_scan_cbom("/srv".to_string(), &pending).await,
        Err(CryptoError::Bootstrapping)
    ));
    assert!(matches!(
        crypto_scan_evidence("/srv".to_string(), None, &pending).await,
        Err(CryptoError::Bootstrapping)
    ));
    assert!(matches!(
        crypto_verify_evidence("/srv/evidence.json".to_string(), &pending).await,
        Err(CryptoError::Bootstrapping)
    ));

    let none = state_of(CryptoInner::NoEnvironment).await;
    assert!(matches!(
        crypto_scan_ncsc("/srv".to_string(), &none).await,
        Err(CryptoError::NoEnvironment)
    ));
    assert!(matches!(
        crypto_scan_cbom("/srv".to_string(), &none).await,
        Err(CryptoError::NoEnvironment)
    ));
    assert!(matches!(
        crypto_scan_evidence("/srv".to_string(), Some("/k.pem".to_string()), &none).await,
        Err(CryptoError::NoEnvironment)
    ));
    assert!(matches!(
        crypto_verify_evidence("/srv/evidence.json".to_string(), &none).await,
        Err(CryptoError::NoEnvironment)
    ));
}

/// The verification path fails closed: an evidence file nothing could check
/// is an error, never `verified`. The scans fail the same way, never as an
/// empty inventory.
#[tokio::test]
async fn a_qryx_that_cannot_start_is_an_error_never_a_scan_or_a_verdict() {
    let bin = absent_qryx();
    let state = state_of(ready_with(bin.clone())).await;
    assert_spawn_error(
        "verify",
        crypto_verify_evidence("/srv/evidence.json".to_string(), &state).await,
        &bin,
    );
    assert_spawn_error(
        "ncsc",
        crypto_scan_ncsc("/srv".to_string(), &state).await,
        &bin,
    );
    assert_spawn_error(
        "cbom",
        crypto_scan_cbom("/srv".to_string(), &state).await,
        &bin,
    );
    assert_spawn_error(
        "evidence",
        crypto_scan_evidence("/srv".to_string(), None, &state).await,
        &bin,
    );
    assert_spawn_error(
        "signed evidence",
        crypto_scan_evidence("/srv".to_string(), Some("/k.pem".to_string()), &state).await,
        &bin,
    );
}
