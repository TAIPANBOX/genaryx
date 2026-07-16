//! ML-DSA (FIPS 204) signature verification.
//!
//! Phase-0 spike #4 (06 §7). Genaryx never signs with ML-DSA itself; Qryx signs
//! evidence packs (07 §4.5) and the license issuer signs offline entitlement files
//! (decision D3/D8). This module only verifies, and stays fail-closed (06 §0.5):
//! any parse or verification failure is `Err` or `Ok(false)`, never a panic and
//! never a silent pass.
//!
//! Qryx embeds the public key as a PKCS#8 SubjectPublicKeyInfo (SPKI) DER blob and
//! signs the document's blended digest directly (07 §4.5: "sha256 digest of the
//! document with a blended digest field, SPKI embedded, self-verifying"). [`verify`]
//! tries SPKI first and falls back to the crate's raw fixed-size encoded key, so a
//! caller holding only a bare key (e.g. an offline-license key with no DER wrapper)
//! still works.
//!
//! Crate choice: RustCrypto `ml-dsa` 0.1, not `fips204`. Both are pure Rust, forbid
//! `unsafe`, and implement FIPS-204 final (not the earlier draft). `ml-dsa` wins on
//! two points that matter here: it is maintained as part of RustCrypto's
//! `signatures` monorepo (the same family as the `ecdsa`/`p256` crates implied by
//! the ES256 device-pairing path in this crate's top-level docs) rather than by a
//! single maintainer, and its `pkcs8` feature (on by default) parses SPKI/PKCS#8
//! directly into a `VerifyingKey`. That is exactly Qryx's key format, so this
//! module never hand-rolls ASN.1 parsing. `fips204` has no SPKI/PKCS8 support at
//! all: using it would have meant writing and trusting our own DER decoder for the
//! gap noted in the spike brief.

use ml_dsa::pkcs8::der::AnyRef;
use ml_dsa::pkcs8::spki::{AssociatedAlgorithmIdentifier, SubjectPublicKeyInfoRef};
use ml_dsa::signature::Verifier as _;
use ml_dsa::{
    EncodedVerifyingKey, MlDsa44, MlDsa65, MlDsa87, MlDsaParams, Signature, VerifyingKey,
};

/// Which ML-DSA parameter set (security level) a key/signature pair was made for.
///
/// Qryx defaults to ML-DSA-65 for evidence packs (07 §4.5 examples).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamSet {
    MlDsa44,
    MlDsa65,
    MlDsa87,
}

/// Verify an ML-DSA signature over `message` made with the private key matching
/// `public_key`.
///
/// `public_key` may be a DER-encoded PKCS#8 SubjectPublicKeyInfo (SPKI), which is
/// how Qryx embeds keys in evidence packs (07 §4.5), or the crate's raw fixed-size
/// encoded verifying key. SPKI is tried first; if it does not parse, or parses but
/// its algorithm OID does not match `param_set`, this falls back to raw-key
/// decoding rather than erroring out early.
///
/// Returns `Ok(true)` only when `public_key` decodes for `param_set` and
/// `signature` cryptographically verifies over `message`. Returns `Ok(false)` for
/// a structurally valid but non-matching signature (tampered message, tampered
/// signature, wrong key). Returns `Err` only when neither input can be decoded at
/// all (malformed key, wrong-length signature). Never panics: this is the
/// verification gate for evidence and license trust, so every failure mode is an
/// explicit, inspectable result (06 §0.5).
pub fn verify(
    param_set: ParamSet,
    public_key: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<bool, String> {
    match param_set {
        ParamSet::MlDsa44 => verify_with::<MlDsa44>(public_key, message, signature),
        ParamSet::MlDsa65 => verify_with::<MlDsa65>(public_key, message, signature),
        ParamSet::MlDsa87 => verify_with::<MlDsa87>(public_key, message, signature),
    }
}

/// Monomorphic verify for one parameter set. `ml-dsa` fixes the security level as
/// a compile-time type parameter (there is no runtime-generic key/signature type
/// in the crate), so [`verify`] dispatches to one instantiation of this per
/// [`ParamSet`] variant.
fn verify_with<P>(public_key: &[u8], message: &[u8], signature: &[u8]) -> Result<bool, String>
where
    P: MlDsaParams + AssociatedAlgorithmIdentifier<Params = AnyRef<'static>>,
    for<'a> VerifyingKey<P>: TryFrom<SubjectPublicKeyInfoRef<'a>>,
{
    let key = decode_verifying_key::<P>(public_key)?;
    let sig = Signature::<P>::try_from(signature)
        .map_err(|e| format!("ML-DSA signature: bad encoding: {e}"))?;
    Ok(key.verify(message, &sig).is_ok())
}

/// Decode a verifying key, trying SPKI DER first (Qryx's format, 07 §4.5) and
/// falling back to the crate's raw fixed-size key encoding.
fn decode_verifying_key<P>(bytes: &[u8]) -> Result<VerifyingKey<P>, String>
where
    P: MlDsaParams + AssociatedAlgorithmIdentifier<Params = AnyRef<'static>>,
    for<'a> VerifyingKey<P>: TryFrom<SubjectPublicKeyInfoRef<'a>>,
{
    let from_spki = SubjectPublicKeyInfoRef::try_from(bytes)
        .ok()
        .and_then(|spki| VerifyingKey::<P>::try_from(spki).ok());
    if let Some(key) = from_spki {
        return Ok(key);
    }
    let raw = EncodedVerifyingKey::<P>::try_from(bytes).map_err(|_| {
        "ML-DSA public key: not valid SPKI DER and not the raw key length for this parameter set"
            .to_string()
    })?;
    Ok(VerifyingKey::decode(&raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ml_dsa::pkcs8::EncodePublicKey;
    use ml_dsa::signature::Signer as _;
    use ml_dsa::{Generate, Keypair as _, SigningKey};

    /// Round-trip KAT: generate a keypair, sign, verify true; tamper, verify
    /// false. Exercised once per parameter set below.
    fn roundtrip_via_spki<P>(param_set: ParamSet)
    where
        P: MlDsaParams + AssociatedAlgorithmIdentifier<Params = AnyRef<'static>>,
        for<'a> VerifyingKey<P>: TryFrom<SubjectPublicKeyInfoRef<'a>>,
        SigningKey<P>: Generate,
    {
        let sk = SigningKey::<P>::generate();
        let vk = sk.verifying_key();
        let spki_der = vk
            .to_public_key_der()
            .expect("encode verifying key as SPKI DER")
            .to_vec();
        let msg = b"qryx evidence pack digest (blended field)";
        let sig = sk.sign(msg).encode();

        assert_eq!(
            verify(param_set, &spki_der, msg, &sig),
            Ok(true),
            "genuine signature over the signed message must verify"
        );

        let tampered_msg = b"qryx evidence pack digest (blended fielD)";
        assert_eq!(
            verify(param_set, &spki_der, tampered_msg, &sig),
            Ok(false),
            "a one-byte-different message must not verify"
        );

        // Flip a byte inside the leading commitment hash (`c_tilde`) rather than
        // in the trailing encoded hint: hint bytes are range-checked on decode,
        // so a flip there can turn this into a decode error (still fail-closed,
        // covered separately by `truncated_signature_fails_closed`) instead of
        // the clean cryptographic mismatch this test wants to demonstrate.
        let mut tampered_sig = sig.to_vec();
        tampered_sig[0] ^= 0xFF;
        assert_eq!(
            verify(param_set, &spki_der, msg, &tampered_sig),
            Ok(false),
            "a tampered signature must not verify"
        );
    }

    #[test]
    fn roundtrip_ml_dsa_44_via_spki() {
        roundtrip_via_spki::<MlDsa44>(ParamSet::MlDsa44);
    }

    #[test]
    fn roundtrip_ml_dsa_65_via_spki() {
        roundtrip_via_spki::<MlDsa65>(ParamSet::MlDsa65);
    }

    #[test]
    fn roundtrip_ml_dsa_87_via_spki() {
        roundtrip_via_spki::<MlDsa87>(ParamSet::MlDsa87);
    }

    #[test]
    fn roundtrip_via_raw_key_bytes() {
        // Covers the fallback path for callers holding a bare key with no DER
        // wrapper (e.g. an offline-license key, D3/D8).
        let sk = SigningKey::<MlDsa65>::generate();
        let vk = sk.verifying_key();
        let raw = vk.encode();
        let msg = b"offline license entitlement v1";
        let sig = sk.sign(msg).encode();

        assert_eq!(verify(ParamSet::MlDsa65, &raw, msg, &sig), Ok(true));
    }

    #[test]
    fn wrong_key_is_rejected() {
        let signer = SigningKey::<MlDsa65>::generate();
        let other = SigningKey::<MlDsa65>::generate();
        let other_vk_der = other
            .verifying_key()
            .to_public_key_der()
            .expect("encode SPKI")
            .to_vec();
        let msg = b"signed by one key, checked against another";
        let sig = signer.sign(msg).encode();

        assert_eq!(
            verify(ParamSet::MlDsa65, &other_vk_der, msg, &sig),
            Ok(false)
        );
    }

    #[test]
    fn malformed_public_key_fails_closed() {
        let result = verify(ParamSet::MlDsa65, b"not a key", b"msg", &[0u8; 16]);
        assert!(result.is_err(), "garbage key input must be Err, not panic");
    }

    #[test]
    fn empty_public_key_fails_closed() {
        let result = verify(ParamSet::MlDsa44, &[], b"msg", &[0u8; 16]);
        assert!(result.is_err());
    }

    #[test]
    fn truncated_signature_fails_closed() {
        let sk = SigningKey::<MlDsa65>::generate();
        let spki_der = sk
            .verifying_key()
            .to_public_key_der()
            .expect("encode SPKI")
            .to_vec();
        let result = verify(ParamSet::MlDsa65, &spki_der, b"msg", &[1, 2, 3]);
        assert!(
            result.is_err(),
            "short/invalid signature encoding must be Err, not panic"
        );
    }

    #[test]
    fn param_set_mismatch_fails_closed() {
        // A key/signature genuinely produced under ML-DSA-65 must not verify (as
        // true or as a spurious size match) when checked as ML-DSA-44: SPKI OID
        // assertion rejects it, and the raw-key fallback length check rejects it
        // too, so the only honest outcome is Err.
        let sk = SigningKey::<MlDsa65>::generate();
        let spki_der = sk
            .verifying_key()
            .to_public_key_der()
            .expect("encode SPKI")
            .to_vec();
        let msg = b"cross param set";
        let sig = sk.sign(msg).encode();

        let result = verify(ParamSet::MlDsa44, &spki_der, msg, &sig);
        assert!(result.is_err());
    }
}
