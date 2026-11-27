//! `build_evidence_pack` end to end against a stub Cloud and stand-in plane
//! CLIs: every requested source either becomes an artifact or is named in the
//! manifest as missing with its reason, a pack of nothing is refused, an
//! unpaired console builds an honestly unsigned pack, and a signer that fails
//! fails the whole build rather than hand back a pack that looks signed.

use std::path::Path;

use genaryx_connectors::{
    CloudClient, ConnectorError, EvidenceBuildError, EvidenceInputs, EvidencePack, QryxClient,
    TokenfuseClient, build_evidence_pack,
};
use genaryx_signing::{Assurance, Es256Signer, SigningError, SoftwareSigner};

use crate::loopback::{closed_base, serve};
use crate::stand_in::{is_busy, scratch_dir, script};

const QRYX: &str = r##"case "$3" in
  evidence) printf '{"tool":"qryx","digest":"sha256:00"}' ;;
  cbom) printf '{"bomFormat":"CycloneDX"}' ;;
esac
"##;

const QRYX_NO_CBOM: &str = r##"case "$3" in
  evidence) printf '{"tool":"qryx","digest":"sha256:00"}' ;;
  cbom) echo 'cbom writer crashed' >&2; exit 1 ;;
esac
"##;

const FOCUS: &str = r##"out=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--out" ]; then out="$a"; fi
  prev="$a"
done
printf 'BilledCost\n0.5\n' > "$out"
"##;

/// Whether `needle` occurs in the zip's bytes. Entry names are stored
/// uncompressed in the local headers, so this tells which files a pack holds.
fn zip_names(zip: &[u8], needle: &str) -> bool {
    zip.windows(needle.len()).any(|w| w == needle.as_bytes())
}

/// Build, again while a stand-in was only refused as busy (see `stand_in`).
async fn build_settled<'a>(
    cloud: &CloudClient,
    make: impl Fn() -> EvidenceInputs<'a>,
) -> Result<EvidencePack, EvidenceBuildError> {
    for _ in 0..50 {
        let r = build_evidence_pack(cloud, make()).await;
        let busy = matches!(&r, Ok(p) if p.manifest.missing.iter().any(|m| is_busy(&m.reason)));
        if !busy {
            return r;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    build_evidence_pack(cloud, make()).await
}

fn names(pack: &EvidencePack) -> Vec<String> {
    pack.manifest
        .artifacts
        .iter()
        .map(|a| a.name.clone())
        .collect()
}

#[tokio::test]
async fn an_unpaired_console_builds_an_honestly_unsigned_cloud_pack() {
    let stub = serve(vec![
        (
            "GET /v1/compliance/evidence",
            "200 OK",
            r#"{"controls":["eu-ai-act-12"]}"#.to_string(),
        ),
        (
            "GET /v1/audit/verify",
            "200 OK",
            r#"{"ok":false,"break_index":3}"#.to_string(),
        ),
    ]);
    let cloud = CloudClient::new(stub.base.clone(), "k").expect("client");
    let pack = build_settled(&cloud, || EvidenceInputs {
        operator: "user://acme.example/pat",
        org: "acme",
        generated_at: "2026-11-26T00:00:00Z",
        include_cloud: true,
        qryx: None,
        idryx: None,
        tokenfuse: None,
    })
    .await
    .expect("a pack");

    assert!(!pack.signed, "no device, so no signature is claimed");
    assert_eq!(
        names(&pack),
        ["Cloud compliance evidence", "Cloud audit-chain verdict"]
    );
    assert_eq!(
        pack.manifest.artifacts[1].verify_status.as_deref(),
        Some("audit chain BROKEN at index 3")
    );
    assert!(pack.manifest.missing.is_empty());
    assert_eq!(pack.manifest.operator, "user://acme.example/pat");
    assert_eq!(pack.manifest.org, "acme");
    assert_eq!(pack.manifest.generated_at, "2026-11-26T00:00:00Z");
    assert!(pack.zip_bytes.starts_with(b"PK"), "a zip");
    assert!(zip_names(&pack.zip_bytes, "manifest.json"));
    assert!(!zip_names(&pack.zip_bytes, "manifest.sig.json"));
}

#[tokio::test]
async fn every_source_lands_as_an_artifact_or_as_a_named_missing_one() {
    let stub = serve(vec![
        (
            "GET /v1/compliance/evidence",
            "500 Internal Server Error",
            "compliance store offline".to_string(),
        ),
        ("GET /v1/audit/verify", "200 OK", r#"{"ok":true}"#.to_string()),
    ]);
    let dir = scratch_dir("evidence-all");
    let qryx = QryxClient::new(script(&dir, "qryx", QRYX));
    let idryx_bin = script(&dir, "idryx", "printf '{\"bomFormat\":\"CycloneDX\"}'\n");
    let tf = TokenfuseClient::new(script(&dir, "tokenfuse", FOCUS));
    let key = dir.join("mldsa.pem");
    let loads = [("tokenfuse", "/bus/tokenfuse.ndjson")];
    let mut cloud = CloudClient::new(stub.base.clone(), "k").expect("client");
    cloud.attach_device(
        "dev-ev",
        "t",
        Box::new(SoftwareSigner::generate().expect("key")),
    );

    let pack = build_settled(&cloud, || EvidenceInputs {
        operator: "user://acme.example/pat",
        org: "acme",
        generated_at: "2026-11-26T00:00:00Z",
        include_cloud: true,
        qryx: Some((&qryx, Path::new("/src"), Some(key.as_path()))),
        idryx: Some((idryx_bin.as_path(), &loads[..])),
        tokenfuse: Some((&tf, dir.as_path(), None, None)),
    })
    .await
    .expect("a pack");
    std::fs::remove_dir_all(&dir).ok();

    assert_eq!(
        names(&pack),
        [
            "Cloud audit-chain verdict",
            "Qryx crypto evidence (CNSA)",
            "CBOM (CycloneDX)",
            "Agent-BOM (CycloneDX)",
            "TokenFuse FOCUS cost export",
        ]
    );
    let a = &pack.manifest.artifacts;
    assert_eq!(
        a[0].verify_status.as_deref(),
        Some("audit chain VERIFIED end-to-end")
    );
    assert!(
        a[1].verify_status
            .as_deref()
            .is_some_and(|s| s.contains("ML-DSA signature")),
        "{:?}",
        a[1].verify_status
    );
    assert_eq!(a[4].content_type, "text/csv");
    assert_eq!(a[4].size_bytes, "BilledCost\n0.5\n".len() as u64);
    assert_eq!(pack.manifest.missing.len(), 1);
    assert_eq!(pack.manifest.missing[0].name, "Cloud compliance evidence");
    assert!(
        pack.manifest.missing[0].reason.contains("500"),
        "the reason is the error itself: {}",
        pack.manifest.missing[0].reason
    );
    assert!(pack.signed);
    assert!(zip_names(&pack.zip_bytes, "manifest.sig.json"));
    assert!(zip_names(&pack.zip_bytes, "focus.csv"));
}

#[tokio::test]
async fn failed_tools_are_named_with_their_reason_and_a_bare_break_is_still_a_break() {
    let stub = serve(vec![(
        "GET /v1/audit/verify",
        "200 OK",
        r#"{"ok":false}"#.to_string(),
    )]);
    let dir = scratch_dir("evidence-partial");
    let qryx = QryxClient::new(script(&dir, "qryx", QRYX_NO_CBOM));
    let absent = dir.join("absent-idryx");
    let tf = TokenfuseClient::new(dir.join("absent-tokenfuse"));
    let cloud = CloudClient::new(stub.base.clone(), "k").expect("client");

    let pack = build_settled(&cloud, || EvidenceInputs {
        operator: "op",
        org: "acme",
        generated_at: "2026-11-26T00:00:00Z",
        include_cloud: true,
        qryx: Some((&qryx, Path::new("/src"), None)),
        idryx: Some((absent.as_path(), &[])),
        tokenfuse: Some((&tf, dir.as_path(), Some("2026-11-01T00:00:00Z"), None)),
    })
    .await
    .expect("a partial pack");
    std::fs::remove_dir_all(&dir).ok();

    assert_eq!(
        names(&pack),
        ["Cloud audit-chain verdict", "Qryx crypto evidence (CNSA)"]
    );
    let a = &pack.manifest.artifacts;
    assert_eq!(a[0].verify_status.as_deref(), Some("audit chain BROKEN"));
    assert_eq!(
        a[1].verify_status.as_deref(),
        Some("self-verifying (embedded digest); run `qryx verify-evidence`")
    );
    let missing: Vec<(&str, &str)> = pack
        .manifest
        .missing
        .iter()
        .map(|m| (m.name.as_str(), m.reason.as_str()))
        .collect();
    assert_eq!(missing.len(), 4, "{missing:?}");
    assert_eq!(missing[0].0, "Cloud compliance evidence");
    assert!(missing[0].1.contains("404"), "{missing:?}");
    assert_eq!(missing[1], ("CBOM (CycloneDX)", "qryx exited 1: cbom writer crashed"));
    assert_eq!(missing[2].0, "Agent-BOM (CycloneDX)");
    assert!(missing[2].1.contains("spawn"), "{missing:?}");
    assert_eq!(missing[3].0, "TokenFuse FOCUS cost export");
    assert!(missing[3].1.starts_with("tokenfuse spawn"), "{missing:?}");
}

#[tokio::test]
async fn a_pack_of_nothing_is_refused() {
    let unreachable = CloudClient::new(closed_base(), "k").expect("client");
    let absent = QryxClient::new("/nonexistent/genaryx-test/qryx");
    let r = build_evidence_pack(
        &unreachable,
        EvidenceInputs {
            operator: "op",
            org: "acme",
            generated_at: "2026-11-26T00:00:00Z",
            include_cloud: true,
            qryx: Some((&absent, Path::new("/src"), None)),
            idryx: None,
            tokenfuse: None,
        },
    )
    .await;
    assert!(matches!(r, Err(EvidenceBuildError::NoArtifacts)));

    let nothing_asked = build_evidence_pack(
        &unreachable,
        EvidenceInputs {
            operator: "op",
            org: "acme",
            generated_at: "2026-11-26T00:00:00Z",
            include_cloud: false,
            qryx: None,
            idryx: None,
            tokenfuse: None,
        },
    )
    .await;
    assert!(matches!(nothing_asked, Err(EvidenceBuildError::NoArtifacts)));
}

/// A device whose key can no longer sign: the hardware went away mid-session.
struct UnpluggedSigner;

impl Es256Signer for UnpluggedSigner {
    fn assurance(&self) -> Assurance {
        Assurance::SoftwareP256
    }
    fn public_key_x963(&self) -> Result<Vec<u8>, SigningError> {
        Err(SigningError::KeyExport("the key is gone".into()))
    }
    fn sign_raw(&self, _message: &[u8]) -> Result<[u8; 64], SigningError> {
        Err(SigningError::Signing("the token was unplugged".into()))
    }
}

#[tokio::test]
async fn a_signer_that_fails_fails_the_build_never_an_unsigned_pack() {
    let stub = serve(vec![(
        "GET /v1/audit/verify",
        "200 OK",
        r#"{"ok":true}"#.to_string(),
    )]);
    let mut cloud = CloudClient::new(stub.base.clone(), "k").expect("client");
    cloud.attach_device("dev-x", "t", Box::new(UnpluggedSigner));
    let r = build_evidence_pack(
        &cloud,
        EvidenceInputs {
            operator: "op",
            org: "acme",
            generated_at: "2026-11-26T00:00:00Z",
            include_cloud: true,
            qryx: None,
            idryx: None,
            tokenfuse: None,
        },
    )
    .await;
    match r {
        Err(EvidenceBuildError::Sign(ConnectorError::Signing(e))) => {
            assert!(e.to_string().contains("the token was unplugged"), "{e}");
        }
        Err(other) => panic!("expected a signing failure, got {other:?}"),
        Ok(_) => panic!("a pack was built although its signature failed"),
    }
}
