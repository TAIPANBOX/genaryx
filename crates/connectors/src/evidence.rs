//! `build_evidence_pack`: the Evidence Center's gather + sign orchestration
//! (docs/PHASE4.md W3). This is the ONE function every caller uses to produce
//! a pack, so the gathering, the honest missing-source accounting, the ES256
//! signing, and the zip assembly are defined once here, never duplicated
//! across shells (originally the Tauri and SwiftUI shells, today genaryx-api
//! on behalf of the web shell). It lives in `genaryx-connectors` because that
//! is the layer that has every service client plus the device signer; the pure
//! zip/manifest half is `genaryx_core::evidence`.
//!
//! ## What it gathers (each source independent + tolerant)
//!
//! - Cloud **compliance evidence** (`GET /v1/compliance/evidence`) + the Cloud
//!   **audit-chain verdict** (`GET /v1/audit/verify`, its `ok`/`break_index`
//!   recorded as the artifact's own verify status).
//! - Qryx **crypto evidence** (`scan --format evidence`, captured VERBATIM so
//!   its embedded digest / ML-DSA signature still self-verify) + the **CBOM**.
//! - idryx **Agent-BOM** (CycloneDX) and TokenFuse **FOCUS** cost CSV.
//!
//! Every source is optional and independent: a source that errors becomes a
//! [`genaryx_core::evidence::MissingSource`] in the manifest (with the error as
//! its reason), never a silently-dropped one, so the pack states exactly what it
//! contains. If NOTHING could be gathered, the whole build fails
//! ([`EvidenceBuildError::NoArtifacts`]) rather than emit a pack of nothing.
//!
//! ## Signing is fail-closed
//!
//! The console signs the manifest with its attached device ES256 key
//! ([`crate::CloudClient::sign_evidence_manifest`]). No device attached ->
//! [`crate::ConnectorError::NoDeviceSigner`] -> an honestly-UNSIGNED pack
//! ([`EvidencePack::signed`] `= false`, no `manifest.sig.json`); a GENUINE
//! signing failure -> [`EvidenceBuildError::Sign`] and NO pack, never one that
//! claims to be signed.

use std::path::Path;

use genaryx_core::evidence::{
    Artifact, EvidenceError, EvidenceManifest, MissingSource, assemble_zip,
};

use crate::{CloudClient, ConnectorError, IdryxClient, QryxClient, TokenfuseClient};

// ---- error -----------------------------------------------------------------

/// Why an evidence-pack build failed outright (as opposed to a single source
/// being absent, which is a recorded [`MissingSource`], not an error).
#[derive(Debug, thiserror::Error)]
pub enum EvidenceBuildError {
    /// Not one source could be gathered - a pack of nothing is not evidence.
    #[error("evidence pack has no artifacts: every requested source failed")]
    NoArtifacts,

    /// A GENUINE manifest-signing failure (not "no device attached", which
    /// yields an honestly-unsigned pack). Fail-closed: no pack is produced.
    #[error("evidence manifest signing failed: {0}")]
    Sign(#[source] ConnectorError),

    /// The manifest could not be serialized or the zip could not be assembled.
    #[error("evidence assembly: {0}")]
    Assemble(#[from] EvidenceError),
}

// ---- inputs + output -------------------------------------------------------

/// Which sources to include and the resolved inputs each needs. Every field is
/// borrowed; the shells own the clients + paths and pass references.
pub struct EvidenceInputs<'a> {
    /// The console operator principal recorded in the manifest.
    pub operator: &'a str,
    /// The org the evidence is for.
    pub org: &'a str,
    /// UTC ISO-8601 build time (the caller stamps it; core never reads the clock).
    pub generated_at: &'a str,
    /// Include the Cloud compliance evidence + the audit-chain verdict.
    pub include_cloud: bool,
    /// Qryx crypto evidence + CBOM: `(client, scan target, optional ML-DSA sign key)`.
    pub qryx: Option<(&'a QryxClient, &'a Path, Option<&'a Path>)>,
    /// idryx Agent-BOM: `(idryx binary, --load source:path specs)`.
    pub idryx: Option<(&'a Path, &'a [(&'a str, &'a str)])>,
    /// TokenFuse FOCUS export: `(client, traces dir, from?, to?)`.
    pub tokenfuse: Option<(
        &'a TokenfuseClient,
        &'a Path,
        Option<&'a str>,
        Option<&'a str>,
    )>,
}

/// A built evidence pack: the zip bytes to save, the manifest (for the panel's
/// contents view), and whether it was signed.
pub struct EvidencePack {
    pub zip_bytes: Vec<u8>,
    pub manifest: EvidenceManifest,
    pub signed: bool,
}

// ---- orchestration ---------------------------------------------------------

/// Gather every requested source, build + sign the manifest, and assemble the
/// zip. `cloud` is always needed (compliance + audit reads, and it holds the
/// device signer). See the module doc for the tolerant-gather + fail-closed-sign
/// contract.
///
/// On-demand batch operation: the subprocess sources (Qryx/idryx/TokenFuse) run
/// synchronously inline, so callers invoke this from a context that tolerates
/// blocking (today, genaryx-api's async command handler on the web shell's
/// tokio runtime; the removed desktop shells used a Tauri async command or an
/// FFI handle's owned runtime), never on a hot path.
pub async fn build_evidence_pack(
    cloud: &CloudClient,
    inputs: EvidenceInputs<'_>,
) -> Result<EvidencePack, EvidenceBuildError> {
    let mut artifacts: Vec<Artifact> = Vec::new();
    let mut missing: Vec<MissingSource> = Vec::new();

    // ---- Cloud: compliance evidence + audit verdict ----
    if inputs.include_cloud {
        match cloud.compliance_evidence().await {
            Ok(v) => artifacts.push(json_artifact(
                "Cloud compliance evidence",
                "compliance-evidence.json",
                "tokenfuse cloud GET /v1/compliance/evidence",
                &v,
                None,
            )),
            Err(e) => missing.push(missing_of("Cloud compliance evidence", &e)),
        }
        match cloud.audit_verify().await {
            Ok(av) => {
                let status = if av.ok {
                    "audit chain VERIFIED end-to-end".to_string()
                } else {
                    match av.break_index {
                        Some(i) => format!("audit chain BROKEN at index {i}"),
                        None => "audit chain BROKEN".to_string(),
                    }
                };
                artifacts.push(json_artifact_of(
                    "Cloud audit-chain verdict",
                    "audit-verify.json",
                    "tokenfuse cloud GET /v1/audit/verify",
                    &av,
                    Some(status),
                ));
            }
            Err(e) => missing.push(missing_of("Cloud audit-chain verdict", &e)),
        }
    }

    // ---- Qryx: crypto evidence (verbatim) + CBOM ----
    if let Some((qryx, target, sign_key)) = inputs.qryx {
        match qryx.scan_evidence_raw(target, sign_key) {
            Ok(bytes) => {
                let status = if sign_key.is_some() {
                    "self-verifying (embedded digest + ML-DSA signature); run `qryx verify-evidence`"
                } else {
                    "self-verifying (embedded digest); run `qryx verify-evidence`"
                };
                artifacts.push(raw_artifact(
                    "Qryx crypto evidence (CNSA)",
                    "qryx-evidence.json",
                    "application/json",
                    "qryx scan --format evidence",
                    bytes,
                    Some(status.to_string()),
                ));
            }
            Err(e) => missing.push(missing_of("Qryx crypto evidence (CNSA)", &e)),
        }
        match qryx.scan_cbom_raw(target) {
            Ok(bytes) => artifacts.push(raw_artifact(
                "CBOM (CycloneDX)",
                "cbom.json",
                "application/json",
                "qryx scan --format cbom",
                bytes,
                None,
            )),
            Err(e) => missing.push(missing_of("CBOM (CycloneDX)", &e)),
        }
    }

    // ---- idryx: Agent-BOM (CycloneDX) ----
    if let Some((idryx_bin, loads)) = inputs.idryx {
        match IdryxClient::agent_bom(idryx_bin, loads) {
            Ok(bytes) => artifacts.push(raw_artifact(
                "Agent-BOM (CycloneDX)",
                "agent-bom.json",
                "application/json",
                "idryx bom --format json",
                bytes,
                None,
            )),
            Err(e) => missing.push(missing_of("Agent-BOM (CycloneDX)", &e)),
        }
    }

    // ---- TokenFuse: FOCUS cost CSV ----
    if let Some((tf, traces, from, to)) = inputs.tokenfuse {
        match tf.focus_export(traces, from, to) {
            Ok(bytes) => artifacts.push(raw_artifact(
                "TokenFuse FOCUS cost export",
                "focus.csv",
                "text/csv",
                "tokenfuse focus-export",
                bytes,
                None,
            )),
            Err(e) => missing.push(missing_of("TokenFuse FOCUS cost export", &e)),
        }
    }

    if artifacts.is_empty() {
        return Err(EvidenceBuildError::NoArtifacts);
    }

    // ---- manifest + fail-closed signing + zip ----
    let manifest = EvidenceManifest::build(
        &artifacts,
        missing,
        inputs.generated_at,
        inputs.operator,
        inputs.org,
    );
    let manifest_bytes = manifest.canonical_bytes()?;

    let signature = match cloud.sign_evidence_manifest(&manifest_bytes) {
        Ok(sig) => Some(sig),
        // No paired device: an honestly-unsigned pack, not a failure.
        Err(ConnectorError::NoDeviceSigner) => None,
        // A real signing failure fails the whole build - never an unsigned pack
        // that could be mistaken for a signed one.
        Err(e) => return Err(EvidenceBuildError::Sign(e)),
    };
    let signed = signature.is_some();

    let zip_bytes = assemble_zip(&manifest_bytes, signature.as_ref(), &artifacts)?;
    Ok(EvidencePack {
        zip_bytes,
        manifest,
        signed,
    })
}

// ---- helpers ---------------------------------------------------------------

fn raw_artifact(
    name: &str,
    filename: &str,
    content_type: &str,
    source: &str,
    bytes: Vec<u8>,
    verify_status: Option<String>,
) -> Artifact {
    Artifact {
        name: name.to_string(),
        filename: filename.to_string(),
        content_type: content_type.to_string(),
        source: source.to_string(),
        tool_version: None,
        verify_status,
        bytes,
    }
}

/// A JSON artifact from an already-parsed [`serde_json::Value`], serialized
/// pretty (the exact bytes are what get hashed + zipped; for these non-self-
/// verifying sources re-serialization is fine, unlike Qryx evidence).
fn json_artifact(
    name: &str,
    filename: &str,
    source: &str,
    value: &serde_json::Value,
    verify_status: Option<String>,
) -> Artifact {
    let bytes = serde_json::to_vec_pretty(value).unwrap_or_else(|_| b"{}".to_vec());
    raw_artifact(
        name,
        filename,
        "application/json",
        source,
        bytes,
        verify_status,
    )
}

/// A JSON artifact from any `Serialize` value (e.g. `AuditVerifyResponse`).
fn json_artifact_of<T: serde::Serialize>(
    name: &str,
    filename: &str,
    source: &str,
    value: &T,
    verify_status: Option<String>,
) -> Artifact {
    let bytes = serde_json::to_vec_pretty(value).unwrap_or_else(|_| b"{}".to_vec());
    raw_artifact(
        name,
        filename,
        "application/json",
        source,
        bytes,
        verify_status,
    )
}

fn missing_of(name: &str, err: &dyn std::fmt::Display) -> MissingSource {
    MissingSource {
        name: name.to_string(),
        reason: err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    // The gather/sign orchestration is exercised end-to-end by the shells'
    // integration paths and (for the pure assembly) by
    // `genaryx_core::evidence`'s own tests. Here we only pin the honest-partial
    // + fail-closed contract shape that is pure enough to test without live
    // services: an all-absent build is `NoArtifacts`, and a `MissingSource`
    // carries the error text. The live end-to-end (real cloud + qryx) lives in
    // the shells / tests/.
    use super::*;

    #[test]
    fn missing_of_carries_the_error_text() {
        let m = missing_of("X", &"idryx binary not found");
        assert_eq!(m.name, "X");
        assert!(m.reason.contains("idryx binary not found"));
    }

    #[test]
    fn json_artifact_of_serializes_and_defaults_content_type() {
        #[derive(serde::Serialize)]
        struct V {
            ok: bool,
        }
        let a = json_artifact_of(
            "Audit",
            "audit.json",
            "src",
            &V { ok: true },
            Some("VERIFIED".into()),
        );
        assert_eq!(a.filename, "audit.json");
        assert_eq!(a.content_type, "application/json");
        assert_eq!(a.verify_status.as_deref(), Some("VERIFIED"));
        assert!(String::from_utf8_lossy(&a.bytes).contains("\"ok\": true"));
    }
}
