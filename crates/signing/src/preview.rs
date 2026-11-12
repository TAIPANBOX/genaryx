//! The console's preview key: it opens what wardryx shows a person of a held
//! tool call's arguments (wardryx invariant 51, CLAUDE.md invariant 48).
//!
//! wardryx builds the preview from the argument bytes the grant is bound to,
//! hides secrets and personal data by fixed rules, and seals it with HPKE
//! (RFC 9180, base mode) to the console's public key. It keeps only the
//! ciphertext. This module is the other half: it reads the console's private
//! key, opens a ciphertext under the binding the hold names, and refuses a
//! document whose digest is not the one the hold records.
//!
//! The wire formats are wardryx's (`internal/previewseal`, `internal/preview`):
//! - keys are `hpke-private:<KEM id, 4 hex>:<base64url>` and
//!   `hpke:<KEM id>:<base64url>`; the KEM is X-Wing (ML-KEM-768 with X25519,
//!   0x647a) or DHKEM(X25519, HKDF-SHA256) (0x0020); KDF HKDF-SHA256, AEAD
//!   ChaCha20-Poly1305;
//! - a ciphertext is the encapsulated key followed by the AEAD output, with no
//!   AAD;
//! - the HPKE info is `wardryx/preview-seal/v1` followed by the org, the call's
//!   digest, the preview's digest and the ruleset, each with an 8-byte
//!   big-endian length;
//! - the preview digest is a sha-256 over `wardryx/preview/v1`, an 8-byte
//!   big-endian length, and the canonical document.
//!
//! The interop test opens ciphertexts wardryx itself made, for both KEMs.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hpke::aead::ChaCha20Poly1305;
use hpke::kdf::HkdfSha256;
use hpke::kem::{X25519HkdfSha256, XWing};
use hpke::{Deserializable, Kem as KemTrait, OpModeR, Serializable};
use sha2::{Digest, Sha256};
use std::fmt;

/// The HPKE info's domain tag, as wardryx's `previewseal` writes it.
const INFO_DOMAIN: &[u8] = b"wardryx/preview-seal/v1";

/// The preview digest's domain tag, as wardryx's `preview.Digest` writes it.
const DIGEST_DOMAIN: &[u8] = b"wardryx/preview/v1";

const PRIVATE_PREFIX: &str = "hpke-private:";
const RECIPIENT_PREFIX: &str = "hpke:";

/// The X-Wing KEM id (ML-KEM-768 with X25519).
pub const KEM_XWING: u16 = 0x647a;
/// The DHKEM(X25519, HKDF-SHA256) KEM id.
pub const KEM_X25519: u16 = 0x0020;

/// The largest sealed preview this opens. A preview is at most 16 KiB and the
/// X-Wing encapsulated key 1120 bytes, so anything near this is not one.
pub const MAX_SEALED_BYTES: usize = 32 * 1024;

/// Why a key or a ciphertext could not be used. The text never carries key
/// material or ciphertext.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PreviewError {
    /// The key text is not a preview key of an allowed KEM.
    #[error("the preview key is not usable: {0}")]
    Key(&'static str),
    /// The ciphertext does not open under this key and this binding.
    #[error("the sealed preview does not open under the console key and the hold it names")]
    Open,
    /// It opened, and what it opened is not the preview the hold records.
    #[error("the opened preview is not the one the hold records")]
    Mismatch,
}

/// The console's private preview key.
pub enum PreviewKey {
    /// X-Wing, the default: a ciphertext kept in a backup stays closed to a
    /// later quantum computer.
    XWing(Box<<XWing as KemTrait>::PrivateKey>),
    /// DHKEM(X25519).
    X25519(Box<<X25519HkdfSha256 as KemTrait>::PrivateKey>),
}

/// Never prints key material.
impl fmt::Debug for PreviewKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreviewKey::XWing(_) => f.write_str("PreviewKey::XWing(<redacted>)"),
            PreviewKey::X25519(_) => f.write_str("PreviewKey::X25519(<redacted>)"),
        }
    }
}

/// What a ciphertext is bound to: the hold's org, the digest of its call, the
/// digest of its preview and the preview's ruleset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub org: String,
    pub tool_call_digest: String,
    pub preview_digest: String,
    pub ruleset: String,
}

/// The HPKE info for a binding: the domain tag, then each field with an 8-byte
/// big-endian length, so no field's bytes can move into its neighbour.
pub fn info(b: &Binding) -> Vec<u8> {
    let mut out = INFO_DOMAIN.to_vec();
    for field in [&b.org, &b.tool_call_digest, &b.preview_digest, &b.ruleset] {
        out.extend_from_slice(&(field.len() as u64).to_be_bytes());
        out.extend_from_slice(field.as_bytes());
    }
    out
}

/// The preview digest (`pvd`) of a canonical document.
pub fn preview_digest(document: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(DIGEST_DOMAIN);
    h.update((document.len() as u64).to_be_bytes());
    h.update(document);
    hex_lower(&h.finalize())
}

/// Whether `s` has the shape of a preview digest: 64 lower-case hex digits.
pub fn is_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Split `hpke-private:647a:<b64>` into the KEM id and the key bytes.
fn parse_key_text(text: &str, prefix: &str) -> Result<(u16, Vec<u8>), PreviewError> {
    let rest = text.trim().strip_prefix(prefix).ok_or(PreviewError::Key(
        "it does not start with the expected prefix",
    ))?;
    let (id, b64) = rest
        .split_once(':')
        .ok_or(PreviewError::Key("it is not <prefix><KEM id>:<base64url>"))?;
    if id.len() != 4 {
        return Err(PreviewError::Key("the KEM id is not four hex digits"));
    }
    let id = u16::from_str_radix(id, 16).map_err(|_| PreviewError::Key("the KEM id is not hex"))?;
    if id != KEM_XWING && id != KEM_X25519 {
        return Err(PreviewError::Key(
            "the KEM is not X-Wing (647a) or X25519 (0020)",
        ));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(b64)
        .map_err(|_| PreviewError::Key("the key is not base64url without padding"))?;
    Ok((id, bytes))
}

impl PreviewKey {
    /// Read a private preview key from its text form.
    pub fn parse(text: &str) -> Result<Self, PreviewError> {
        let (id, bytes) = parse_key_text(text, PRIVATE_PREFIX)?;
        match id {
            KEM_XWING => <XWing as KemTrait>::PrivateKey::from_bytes(&bytes)
                .map(|k| PreviewKey::XWing(Box::new(k)))
                .map_err(|_| PreviewError::Key("the key does not parse for X-Wing")),
            _ => <X25519HkdfSha256 as KemTrait>::PrivateKey::from_bytes(&bytes)
                .map(|k| PreviewKey::X25519(Box::new(k)))
                .map_err(|_| PreviewError::Key("the key does not parse for X25519")),
        }
    }

    /// Make a new key pair and return the private key's text and the
    /// recipient's, the value wardryx reads as `WARDRYX_PREVIEW_RECIPIENT`.
    pub fn generate(kem: u16) -> Result<(String, String), PreviewError> {
        match kem {
            KEM_XWING => {
                let (sk, pk) = XWing::gen_keypair();
                Ok((
                    key_text(PRIVATE_PREFIX, KEM_XWING, &sk.to_bytes()),
                    key_text(RECIPIENT_PREFIX, KEM_XWING, &pk.to_bytes()),
                ))
            }
            KEM_X25519 => {
                let (sk, pk) = X25519HkdfSha256::gen_keypair();
                Ok((
                    key_text(PRIVATE_PREFIX, KEM_X25519, &sk.to_bytes()),
                    key_text(RECIPIENT_PREFIX, KEM_X25519, &pk.to_bytes()),
                ))
            }
            _ => Err(PreviewError::Key(
                "the KEM is not X-Wing (647a) or X25519 (0020)",
            )),
        }
    }

    /// The recipient text of this key's public half.
    pub fn recipient(&self) -> String {
        match self {
            PreviewKey::XWing(sk) => {
                key_text(RECIPIENT_PREFIX, KEM_XWING, &XWing::sk_to_pk(sk).to_bytes())
            }
            PreviewKey::X25519(sk) => key_text(
                RECIPIENT_PREFIX,
                KEM_X25519,
                &X25519HkdfSha256::sk_to_pk(sk).to_bytes(),
            ),
        }
    }

    /// Open a sealed preview under `binding` and check that what it holds is
    /// the preview the binding names. The returned bytes are the canonical
    /// document, JSON.
    pub fn open(&self, binding: &Binding, sealed: &[u8]) -> Result<Vec<u8>, PreviewError> {
        if sealed.len() > MAX_SEALED_BYTES {
            return Err(PreviewError::Open);
        }
        let info = info(binding);
        let document = match self {
            PreviewKey::XWing(sk) => open_with::<XWing>(sk, &info, sealed)?,
            PreviewKey::X25519(sk) => open_with::<X25519HkdfSha256>(sk, &info, sealed)?,
        };
        if preview_digest(&document) != binding.preview_digest {
            return Err(PreviewError::Mismatch);
        }
        Ok(document)
    }
}

fn open_with<K: KemTrait>(
    sk: &K::PrivateKey,
    info: &[u8],
    sealed: &[u8],
) -> Result<Vec<u8>, PreviewError> {
    let enc_len = <K::EncappedKey as Serializable>::size();
    if sealed.len() <= enc_len {
        return Err(PreviewError::Open);
    }
    let (enc, ct) = sealed.split_at(enc_len);
    let encapped = K::EncappedKey::from_bytes(enc).map_err(|_| PreviewError::Open)?;
    hpke::single_shot_open::<ChaCha20Poly1305, HkdfSha256, K>(
        &OpModeR::Base,
        sk,
        &encapped,
        info,
        ct,
        &[],
    )
    .map_err(|_| PreviewError::Open)
}

fn key_text(prefix: &str, kem: u16, bytes: &[u8]) -> String {
    format!("{prefix}{kem:04x}:{}", URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;

    /// Ciphertexts wardryx's `internal/previewseal` made, with throwaway test
    /// keys, for both KEMs.
    const INTEROP: &str = include_str!("../testdata/preview-interop.json");

    struct Vector {
        suite: String,
        key: PreviewKey,
        recipient: String,
        binding: Binding,
        info: Vec<u8>,
        sealed: Vec<u8>,
        document: String,
    }

    fn vectors() -> Vec<Vector> {
        let v: serde_json::Value = serde_json::from_str(INTEROP).expect("vectors parse");
        v["vectors"]
            .as_array()
            .expect("vectors")
            .iter()
            .map(|x| {
                let s = |k: &str| x[k].as_str().expect(k).to_string();
                Vector {
                    suite: s("suite"),
                    key: PreviewKey::parse(&s("private")).expect("the vector's key parses"),
                    recipient: s("recipient"),
                    binding: Binding {
                        org: s("org"),
                        tool_call_digest: s("tool_call_digest"),
                        preview_digest: s("pvd"),
                        ruleset: s("ruleset"),
                    },
                    info: STANDARD.decode(s("info_b64")).expect("info"),
                    sealed: STANDARD.decode(s("sealed_b64")).expect("sealed"),
                    document: s("document"),
                }
            })
            .collect()
    }

    #[test]
    fn a_preview_wardryx_sealed_opens_under_the_console_key_for_both_kems() {
        let vs = vectors();
        assert_eq!(vs.len(), 2);
        for v in &vs {
            let doc = v.key.open(&v.binding, &v.sealed).expect(&v.suite);
            assert_eq!(
                String::from_utf8(doc).expect("utf-8"),
                v.document,
                "{}",
                v.suite
            );
        }
        assert!(vs.iter().any(|v| v.suite.starts_with("kem=0x647a")));
        assert!(vs.iter().any(|v| v.suite.starts_with("kem=0x0020")));
    }

    #[test]
    fn the_info_and_the_digest_are_wardryxs_byte_for_byte() {
        for v in vectors() {
            assert_eq!(info(&v.binding), v.info, "{}", v.suite);
            assert_eq!(
                preview_digest(v.document.as_bytes()),
                v.binding.preview_digest
            );
            assert_eq!(v.key.recipient(), v.recipient, "{}", v.suite);
        }
    }

    #[test]
    fn a_ciphertext_does_not_open_under_another_binding_or_another_key() {
        for v in vectors() {
            for other in [
                Binding {
                    org: "other".into(),
                    ..v.binding.clone()
                },
                Binding {
                    tool_call_digest: "b".repeat(64),
                    ..v.binding.clone()
                },
                Binding {
                    ruleset: "r0".into(),
                    ..v.binding.clone()
                },
            ] {
                assert_eq!(
                    v.key.open(&other, &v.sealed),
                    Err(PreviewError::Open),
                    "{}",
                    v.suite
                );
            }
            let kem = if v.suite.starts_with("kem=0x647a") {
                KEM_XWING
            } else {
                KEM_X25519
            };
            let (fresh, _) = PreviewKey::generate(kem).expect("generate");
            let fresh = PreviewKey::parse(&fresh).expect("parse");
            assert_eq!(fresh.open(&v.binding, &v.sealed), Err(PreviewError::Open));
        }
    }

    #[test]
    fn a_ciphertext_that_opens_to_another_document_is_refused_for_its_digest() {
        // wardryx made this one by hand: the document sealed under a binding
        // that names another document's digest. It opens, and the digest check
        // is what refuses it.
        let v: serde_json::Value = serde_json::from_str(INTEROP).expect("vectors parse");
        let m = &v["mismatch"];
        let s = |k: &str| m[k].as_str().expect(k).to_string();
        let key = PreviewKey::parse(&s("private")).expect("key");
        let binding = Binding {
            org: s("org"),
            tool_call_digest: s("tool_call_digest"),
            preview_digest: s("pvd"),
            ruleset: s("ruleset"),
        };
        let sealed = STANDARD.decode(s("sealed_b64")).expect("sealed");
        assert_eq!(key.open(&binding, &sealed), Err(PreviewError::Mismatch));
    }

    #[test]
    fn hostile_ciphertexts_are_refused_without_panic() {
        for v in vectors() {
            for n in 0..v.sealed.len() {
                assert!(
                    v.key.open(&v.binding, &v.sealed[..n]).is_err(),
                    "prefix {n}"
                );
            }
            for i in (0..v.sealed.len()).step_by(7) {
                let mut flipped = v.sealed.clone();
                flipped[i] ^= 0x01;
                assert!(v.key.open(&v.binding, &flipped).is_err(), "flip {i}");
            }
            assert!(
                v.key
                    .open(&v.binding, &vec![0u8; MAX_SEALED_BYTES + 1])
                    .is_err()
            );
        }
    }

    #[test]
    fn only_the_two_allowed_kems_and_well_formed_keys_are_accepted() {
        for bad in [
            "",
            "hpke-private:",
            "hpke-private:0020",
            "hpke-private:zzzz:AAAA",
            "hpke-private:0010:AAAA",
            "hpke-private:0020:!!!",
            "hpke:0020:AAAA",
            "hpke-private:0020:AAAA",
        ] {
            assert!(PreviewKey::parse(bad).is_err(), "{bad:?}");
        }
        assert!(PreviewKey::generate(0x0010).is_err());
        let (private, recipient) = PreviewKey::generate(KEM_XWING).expect("generate");
        assert!(private.starts_with("hpke-private:647a:"));
        assert!(recipient.starts_with("hpke:647a:"));
        assert_eq!(
            PreviewKey::parse(&private).expect("parse").recipient(),
            recipient
        );
    }

    #[test]
    fn the_key_never_prints_its_material() {
        let (private, _) = PreviewKey::generate(KEM_X25519).expect("generate");
        let key = PreviewKey::parse(&private).expect("parse");
        let shown = format!("{key:?}");
        let b64 = private.rsplit(':').next().expect("b64");
        assert!(!shown.contains(b64), "{shown}");
        assert!(shown.contains("redacted"));
    }

    #[test]
    fn a_digest_has_the_shape_it_checks() {
        assert!(is_digest(&"a".repeat(64)));
        for bad in [
            "",
            "none",
            &"A".repeat(64),
            &"a".repeat(63),
            &"g".repeat(64),
        ] {
            assert!(!is_digest(bad), "{bad}");
        }
    }
}
