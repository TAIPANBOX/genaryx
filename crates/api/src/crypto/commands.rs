//! Tauri commands for the Crypto view (docs/PHASE4.md W1): `crypto_status`
//! plus four qryx-backed reads/actions - [`crypto_scan_ncsc`] (the PQC
//! readiness timeline + its quantum-vulnerable findings), [`crypto_scan_cbom`]
//! (the CycloneDX crypto-component inventory), [`crypto_scan_evidence`] (a
//! CNSA evidence bundle, unsigned for W1 - see below), and
//! [`crypto_verify_evidence`] (checks a saved evidence file's
//! digest+signature).
//!
//! Every scan is genuinely on-demand: qryx has no live feed at all (unlike
//! Verdryx's `quality_drift` bus event), so there is no "as of load"
//! snapshot to keep fresh - the frontend labels every result "as of last
//! scan <time>" and nothing here ever auto-triggers a scan on its own
//! (docs/PHASE4.md: "Qryx is on-demand (not a live feed)").
//!
//! [`crypto_verify_evidence`] deliberately does NOT operate on whatever
//! [`crypto_scan_evidence`] just returned in memory: `QryxClient::scan_evidence`
//! only ever hands back the PARSED `EvidenceReport`, not qryx's original
//! stdout bytes the digest was computed over (the connector is frozen, this
//! is not addable here), and re-serializing our own typed DTO back to JSON
//! risks a subtly different byte stream than what qryx itself would
//! recompute against. So Verify is its own, independent action: the operator
//! supplies the path to an evidence JSON file that already exists on disk
//! (e.g. one saved from a previous qryx run, or - once W3's Evidence Center
//! exists - one it wrote), exactly matching `QryxClient::verify_evidence`'s
//! own signature (`file: &Path`, not a report value). See
//! `CryptoEvidence.tsx` for the two-form UI this implies.
//!
//! Read-only in the sense that matters here (no plane-mutating command, no
//! `genaryx_core::command::record` journal entry): a scan/evidence-build has
//! real side effects on the filesystem outside the console (temp files qryx
//! itself may write) but never touches any TAIPANBOX plane's state.
//!
//! Every qryx call is a synchronous process spawn+wait
//! (`std::process::Command::output`, blocking IO) - run inside
//! `spawn_blocking`, mirroring `identity::commands::identity_rescan`'s
//! identical discipline for `idryx detect`.

use super::state::{CryptoClient, CryptoInner, CryptoState};
use genaryx_connectors::{EvidenceReport, NcscReport, QryxError, VerifyOutcome};
use serde::Serialize;
use std::path::{Path, PathBuf};

// ============================================================================
// DTOs
// ============================================================================

/// Whole-panel connection state - mirrors `identity::commands::IdentityStatusDto`,
/// minus `Unreachable` (see `state.rs`'s module doc for why Crypto has none).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CryptoStatusDto {
    Bootstrapping,
    NoEnvironment,
    Ready {
        qryx_bin: String,
        default_target: String,
    },
}

/// Every error a crypto command can return - mirrors
/// `quality::commands::QualityError`'s shape: `QryxError` carries no
/// HTTP-style status to preserve either, just a message.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CryptoError {
    Bootstrapping,
    NoEnvironment,
    Qryx { message: String },
}

impl From<QryxError> for CryptoError {
    fn from(e: QryxError) -> Self {
        CryptoError::Qryx {
            message: e.to_string(),
        }
    }
}

// ============================================================================
// helpers
// ============================================================================

/// Resolve the current [`CryptoClient`] out of managed state, or the
/// appropriate [`CryptoError`] when the panel is not ready.
async fn ready_client(state: &&CryptoState) -> Result<CryptoClient, CryptoError> {
    let guard = state.inner.lock().await;
    match &*guard {
        CryptoInner::Ready(client) => Ok(client.clone()),
        CryptoInner::Bootstrapping => Err(CryptoError::Bootstrapping),
        CryptoInner::NoEnvironment => Err(CryptoError::NoEnvironment),
    }
}

/// Pure `CryptoInner` -> `CryptoStatusDto` mapping, factored out of
/// [`crypto_status`] so it is directly unit-testable.
fn status_dto(inner: &CryptoInner) -> CryptoStatusDto {
    match inner {
        CryptoInner::Bootstrapping => CryptoStatusDto::Bootstrapping,
        CryptoInner::NoEnvironment => CryptoStatusDto::NoEnvironment,
        CryptoInner::Ready(client) => CryptoStatusDto::Ready {
            qryx_bin: client.qryx_bin.display().to_string(),
            default_target: client.default_target.display().to_string(),
        },
    }
}

/// Run a blocking qryx call off the async executor thread - shared by every
/// command below.
async fn run_blocking<T, F>(f: F) -> Result<T, CryptoError>
where
    F: FnOnce() -> Result<T, QryxError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| CryptoError::Qryx {
            message: format!("crypto scan task failed to run: {e}"),
        })?
        .map_err(CryptoError::from)
}

// ============================================================================
// commands
// ============================================================================

/// Whole-panel connection state. Never fails: every outcome of
/// [`super::state::bootstrap`] is a renderable [`CryptoStatusDto`] variant.
pub async fn crypto_status(state: &CryptoState) -> Result<CryptoStatusDto, ()> {
    let guard = state.inner.lock().await;
    Ok(status_dto(&guard))
}

/// `qryx scan --format ncsc <path>` - the PQC readiness timeline (2028
/// discovery / 2031 highest-priority / 2035 full migration) plus the 2028
/// milestone's quantum-vulnerable findings (docs/PHASE4.md W1 positions
/// 1-2).
pub async fn crypto_scan_ncsc(
    path: String,
    state: &CryptoState,
) -> Result<NcscReport, CryptoError> {
    let client = ready_client(&state).await?;
    run_blocking(move || client.client.scan_ncsc(Path::new(&path))).await
}

/// `qryx scan --format cbom <path>` - the CycloneDX crypto-component
/// inventory (docs/PHASE4.md W1 position 3). Kept untyped on the wire, same
/// as the connector itself: CycloneDX is a large external schema this
/// console renders, not one it reasons over.
pub async fn crypto_scan_cbom(
    path: String,
    state: &CryptoState,
) -> Result<serde_json::Value, CryptoError> {
    let client = ready_client(&state).await?;
    run_blocking(move || client.client.scan_cbom(Path::new(&path))).await
}

/// `qryx scan --format evidence <path>` - a CNSA evidence bundle. Always
/// unsigned for W1 ("unsigned is fine for W1", docs/PHASE4.md W1 position
/// 4); the `sign_key` parameter still exists so this command does not need a
/// breaking signature change when W3's Evidence Center wires a real signing
/// key through the same connector method.
pub async fn crypto_scan_evidence(
    path: String,
    sign_key: Option<String>,
    state: &CryptoState,
) -> Result<EvidenceReport, CryptoError> {
    let client = ready_client(&state).await?;
    run_blocking(move || {
        let sign_key_path: Option<PathBuf> = sign_key.map(PathBuf::from);
        client
            .client
            .scan_evidence(Path::new(&path), sign_key_path.as_deref())
    })
    .await
}

/// `qryx verify-evidence <file>` - recompute the digest and check the
/// signature of a SAVED evidence JSON file. See this module's doc comment
/// for why this is deliberately decoupled from [`crypto_scan_evidence`]'s
/// in-memory result rather than round-tripping it.
pub async fn crypto_verify_evidence(
    file: String,
    state: &CryptoState,
) -> Result<VerifyOutcome, CryptoError> {
    let client = ready_client(&state).await?;
    run_blocking(move || client.client.verify_evidence(Path::new(&file))).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use genaryx_connectors::QryxClient;

    #[test]
    fn status_dto_maps_bootstrapping_and_no_environment_directly() {
        assert!(matches!(
            status_dto(&CryptoInner::Bootstrapping),
            CryptoStatusDto::Bootstrapping
        ));
        assert!(matches!(
            status_dto(&CryptoInner::NoEnvironment),
            CryptoStatusDto::NoEnvironment
        ));
    }

    #[test]
    fn status_dto_ready_carries_bin_and_default_target() {
        let ready = CryptoInner::Ready(CryptoClient {
            client: QryxClient::new("/tmp/qryx"),
            qryx_bin: PathBuf::from("/tmp/qryx"),
            default_target: PathBuf::from("/tmp"),
        });
        match status_dto(&ready) {
            CryptoStatusDto::Ready {
                qryx_bin,
                default_target,
            } => {
                assert_eq!(qryx_bin, "/tmp/qryx");
                assert_eq!(default_target, "/tmp");
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn crypto_error_from_qryx_error_carries_a_message() {
        // A binary that cannot spawn -> a genuine QryxError::Spawn, same
        // fixture `QryxClient`'s own tests use - avoids hand-constructing a
        // QryxError variant just for this test.
        let c = QryxClient::new("/nonexistent/qryx-binary-xyz");
        let err = c
            .scan_ncsc(Path::new("/repo"))
            .expect_err("a nonexistent binary must fail to spawn");

        let mapped = CryptoError::from(err);
        let CryptoError::Qryx { message } = mapped else {
            panic!("expected a Qryx-shaped CryptoError")
        };
        assert!(!message.is_empty());
    }
}
