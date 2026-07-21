//! Tauri commands for the Evidence Center view (docs/PHASE4.md W3):
//! `evidence_status` plus [`evidence_build`] - the one on-demand "Build
//! evidence pack" action that assembles + signs a zip via the frozen
//! `genaryx_connectors::build_evidence_pack`.
//!
//! ## Why this command reuses the Money plane's `CloudClient`
//!
//! The pack's Cloud sources (compliance evidence + the audit-chain verdict)
//! and its ES256 manifest signature both need a DEVICE-PAIRED `CloudClient`
//! (`CloudClient::sign_evidence_manifest` requires an attached device
//! signer). Pairing is a real ceremony (`POST /v1/pair/new` + `POST
//! /v1/pair`, `money::state::connect`) that mints a NEW device identity every
//! time it runs - so pairing a second one here, independent of the Money
//! panel's, would mean two different device keys attest for the same
//! console, which is exactly the kind of identity drift 06 §0.5 forbids.
//! Instead, [`evidence_build`] takes `&MoneyState` (crate::
//! money) ALONGSIDE its own [`EvidenceState`] and pulls the already-paired
//! `Arc<CloudClient>` straight out of `MoneyClient` (`money/state.rs`) - the
//! SAME client the Overview/Money views read and mutate through.
//!
//! When Money is NOT `Ready` (no cloud/pairing), the build does not fail:
//! it degrades to `include_cloud: false` and builds the pack from local-tool
//! sources only. A throwaway, never-paired `CloudClient` is still
//! constructed (never a real network target - see `UNPAIRED_CLOUD_URL`)
//! purely so `sign_evidence_manifest` has something to call; with no device
//! ever attached to it, that call honestly returns
//! `ConnectorError::NoDeviceSigner`, which `build_evidence_pack` itself turns
//! into an unsigned-but-successful pack (`EvidencePack::signed = false`) -
//! never a failure, and never a pack mislabeled as signed.
//!
//! ## Why an unresolved-but-requested source still reaches `build_evidence_pack`
//!
//! `env.rs` resolves qryx/idryx/tokenfuse independently; the frontend is
//! expected to disable a source's checkbox when its tool never resolved
//! (`EvidenceStatusDto::Ready`'s `*_available` fields). This command does not
//! trust that alone (the same "re-check independently rather than trusting
//! the caller" discipline `identity::commands::identity_rescan` follows): if
//! a source is requested (`include_*: true`) but [`EvidenceEnv`] holds no
//! resolved client/binary for it, [`resolve_requested_source`] substitutes a
//! deliberately-nonexistent sentinel path/binary rather than silently
//! dropping the request. The real gather call inside `build_evidence_pack`
//! then fails to spawn it, which its own tolerant-gather loop turns into an
//! honest `MissingSource` - reusing the SAME fail-closed machinery the
//! connector already has, rather than this command inventing a parallel one.
//! `unresolved_sentinels_genuinely_fail_to_spawn` (below) proves the
//! sentinels actually trigger that path.
//!
//! ## Journaling
//!
//! After a SUCCESSFUL build, [`journal_build`] appends one
//! `console_evidence_built` `CommandRecord` via
//! `genaryx_core::command::record`, mirroring `money::commands::journal`'s
//! identical best-effort discipline (a journal failure is reported back,
//! never fatal, never panics). Only possible when Money was `Ready` (a
//! journal entry needs the SAME paired device's bus handle); when it was
//! not, `journaled: false` is reported honestly rather than attempting to
//! journal against nothing.
//!
//! ## Blocking discipline
//!
//! `build_evidence_pack`'s own module doc sanctions calling it directly from
//! "a Tauri async command" - the subprocess sources it gathers run
//! synchronously inline, but briefly blocking one worker thread of Tauri's
//! async runtime for an explicit, rare, operator-initiated "Build evidence
//! pack" click (never on a hot path) is the accepted cost that contract
//! already signs off on, unlike Crypto/Drills' fully-synchronous connector
//! calls which each need their own `spawn_blocking` wrapper.

use super::state::{EvidenceEnv, EvidenceInner, EvidenceState};
use crate::money::state::{MoneyClient, MoneyInner, MoneyState};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use chrono::{SecondsFormat, Utc};
use genaryx_connectors::{
    CloudClient, EvidenceBuildError, EvidenceInputs, QryxClient, TokenfuseClient,
    build_evidence_pack,
};
use genaryx_core::evidence::EvidenceManifest;
use genaryx_core::{CommandRecord, command};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;

// ============================================================================
// sentinels
// ============================================================================

/// A deliberately nonexistent binary, used when a source was requested
/// (`include_*: true`) but its tool never resolved - see this module's doc
/// comment.
const UNRESOLVED_QRYX_BIN: &str = "qryx-not-resolved-by-genaryx";
const UNRESOLVED_IDRYX_BIN: &str = "idryx-not-resolved-by-genaryx";
const UNRESOLVED_TOKENFUSE_BIN: &str = "tokenfuse-not-resolved-by-genaryx";

/// Reserved, obviously-fake values used ONLY for the throwaway `CloudClient`
/// built when Money is not `Ready` - see this module's doc comment. `.invalid`
/// is a reserved TLD (RFC 2606) that never resolves, so this is never dialed
/// (it never needs to be: `include_cloud` is forced `false` in that branch,
/// and the only method ever called on this client is the local, non-network
/// `sign_evidence_manifest`, which honestly fails closed with no device
/// attached).
const UNPAIRED_CLOUD_URL: &str = "http://unpaired.invalid";
const UNPAIRED_OPERATOR: &str = "console (no paired device)";
const UNPAIRED_ORG: &str = "unpaired";

// ============================================================================
// DTOs
// ============================================================================

/// Whole-panel local-tool availability, for the frontend's source checkboxes,
/// mirroring `crypto::commands::CryptoStatusDto`'s shape. Deliberately says
/// NOTHING about Cloud availability: the frontend already has `money_status`
/// for that (see this module's doc comment for why Evidence never re-derives
/// Money's own state here).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EvidenceStatusDto {
    Bootstrapping,
    Ready {
        qryx_available: bool,
        qryx_bin: Option<String>,
        qryx_default_target: Option<String>,
        idryx_available: bool,
        idryx_bin: Option<String>,
        /// The stack-bus sources Agent-BOM will actually `--load` from, e.g.
        /// `["tokenfuse", "wardryx"]` - empty is a normal, honestly-smaller
        /// Agent-BOM input, not a failure.
        idryx_load_sources: Vec<String>,
        tokenfuse_available: bool,
        tokenfuse_bin: Option<String>,
        tokenfuse_default_traces_dir: Option<String>,
    },
}

/// Every error an evidence command can return. `Build` folds every
/// `EvidenceBuildError` (and the one CloudClient-construction failure on the
/// unpaired fallback path) into a message - same "just wrap the message"
/// shallowness `crypto::commands::CryptoError`/`drills::commands::DrillsError`
/// use for their own connector errors.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvidenceError {
    Bootstrapping,
    Build { message: String },
}

impl From<EvidenceBuildError> for EvidenceError {
    fn from(e: EvidenceBuildError) -> Self {
        EvidenceError::Build {
            message: e.to_string(),
        }
    }
}

/// [`evidence_build`]'s single argument: which sources to include, plus each
/// source's operator-editable path field. A single `Deserialize` struct
/// rather than six flat parameters (clippy's `too_many_arguments`, and a
/// natural fit anyway - this is one coherent "what to build" request, not six
/// independent facts) - the frontend sends it as one `request` object.
#[derive(Debug, Deserialize)]
pub struct EvidenceBuildRequest {
    pub include_cloud: bool,
    pub include_qryx: bool,
    pub qryx_target: Option<String>,
    pub include_idryx: bool,
    pub include_tokenfuse: bool,
    pub tokenfuse_traces_dir: Option<String>,
}

/// [`evidence_build`]'s successful result: the pack, ready for the frontend
/// to trigger a browser download of (a Blob + a temporary `<a download>`, no
/// Tauri dialog plugin), plus the manifest for the contents view and the
/// journaling outcome. `cloud_included`/`journal_error` are honest additions
/// beyond the task's minimum field list (mirrors `MutationOutcome`'s own
/// `bus_recorded`/`bus_error` pairing) so the panel can show exactly what
/// happened rather than a bare boolean.
#[derive(Debug, Clone, Serialize)]
pub struct EvidenceBuildDto {
    pub zip_base64: String,
    pub filename: String,
    pub manifest: EvidenceManifest,
    pub signed: bool,
    /// The ACTUAL `include_cloud` used, after the Money-not-`Ready` degrade
    /// (see this module's doc comment) - may be `false` even when the
    /// operator checked the Cloud box.
    pub cloud_included: bool,
    pub journaled: bool,
    pub journal_error: Option<String>,
}

// ============================================================================
// helpers
// ============================================================================

/// Resolve the current [`EvidenceEnv`] out of managed state, or
/// [`EvidenceError::Bootstrapping`] when discovery has not finished yet -
/// mirrors every sibling panel's `ready_client` helper.
async fn ready_env(state: &&EvidenceState) -> Result<EvidenceEnv, EvidenceError> {
    let guard = state.inner.lock().await;
    match &*guard {
        EvidenceInner::Ready(env) => Ok(env.clone()),
        EvidenceInner::Bootstrapping => Err(EvidenceError::Bootstrapping),
    }
}

/// Pure `EvidenceInner` -> `EvidenceStatusDto` mapping, factored out of
/// [`evidence_status`] so it is directly unit-testable - same rationale as
/// `crypto::commands::status_dto`.
fn status_dto(inner: &EvidenceInner) -> EvidenceStatusDto {
    match inner {
        EvidenceInner::Bootstrapping => EvidenceStatusDto::Bootstrapping,
        EvidenceInner::Ready(env) => EvidenceStatusDto::Ready {
            qryx_available: env.qryx.is_some(),
            qryx_bin: env.qryx.as_ref().map(|q| q.qryx_bin.display().to_string()),
            qryx_default_target: env
                .qryx
                .as_ref()
                .map(|q| q.default_target.display().to_string()),
            idryx_available: env.idryx.is_some(),
            idryx_bin: env
                .idryx
                .as_ref()
                .map(|i| i.idryx_bin.display().to_string()),
            idryx_load_sources: env
                .idryx
                .as_ref()
                .map(|i| i.loads.iter().map(|(source, _)| source.clone()).collect())
                .unwrap_or_default(),
            tokenfuse_available: env.tokenfuse.is_some(),
            tokenfuse_bin: env
                .tokenfuse
                .as_ref()
                .map(|t| t.tokenfuse_bin.display().to_string()),
            tokenfuse_default_traces_dir: env
                .tokenfuse
                .as_ref()
                .and_then(|t| t.default_traces_dir.as_ref())
                .map(|p| p.display().to_string()),
        },
    }
}

/// `None` when the source was never requested at all (omitted from the
/// pack, no `MissingSource` entry - matches `build_evidence_pack`'s own
/// "independent + optional" contract); `Some(resolved)` when it was
/// requested and the environment actually has one; `Some(sentinel)` when it
/// was requested but nothing resolved - see this module's doc comment for
/// why a sentinel, not a silent drop.
fn resolve_requested_source<T>(requested: bool, resolved: Option<T>, sentinel: T) -> Option<T> {
    if !requested {
        return None;
    }
    Some(resolved.unwrap_or(sentinel))
}

/// Journal one `console_evidence_built` `CommandRecord` (best-effort: a
/// journal failure is reported, never panics and never blocks the caller
/// from getting the pack) - mirrors `money::commands::journal`'s identical
/// discipline, duplicated rather than reached into across the module
/// boundary (that helper is private to `money::commands`; same per-panel
/// convention `identity::env`'s doc comment already documents for this
/// codebase). Not a break-glass action: building an evidence pack overrides
/// no governance decision, so `decision: "allow"`, same as
/// `money::commands::money_ack_incident`.
fn journal_build(
    mc: &MoneyClient,
    sha256: &str,
    artifact_count: usize,
    missing_count: usize,
    signed: bool,
) -> (bool, Option<String>) {
    let Some(bus) = &mc.bus else {
        return (
            false,
            Some("no live event bus available (startup seeding did not complete)".to_string()),
        );
    };
    let rec = CommandRecord {
        operator: mc.operator.clone(),
        env: "local".to_string(),
        // `console.<action>` dotted form, matching this shell's own money
        // mutations (`console.kill_run`/`console.set_budget`/`console.ack_incident`)
        // and the SwiftUI shell's `console.evidence_built`, so the bus carries
        // one event type across both shells.
        action: "console.evidence_built".to_string(),
        target: sha256.to_string(),
        params: json!({
            "sha256": sha256,
            "artifact_count": artifact_count,
            "signed": signed,
            "missing_count": missing_count,
            "operator": mc.operator,
            "org": mc.org_domain,
        }),
        decision: "allow".to_string(),
        sig_alg: "es256".to_string(),
        sig_fpr: mc.sig_fpr.to_string(),
        http_status: 200,
        verify_result: format!(
            "artifacts:{artifact_count} missing:{missing_count} signed:{signed}"
        ),
    };
    match genaryx_core::store::Store::open(&bus.store_db_path) {
        Ok(store) => {
            match command::record(
                &store,
                &bus.console_events_path,
                &mc.org_domain,
                &mc.host,
                &rec,
            ) {
                Ok(()) => (true, None),
                Err(e) => (false, Some(e.to_string())),
            }
        }
        Err(e) => (false, Some(e.to_string())),
    }
}

// ============================================================================
// commands
// ============================================================================

/// Whole-panel local-tool availability. Never fails: every outcome of
/// [`super::state::bootstrap`] is a renderable [`EvidenceStatusDto`] variant.
pub async fn evidence_status(state: &EvidenceState) -> Result<EvidenceStatusDto, ()> {
    let guard = state.inner.lock().await;
    Ok(status_dto(&guard))
}

/// Build (and, when possible, sign) an evidence pack from whichever sources
/// the operator checked, journal a `console_evidence_built` record, and hand
/// the zip back as base64 for the frontend to save via a Blob download - see
/// this module's doc comment for the full Cloud-reuse and sentinel-source
/// rationale. Never auto-triggered, only on an explicit "Build evidence
/// pack" click.
pub async fn evidence_build(
    request: EvidenceBuildRequest,
    evidence_state: &EvidenceState,
    money_state: &MoneyState,
) -> Result<EvidenceBuildDto, EvidenceError> {
    let env = ready_env(&evidence_state).await?;

    // ---- Cloud: reuse Money's already-paired CloudClient; degrade honestly
    // when Money is not Ready (see this module's doc comment). ----
    let (cloud, operator, org, journal_target, include_cloud_effective) = {
        let guard = money_state.inner.lock().await;
        match &*guard {
            MoneyInner::Ready(mc) => (
                mc.client.clone(),
                mc.operator.clone(),
                mc.org_domain.clone(),
                Some(mc.clone()),
                request.include_cloud,
            ),
            _ => {
                let fallback =
                    CloudClient::new(UNPAIRED_CLOUD_URL, "").map_err(|e| EvidenceError::Build {
                        message: format!("could not prepare a client for an unsigned pack: {e}"),
                    })?;
                (
                    Arc::new(fallback),
                    UNPAIRED_OPERATOR.to_string(),
                    UNPAIRED_ORG.to_string(),
                    None,
                    false,
                )
            }
        }
    };

    let generated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

    // ---- qryx ----
    let qryx_target_owned: PathBuf = request
        .qryx_target
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| env.qryx.as_ref().map(|q| q.default_target.clone()))
        .unwrap_or_else(|| PathBuf::from("."));
    let qryx_client_owned: Option<QryxClient> = resolve_requested_source(
        request.include_qryx,
        env.qryx.as_ref().map(|q| q.client.clone()),
        QryxClient::new(UNRESOLVED_QRYX_BIN),
    );

    // ---- idryx ----
    let idryx_bin_owned: Option<PathBuf> = resolve_requested_source(
        request.include_idryx,
        env.idryx.as_ref().map(|i| i.idryx_bin.clone()),
        PathBuf::from(UNRESOLVED_IDRYX_BIN),
    );
    let idryx_loads_owned: Vec<(String, String)> = env
        .idryx
        .as_ref()
        .map(|i| {
            i.loads
                .iter()
                .map(|(source, path)| (source.clone(), path.to_string_lossy().into_owned()))
                .collect()
        })
        .unwrap_or_default();
    let idryx_loads_refs: Vec<(&str, &str)> = idryx_loads_owned
        .iter()
        .map(|(source, path)| (source.as_str(), path.as_str()))
        .collect();

    // ---- tokenfuse ----
    let tokenfuse_client_owned: Option<TokenfuseClient> = resolve_requested_source(
        request.include_tokenfuse,
        env.tokenfuse.as_ref().map(|t| t.client.clone()),
        TokenfuseClient::new(UNRESOLVED_TOKENFUSE_BIN),
    );
    let tokenfuse_traces_owned: PathBuf = request
        .tokenfuse_traces_dir
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env.tokenfuse
                .as_ref()
                .and_then(|t| t.default_traces_dir.clone())
        })
        .unwrap_or_else(|| PathBuf::from("."));

    let inputs = EvidenceInputs {
        operator: &operator,
        org: &org,
        generated_at: &generated_at,
        include_cloud: include_cloud_effective,
        qryx: qryx_client_owned
            .as_ref()
            .map(|c| (c, qryx_target_owned.as_path(), None)),
        idryx: idryx_bin_owned
            .as_deref()
            .map(|bin| (bin, idryx_loads_refs.as_slice())),
        tokenfuse: tokenfuse_client_owned
            .as_ref()
            .map(|c| (c, tokenfuse_traces_owned.as_path(), None, None)),
    };

    let pack = build_evidence_pack(&cloud, inputs)
        .await
        .map_err(EvidenceError::from)?;

    let filename = format!(
        "genaryx-evidence-{}.zip",
        generated_at.replace([':', '.'], "-")
    );
    let sha256 = {
        let mut h = Sha256::new();
        h.update(&pack.zip_bytes);
        format!("sha256:{:x}", h.finalize())
    };
    let artifact_count = pack.manifest.artifacts.len();
    let missing_count = pack.manifest.missing.len();

    let (journaled, journal_error) = match journal_target {
        Some(mc) => journal_build(&mc, &sha256, artifact_count, missing_count, pack.signed),
        None => (
            false,
            Some("no paired Money device to journal against".to_string()),
        ),
    };

    Ok(EvidenceBuildDto {
        zip_base64: B64.encode(&pack.zip_bytes),
        filename,
        manifest: pack.manifest,
        signed: pack.signed,
        cloud_included: include_cloud_effective,
        journaled,
        journal_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::state::{IdryxSource, TokenfuseSource};
    use genaryx_connectors::{IdryxClient, IdryxError, QryxError, TokenfuseError};
    use std::path::Path;

    // ---- resolve_requested_source ----

    #[test]
    fn resolve_requested_source_omits_when_not_requested() {
        assert_eq!(resolve_requested_source(false, Some(5), 0), None);
        assert_eq!(resolve_requested_source(false, None::<i32>, 0), None);
    }

    #[test]
    fn resolve_requested_source_passes_through_the_resolved_value() {
        assert_eq!(resolve_requested_source(true, Some(5), 0), Some(5));
    }

    #[test]
    fn resolve_requested_source_falls_back_to_the_sentinel_when_unresolved_but_requested() {
        assert_eq!(resolve_requested_source(true, None::<i32>, 0), Some(0));
    }

    // ---- the sentinels actually fail to spawn (proves the mechanism the
    // module doc comment describes: build_evidence_pack turns this into an
    // honest MissingSource, never a silent drop or a fake include). ----

    #[test]
    fn unresolved_sentinels_genuinely_fail_to_spawn() {
        let qryx_err = QryxClient::new(UNRESOLVED_QRYX_BIN)
            .scan_cbom_raw(Path::new("."))
            .expect_err("the sentinel qryx binary must not exist");
        assert!(matches!(qryx_err, QryxError::Spawn { .. }));

        let idryx_err = IdryxClient::agent_bom(Path::new(UNRESOLVED_IDRYX_BIN), &[])
            .expect_err("the sentinel idryx binary must not exist");
        assert!(matches!(idryx_err, IdryxError::Cli(_)));

        let tokenfuse_err = TokenfuseClient::new(UNRESOLVED_TOKENFUSE_BIN)
            .focus_export(Path::new("."), None, None)
            .expect_err("the sentinel tokenfuse binary must not exist");
        assert!(matches!(tokenfuse_err, TokenfuseError::Spawn { .. }));
    }

    // ---- status_dto ----

    #[test]
    fn status_dto_bootstrapping_maps_directly() {
        assert!(matches!(
            status_dto(&EvidenceInner::Bootstrapping),
            EvidenceStatusDto::Bootstrapping
        ));
    }

    #[test]
    fn status_dto_ready_reports_each_source_independently() {
        let env = EvidenceEnv {
            qryx: Some(crate::evidence::state::QryxSource {
                client: QryxClient::new("/tmp/qryx"),
                qryx_bin: PathBuf::from("/tmp/qryx"),
                default_target: PathBuf::from("/tmp"),
            }),
            idryx: None,
            tokenfuse: Some(TokenfuseSource {
                client: TokenfuseClient::new("/tmp/tokenfuse-gateway"),
                tokenfuse_bin: PathBuf::from("/tmp/tokenfuse-gateway"),
                default_traces_dir: None,
            }),
        };
        match status_dto(&EvidenceInner::Ready(env)) {
            EvidenceStatusDto::Ready {
                qryx_available,
                qryx_bin,
                idryx_available,
                idryx_load_sources,
                tokenfuse_available,
                tokenfuse_default_traces_dir,
                ..
            } => {
                assert!(qryx_available);
                assert_eq!(qryx_bin.as_deref(), Some("/tmp/qryx"));
                assert!(!idryx_available);
                assert!(idryx_load_sources.is_empty());
                assert!(tokenfuse_available);
                assert!(tokenfuse_default_traces_dir.is_none());
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn status_dto_ready_with_nothing_resolved_disables_every_source() {
        match status_dto(&EvidenceInner::Ready(EvidenceEnv::default())) {
            EvidenceStatusDto::Ready {
                qryx_available,
                idryx_available,
                tokenfuse_available,
                ..
            } => {
                assert!(!qryx_available);
                assert!(!idryx_available);
                assert!(!tokenfuse_available);
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn status_dto_reports_idryx_load_sources_by_name() {
        let env = EvidenceEnv {
            qryx: None,
            idryx: Some(IdryxSource {
                idryx_bin: PathBuf::from("/tmp/idryx"),
                loads: vec![
                    (
                        "tokenfuse".to_string(),
                        PathBuf::from("/tmp/tokenfuse.ndjson"),
                    ),
                    ("wardryx".to_string(), PathBuf::from("/tmp/wardryx.ndjson")),
                ],
            }),
            tokenfuse: None,
        };
        match status_dto(&EvidenceInner::Ready(env)) {
            EvidenceStatusDto::Ready {
                idryx_available,
                idryx_load_sources,
                ..
            } => {
                assert!(idryx_available);
                assert_eq!(idryx_load_sources, vec!["tokenfuse", "wardryx"]);
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    // ---- EvidenceError::from(EvidenceBuildError) ----

    #[test]
    fn evidence_error_from_build_error_carries_a_message() {
        let e = EvidenceError::from(EvidenceBuildError::NoArtifacts);
        match e {
            EvidenceError::Build { message } => {
                assert!(message.contains("no artifacts"), "got {message:?}");
            }
            other => panic!("expected Build, got {other:?}"),
        }
    }
}
