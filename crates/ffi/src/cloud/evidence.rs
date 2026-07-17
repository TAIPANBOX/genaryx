//! Wire DTOs and error taxonomy for the Evidence Center (docs/PHASE4.md W3),
//! exported from [`super::CloudHandle`] (see [`super::CloudHandle::build_evidence_pack`]).
//! Mirrors `crates/ffi/src/idryx/dto.rs`/`crypto/dto.rs`'s shape (UniFFI
//! `Record`/`Error` types instead of `genaryx_core`/`genaryx_connectors`'
//! plain Rust structs), field-for-field over
//! [`genaryx_core::evidence::EvidenceManifest`] and its children, plus the
//! two Evidence-Center-only shapes [`EvidenceBuildInputs`] (what the Swift
//! shell sends) and [`EvidencePackRecord`] (what it gets back).
//!
//! ## `operator_name`, not `operator`
//!
//! [`genaryx_core::evidence::EvidenceManifest::operator`] is mirrored here as
//! [`EvidenceManifestRecord::operator_name`] (same for
//! [`EvidenceBuildInputs::operator_name`]) rather than the identical
//! `operator` - `operator` is a Swift keyword (`KEYWORDS` in
//! `uniffi_bindgen`'s own Swift codegen, `gen_swift/mod.rs`), which UniFFI
//! backtick-escapes in a `Record`'s field DECLARATION but not consistently
//! everywhere a generated binding might read it back (member access), so
//! this crate avoids the whole class of friction the same way it already
//! avoids `type_`/`type` collisions for [`crate::UiEvent::event_type`] - see
//! that field's own doc comment for the identical reasoning.
//!
//! ## Optional sources: `Option<String>`, not an empty-string sentinel
//!
//! Every field naming a possibly-absent tool path/target is `Option<String>`
//! (mirrors [`crate::crypto::CryptoHandle::scan_evidence`]'s own `sign_key:
//! Option<String>` parameter), not a plain `String` with `""` meaning
//! "absent" - `None` is a source deliberately not requested; the connector
//! layer already distinguishes that from a source that WAS requested but
//! failed (recorded as a [`MissingSourceRecord`] in the returned manifest,
//! never silently dropped). [`super::CloudHandle::build_evidence_pack`]
//! additionally treats a whitespace-only `Some("   ")` as `None` (defense in
//! depth at the FFI boundary; the Swift side already converts an empty text
//! field to `nil` before crossing, mirroring `DrillsModel.run`'s own
//! `apiKey.isEmpty ? nil : apiKeyValue`).

use genaryx_core::evidence::{EvidenceManifest, ManifestArtifact, MissingSource};

// ============================================================================
// EvidenceBuildInputs: what the Swift shell sends
// ============================================================================

/// One idryx `--load source:path` pair - the same shape
/// [`crate::idryx::IdryxHandle::rescan`] resolves internally, exposed here
/// so [`super::CloudHandle::evidence_env_defaults`] can hand the identical
/// pairs back for Agent-BOM (`IdryxClient::agent_bom` takes the exact same
/// `(source, path)` shape `IdryxClient::rescan` does).
#[derive(Debug, Clone, uniffi::Record)]
pub struct EvidenceLoadEntry {
    pub source: String,
    pub path: String,
}

/// Everything [`super::CloudHandle::build_evidence_pack`] needs, resolved
/// Swift-side (either typed in by the operator, or pre-filled from
/// [`super::CloudHandle::evidence_env_defaults`] - see that method's own
/// doc). A source's own fields are `None`/empty exactly when that source's
/// toggle is off or unresolved in the Swift panel - see the module doc's
/// "Optional sources" section for how that is honored one layer down.
#[derive(Debug, Clone, uniffi::Record)]
pub struct EvidenceBuildInputs {
    /// The console operator principal to record in the manifest. `None`
    /// (or blank) falls back to this handle's own paired
    /// [`super::CloudHandle::console_operator`] - the manifest never ships
    /// with a blank operator field.
    pub operator_name: Option<String>,
    /// The org the evidence is for. `None` (or blank) falls back to this
    /// handle's own [`super::CloudHandle::org_domain`].
    pub org: Option<String>,
    /// UTC ISO-8601 build time, stamped by the Swift caller - the FFI layer
    /// is a thin passthrough here, never fabricating a fallback (mirrors
    /// [`genaryx_connectors::EvidenceInputs::generated_at`]'s own "the
    /// caller stamps it; core never reads the clock" contract).
    pub generated_at: String,
    /// Include the Cloud compliance evidence + the audit-chain verdict.
    pub include_cloud: bool,
    /// Qryx: `scan --format evidence` + `--format cbom`. Needs BOTH
    /// `qryx_bin` and `qryx_target` non-blank to run at all; `qryx_sign_key`
    /// is independently optional (an unsigned Qryx evidence bundle is a
    /// normal, valid choice).
    pub qryx_bin: Option<String>,
    pub qryx_target: Option<String>,
    pub qryx_sign_key: Option<String>,
    /// idryx Agent-BOM: needs `idryx_bin` non-blank; `idryx_loads` may
    /// legitimately be empty (idryx's own CLI then honestly refuses the
    /// zero-input run, surfacing as a [`MissingSourceRecord`], never a
    /// silent drop - see `genaryx_connectors::build_evidence_pack`'s doc).
    pub idryx_bin: Option<String>,
    pub idryx_loads: Vec<EvidenceLoadEntry>,
    /// TokenFuse FOCUS export: needs BOTH `tokenfuse_bin` and
    /// `tokenfuse_traces_dir` non-blank; `tokenfuse_from`/`tokenfuse_to`
    /// independently optionally window the export.
    pub tokenfuse_bin: Option<String>,
    pub tokenfuse_traces_dir: Option<String>,
    pub tokenfuse_from: Option<String>,
    pub tokenfuse_to: Option<String>,
}

// ============================================================================
// EvidenceManifestRecord + children: mirrors genaryx_core::evidence
// ============================================================================

/// One artifact's entry in the manifest: exact field set of
/// [`ManifestArtifact`] (the bytes themselves live in
/// [`EvidencePackRecord::zip_bytes`], never duplicated here).
#[derive(Debug, Clone, uniffi::Record)]
pub struct ManifestArtifactRecord {
    pub name: String,
    pub filename: String,
    pub content_type: String,
    pub source: String,
    pub tool_version: Option<String>,
    /// The artifact's OWN self-verification status when it has one (Qryx's
    /// embedded digest/signature, the Cloud audit-chain verdict), else
    /// `None` - never fabricated.
    pub verify_status: Option<String>,
    /// `"sha256:<hex>"` over the artifact's own bytes.
    pub sha256: String,
    pub size_bytes: u64,
}

impl From<&ManifestArtifact> for ManifestArtifactRecord {
    fn from(a: &ManifestArtifact) -> Self {
        Self {
            name: a.name.clone(),
            filename: a.filename.clone(),
            content_type: a.content_type.clone(),
            source: a.source.clone(),
            tool_version: a.tool_version.clone(),
            verify_status: a.verify_status.clone(),
            sha256: a.sha256.clone(),
            size_bytes: a.size_bytes,
        }
    }
}

/// A source that was requested but could not be included - exact field set
/// of [`MissingSource`]. The manifest's honest "what did NOT make it in and
/// why" list; the panel's "Not included" section renders this verbatim,
/// never silently drops it.
#[derive(Debug, Clone, uniffi::Record)]
pub struct MissingSourceRecord {
    pub name: String,
    pub reason: String,
}

impl From<&MissingSource> for MissingSourceRecord {
    fn from(m: &MissingSource) -> Self {
        Self {
            name: m.name.clone(),
            reason: m.reason.clone(),
        }
    }
}

/// The pack manifest the panel's contents view renders: exact field set of
/// [`EvidenceManifest`] (`operator` renamed `operator_name` - see the module
/// doc).
#[derive(Debug, Clone, uniffi::Record)]
pub struct EvidenceManifestRecord {
    pub pack_version: String,
    pub generated_at: String,
    pub operator_name: String,
    pub org: String,
    pub artifacts: Vec<ManifestArtifactRecord>,
    pub missing: Vec<MissingSourceRecord>,
}

impl From<&EvidenceManifest> for EvidenceManifestRecord {
    fn from(m: &EvidenceManifest) -> Self {
        Self {
            pack_version: m.pack_version.clone(),
            generated_at: m.generated_at.clone(),
            operator_name: m.operator.clone(),
            org: m.org.clone(),
            artifacts: m
                .artifacts
                .iter()
                .map(ManifestArtifactRecord::from)
                .collect(),
            missing: m.missing.iter().map(MissingSourceRecord::from).collect(),
        }
    }
}

// ============================================================================
// EvidencePackRecord: what the Swift shell gets back
// ============================================================================

/// A built evidence pack, ready for [`super::CloudHandle::build_evidence_pack`]'s
/// caller to save via `NSSavePanel`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct EvidencePackRecord {
    /// The complete zip file bytes - write these verbatim to disk.
    pub zip_bytes: Vec<u8>,
    pub manifest: EvidenceManifestRecord,
    /// Whether the manifest carries a real ES256 signature
    /// (`manifest.sig.json` inside the zip). `false` is an honest,
    /// non-error outcome (no device attached) - the panel MUST label this
    /// UNSIGNED, never claim signed.
    pub signed: bool,
    /// Whether the `console_evidence_built` record was journaled
    /// (best-effort: a journal failure does not fail the whole build, since
    /// the pack itself was already successfully produced - see
    /// [`super::CloudHandle::build_evidence_pack`]'s own doc).
    pub journaled: bool,
}

// ============================================================================
// error taxonomy
// ============================================================================

/// Every failure mode [`super::CloudHandle::build_evidence_pack`] can
/// surface, fail-closed throughout (06 §0.5). Collapsed from
/// [`genaryx_connectors::EvidenceBuildError`]'s three variants - nested
/// error detail folds to a plain `String` (`.to_string()`), matching this
/// crate's established convention for a terminal, display-only nested error
/// (e.g. `IdryxError::RescanUnavailable { reason: String }`).
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum EvidenceError {
    /// Not one requested source could be gathered - a pack of nothing is not
    /// evidence. Never partially returned; the operator must enable at
    /// least one resolvable source and retry.
    #[error("evidence pack has no artifacts: every requested source failed or was disabled")]
    NoArtifacts,
    /// A GENUINE manifest-signing failure - NOT "no device attached" (that
    /// yields an honestly-unsigned pack via [`EvidencePackRecord::signed`]
    /// `= false`, never this error). Fail-closed: no pack is produced.
    #[error("evidence manifest signing failed: {reason}")]
    Sign { reason: String },
    /// The manifest could not be serialized, or the zip could not be
    /// assembled (an unsafe/duplicate artifact filename, a zip-writer
    /// failure).
    #[error("evidence assembly failed: {reason}")]
    Assemble { reason: String },
}

impl From<genaryx_connectors::EvidenceBuildError> for EvidenceError {
    fn from(e: genaryx_connectors::EvidenceBuildError) -> Self {
        use genaryx_connectors::EvidenceBuildError as E;
        match e {
            E::NoArtifacts => EvidenceError::NoArtifacts,
            E::Sign(inner) => EvidenceError::Sign {
                reason: inner.to_string(),
            },
            E::Assemble(inner) => EvidenceError::Assemble {
                reason: inner.to_string(),
            },
        }
    }
}

// ============================================================================
// helpers
// ============================================================================

/// `None` for a missing or whitespace-only optional field - defense in depth
/// at the FFI boundary (see the module doc's "Optional sources"). Consumes
/// `s` and hands the ORIGINAL (untrimmed) string back on the `Some` path -
/// this only decides presence, it never mutates the value itself.
pub(super) fn non_blank(s: Option<String>) -> Option<String> {
    s.and_then(|s| if s.trim().is_empty() { None } else { Some(s) })
}

/// `"sha256:<hex>"` over `bytes` - the SAME format
/// `genaryx_core::evidence`'s own (private) `sha256_hex` uses for every
/// artifact's own hash, reproduced here (rather than exposed from core) so
/// [`super::CloudHandle::build_evidence_pack`] can hash the ASSEMBLED zip for
/// its `console_evidence_built` journal entry - a different input (the whole
/// pack) to the same well-known digest shape.
pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    format!("sha256:{:x}", h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_blank_treats_missing_and_whitespace_as_none() {
        assert_eq!(non_blank(None), None);
        assert_eq!(non_blank(Some(String::new())), None);
        assert_eq!(non_blank(Some("   ".to_string())), None);
        assert_eq!(
            non_blank(Some("/a/path".to_string())),
            Some("/a/path".to_string())
        );
        // Preserves internal whitespace/case verbatim - only presence is
        // decided here, not content normalization.
        assert_eq!(
            non_blank(Some("  /a/path  ".to_string())),
            Some("  /a/path  ".to_string())
        );
    }

    #[test]
    fn manifest_artifact_record_mirrors_every_field() {
        let a = ManifestArtifact {
            name: "Qryx crypto evidence (CNSA)".to_string(),
            filename: "qryx-evidence.json".to_string(),
            content_type: "application/json".to_string(),
            source: "qryx scan --format evidence".to_string(),
            tool_version: Some("0.4.0".to_string()),
            verify_status: Some("VERIFIED (ml-dsa-65)".to_string()),
            sha256: "sha256:abc123".to_string(),
            size_bytes: 42,
        };
        let record = ManifestArtifactRecord::from(&a);
        assert_eq!(record.name, a.name);
        assert_eq!(record.filename, a.filename);
        assert_eq!(record.content_type, a.content_type);
        assert_eq!(record.source, a.source);
        assert_eq!(record.tool_version, a.tool_version);
        assert_eq!(record.verify_status, a.verify_status);
        assert_eq!(record.sha256, a.sha256);
        assert_eq!(record.size_bytes, a.size_bytes);
    }

    #[test]
    fn missing_source_record_mirrors_every_field() {
        let m = MissingSource {
            name: "Agent-BOM (CycloneDX)".to_string(),
            reason: "idryx binary not found".to_string(),
        };
        let record = MissingSourceRecord::from(&m);
        assert_eq!(record.name, m.name);
        assert_eq!(record.reason, m.reason);
    }

    #[test]
    fn evidence_manifest_record_renames_operator_and_mirrors_the_rest() {
        let manifest = EvidenceManifest::build(
            &[],
            vec![MissingSource {
                name: "X".to_string(),
                reason: "Y".to_string(),
            }],
            "2026-07-17T10:00:00Z",
            "user://acme.local/alice",
            "acme",
        );
        let record = EvidenceManifestRecord::from(&manifest);
        assert_eq!(record.pack_version, manifest.pack_version);
        assert_eq!(record.generated_at, manifest.generated_at);
        assert_eq!(record.operator_name, manifest.operator);
        assert_eq!(record.org, manifest.org);
        assert_eq!(record.artifacts.len(), 0);
        assert_eq!(record.missing.len(), 1);
        assert_eq!(record.missing[0].name, "X");
    }

    #[test]
    fn evidence_error_from_connector_no_artifacts() {
        let err = EvidenceError::from(genaryx_connectors::EvidenceBuildError::NoArtifacts);
        assert!(matches!(err, EvidenceError::NoArtifacts));
    }

    #[test]
    fn sha256_hex_is_deterministic_and_prefixed() {
        let a = sha256_hex(b"hello evidence");
        let b = sha256_hex(b"hello evidence");
        assert_eq!(a, b);
        assert!(a.starts_with("sha256:"));
        assert_ne!(a, sha256_hex(b"different bytes"));
    }
}
