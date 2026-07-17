//! Evidence Center assembly (docs/PHASE4.md W3): collect a set of governance
//! artifacts (Cloud compliance evidence + audit verdict, Qryx CNSA evidence +
//! CBOM, TokenFuse FOCUS CSV, idryx Agent-BOM) into ONE portable zip with a
//! signed `manifest.json`. This module is the PURE assembly half - it takes
//! already-gathered artifact bytes plus an already-computed signature and
//! produces the zip. The gathering (calling each connector) and the ES256
//! signing live in `genaryx-connectors` (`evidence.rs`), which has the clients
//! and the device signer; keeping them out of here lets `genaryx-core` stay
//! free of any network/signing dependency, exactly as it is for every other
//! plane (the Store/graph/layout are all pure too).
//!
//! ## What integrity the pack carries
//!
//! Each artifact is captured VERBATIM (the exact bytes the tool emitted), so an
//! artifact that self-verifies still does after extraction: Qryx evidence keeps
//! its embedded ML-DSA signature, a CycloneDX BOM stays byte-identical. The
//! `manifest.json` records every artifact's `sha256` + size + source + the
//! artifact's own verify status, and the manifest itself is what the console
//! signs (ES256, `manifest.sig.json`). So the pack is tamper-evident two ways:
//! the console's signature over the manifest, and each artifact's own hash in
//! that signed manifest.
//!
//! ## Honest about what is inside (06 §0.5)
//!
//! A source that could not be gathered is NOT silently dropped: it is recorded
//! as a [`MissingSource`] in the manifest with its reason, so the pack always
//! states exactly what it contains and what it does not - never a
//! silently-partial pack passed off as complete. Fail-closed elsewhere too: an
//! artifact filename that could escape the archive root (a `/`, `\`, or `..`)
//! is rejected ([`EvidenceError::UnsafeFilename`]) rather than written, and a
//! zip write failure surfaces as [`EvidenceError::Zip`], never a panic.

use std::io::Write;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The pack format identifier written into every manifest.
pub const PACK_VERSION: &str = "genaryx-evidence/v1";

// ---- error -----------------------------------------------------------------

/// Every failure mode assembling an evidence pack can surface. Fail-closed: no
/// panics, and an unsafe artifact filename is a hard error, never written.
#[derive(Debug, thiserror::Error)]
pub enum EvidenceError {
    /// An artifact filename could escape the archive root (contains `/`, `\`,
    /// `..`, or is empty). Rejected before any bytes are written.
    #[error("unsafe artifact filename: {0:?}")]
    UnsafeFilename(String),

    /// Two artifacts want the same zip entry name - the pack would be ambiguous.
    #[error("duplicate artifact filename: {0:?}")]
    DuplicateFilename(String),

    /// The underlying zip writer failed.
    #[error("zip assembly: {0}")]
    Zip(String),

    /// Serializing the manifest to JSON failed.
    #[error("manifest json: {0}")]
    Json(#[from] serde_json::Error),
}

// ---- inputs: artifacts + missing sources -----------------------------------

/// One artifact to include in the pack, with its verbatim bytes. `bytes` is
/// stored as its own zip entry (`filename`); it is never embedded in the
/// manifest (the manifest carries only its `sha256` + size, see
/// [`ManifestArtifact`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Artifact {
    /// Human-readable name, e.g. `"Cloud compliance evidence"`.
    pub name: String,
    /// The entry name inside the zip, e.g. `"compliance-evidence.json"`. Must
    /// be a bare filename (no path separators).
    pub filename: String,
    /// MIME-ish type, e.g. `"application/json"`, `"text/csv"`.
    pub content_type: String,
    /// How it was produced, e.g. `"tokenfuse cloud GET /v1/compliance/evidence"`.
    pub source: String,
    /// The producing tool's version, when known.
    pub tool_version: Option<String>,
    /// The artifact's OWN self-verification status, when it self-verifies (Qryx
    /// evidence `"VERIFIED (ml-dsa-65 …)"`, the Cloud audit chain verdict), else
    /// `None`.
    pub verify_status: Option<String>,
    /// The exact bytes the tool emitted.
    pub bytes: Vec<u8>,
}

/// A source that was requested but could not be included, recorded honestly in
/// the manifest so the pack never overstates its contents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissingSource {
    pub name: String,
    /// Why it is absent, e.g. `"idryx binary not found"` / `"cloud unreachable"`.
    pub reason: String,
}

// ---- manifest (what gets signed + shown) -----------------------------------

/// One artifact's entry in the signed manifest: everything except the bytes
/// themselves (which live in the artifact's own zip entry).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestArtifact {
    pub name: String,
    pub filename: String,
    pub content_type: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_status: Option<String>,
    /// `"sha256:<hex>"` over the artifact bytes.
    pub sha256: String,
    pub size_bytes: u64,
}

/// The pack manifest: the tamper-evident index the console signs. Field order is
/// fixed (serde emits in declaration order, no maps), so
/// [`EvidenceManifest::canonical_bytes`] is deterministic and safe to sign.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceManifest {
    /// Always [`PACK_VERSION`].
    pub pack_version: String,
    /// UTC ISO-8601 build time (supplied by the caller; core never reads the
    /// wall clock).
    pub generated_at: String,
    /// The console operator principal who built the pack.
    pub operator: String,
    /// The org the evidence is for.
    pub org: String,
    pub artifacts: Vec<ManifestArtifact>,
    /// Sources explicitly NOT included (honest partial pack).
    pub missing: Vec<MissingSource>,
}

impl EvidenceManifest {
    /// Build a manifest from the gathered artifacts + missing sources, hashing
    /// each artifact's bytes. `generated_at` is caller-supplied (UTC ISO-8601).
    pub fn build(
        artifacts: &[Artifact],
        missing: Vec<MissingSource>,
        generated_at: impl Into<String>,
        operator: impl Into<String>,
        org: impl Into<String>,
    ) -> Self {
        let manifest_artifacts = artifacts
            .iter()
            .map(|a| ManifestArtifact {
                name: a.name.clone(),
                filename: a.filename.clone(),
                content_type: a.content_type.clone(),
                source: a.source.clone(),
                tool_version: a.tool_version.clone(),
                verify_status: a.verify_status.clone(),
                sha256: sha256_hex(&a.bytes),
                size_bytes: a.bytes.len() as u64,
            })
            .collect();
        Self {
            pack_version: PACK_VERSION.to_string(),
            generated_at: generated_at.into(),
            operator: operator.into(),
            org: org.into(),
            artifacts: manifest_artifacts,
            missing,
        }
    }

    /// The exact bytes to sign (and to write as `manifest.json`). Deterministic
    /// pretty JSON so a human can read the manifest inside the pack.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, EvidenceError> {
        Ok(serde_json::to_vec_pretty(self)?)
    }
}

/// A detached signature over the manifest's [`EvidenceManifest::canonical_bytes`],
/// self-describing so the pack verifies without external key distribution.
/// Produced by the connectors layer's ES256 device signer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignatureBlock {
    /// The signature algorithm, e.g. `"ES256"`.
    pub alg: String,
    /// Base64 (standard) of the raw signature over the manifest bytes.
    pub signature_b64: String,
    /// Base64 (standard) of the signer's public key (SEC1/SPKI), so the pack is
    /// self-verifying.
    pub public_key_b64: String,
    /// What the signature is over, e.g. `"manifest.json"`.
    pub over: String,
}

// ---- assembly --------------------------------------------------------------

/// Assemble the pack into one zip: `manifest.json` (the signed bytes),
/// `manifest.sig.json` (the [`SignatureBlock`], when signed), and one entry per
/// artifact (its verbatim bytes under its `filename`). `manifest_bytes` MUST be
/// the exact bytes the [`SignatureBlock`] was computed over (pass
/// `manifest.canonical_bytes()`).
///
/// Fail-closed: an unsafe or duplicate artifact filename is rejected before any
/// bytes are written.
pub fn assemble_zip(
    manifest_bytes: &[u8],
    signature: Option<&SignatureBlock>,
    artifacts: &[Artifact],
) -> Result<Vec<u8>, EvidenceError> {
    // Validate every filename up front (reserved manifest names + traversal +
    // duplicates), so we never start writing a pack we cannot finish cleanly.
    let mut seen = std::collections::BTreeSet::new();
    for a in artifacts {
        validate_filename(&a.filename)?;
        if matches!(a.filename.as_str(), "manifest.json" | "manifest.sig.json") {
            return Err(EvidenceError::UnsafeFilename(a.filename.clone()));
        }
        if !seen.insert(a.filename.as_str()) {
            return Err(EvidenceError::DuplicateFilename(a.filename.clone()));
        }
    }

    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut cursor);
        let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o644);

        let z = |e: zip::result::ZipError| EvidenceError::Zip(e.to_string());
        let io = |e: std::io::Error| EvidenceError::Zip(e.to_string());

        zw.start_file("manifest.json", opts).map_err(z)?;
        zw.write_all(manifest_bytes).map_err(io)?;

        if let Some(sig) = signature {
            let sig_bytes = serde_json::to_vec_pretty(sig)?;
            zw.start_file("manifest.sig.json", opts).map_err(z)?;
            zw.write_all(&sig_bytes).map_err(io)?;
        }

        for a in artifacts {
            zw.start_file(&a.filename, opts).map_err(z)?;
            zw.write_all(&a.bytes).map_err(io)?;
        }

        zw.finish().map_err(z)?;
    }
    Ok(cursor.into_inner())
}

// ---- helpers ---------------------------------------------------------------

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("sha256:{:x}", h.finalize())
}

/// Reject anything that is not a bare filename inside the archive root.
fn validate_filename(name: &str) -> Result<(), EvidenceError> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.split(['/', '\\']).any(|seg| seg == "..")
        || name == ".."
    {
        return Err(EvidenceError::UnsafeFilename(name.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn artifact(filename: &str, bytes: &[u8]) -> Artifact {
        Artifact {
            name: "Test artifact".to_string(),
            filename: filename.to_string(),
            content_type: "application/json".to_string(),
            source: "test".to_string(),
            tool_version: Some("1.0".to_string()),
            verify_status: None,
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn manifest_hashes_each_artifact_and_records_missing() {
        let arts = vec![
            artifact("a.json", b"{\"x\":1}"),
            artifact("b.csv", b"col\n1\n"),
        ];
        let missing = vec![MissingSource {
            name: "Agent-BOM".to_string(),
            reason: "idryx binary not found".to_string(),
        }];
        let m = EvidenceManifest::build(&arts, missing, "2026-07-17T10:00:00Z", "op@org", "org");
        assert_eq!(m.pack_version, PACK_VERSION);
        assert_eq!(m.artifacts.len(), 2);
        assert!(m.artifacts[0].sha256.starts_with("sha256:"));
        assert_eq!(m.artifacts[0].size_bytes, 7);
        // The missing source is recorded, not dropped.
        assert_eq!(m.missing.len(), 1);
        assert_eq!(m.missing[0].name, "Agent-BOM");

        // canonical_bytes is deterministic (same manifest -> same bytes to sign).
        assert_eq!(m.canonical_bytes().unwrap(), m.canonical_bytes().unwrap());
    }

    #[test]
    fn assemble_zip_contains_manifest_sig_and_every_artifact() {
        let arts = vec![
            artifact("a.json", b"{\"x\":1}"),
            artifact("b.csv", b"c\n1\n"),
        ];
        let m = EvidenceManifest::build(&arts, vec![], "t", "op", "org");
        let mbytes = m.canonical_bytes().unwrap();
        let sig = SignatureBlock {
            alg: "ES256".to_string(),
            signature_b64: "AAAA".to_string(),
            public_key_b64: "BBBB".to_string(),
            over: "manifest.json".to_string(),
        };
        let zip_bytes = assemble_zip(&mbytes, Some(&sig), &arts).expect("assemble");

        // Read it back: manifest.json, manifest.sig.json, a.json, b.csv all present.
        let mut zr = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).expect("open zip");
        let names: std::collections::BTreeSet<String> = (0..zr.len())
            .map(|i| zr.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(names.contains("manifest.json"));
        assert!(names.contains("manifest.sig.json"));
        assert!(names.contains("a.json"));
        assert!(names.contains("b.csv"));

        // The stored manifest bytes equal what we signed, and a.json is verbatim.
        let mut got = String::new();
        zr.by_name("manifest.json")
            .unwrap()
            .read_to_string(&mut got)
            .unwrap();
        assert_eq!(got.as_bytes(), mbytes.as_slice());
        let mut a = Vec::new();
        zr.by_name("a.json").unwrap().read_to_end(&mut a).unwrap();
        assert_eq!(a, b"{\"x\":1}");
    }

    #[test]
    fn unsigned_pack_omits_the_sig_entry() {
        let arts = vec![artifact("a.json", b"{}")];
        let m = EvidenceManifest::build(&arts, vec![], "t", "op", "org");
        let zip_bytes = assemble_zip(&m.canonical_bytes().unwrap(), None, &arts).unwrap();
        let mut zr = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).unwrap();
        assert!(zr.by_name("manifest.json").is_ok());
        assert!(
            zr.by_name("manifest.sig.json").is_err(),
            "no sig when unsigned"
        );
    }

    #[test]
    fn unsafe_and_reserved_filenames_are_rejected_before_writing() {
        let m_bytes = b"{}".to_vec();
        for bad in ["../escape.json", "a/b.json", "..", "", "manifest.json"] {
            let arts = vec![artifact(bad, b"x")];
            match assemble_zip(&m_bytes, None, &arts) {
                Err(EvidenceError::UnsafeFilename(_)) => {}
                other => panic!("expected UnsafeFilename for {bad:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn duplicate_artifact_filenames_are_rejected() {
        let arts = vec![artifact("dup.json", b"1"), artifact("dup.json", b"2")];
        match assemble_zip(b"{}", None, &arts) {
            Err(EvidenceError::DuplicateFilename(_)) => {}
            other => panic!("expected DuplicateFilename, got {other:?}"),
        }
    }
}
