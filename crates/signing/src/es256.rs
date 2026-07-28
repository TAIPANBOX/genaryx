//! ES256 (ECDSA P-256 / SHA-256) device-pairing signatures - the client half of
//! `tokenfuse-cloud`'s `crates/cloud/src/devices.rs` verification (07 §4.2).
//!
//! Wire formats, fixed by `devices.rs` (its module doc is the authority):
//! - public key: base64 of the SEC1/X9.63 **uncompressed** point (65 bytes,
//!   `0x04 || X || Y`) - exactly CryptoKit's `x963Representation`;
//! - signature: base64 of the **raw 64-byte `r||s`** (IEEE P1363) form -
//!   exactly CryptoKit's `ECDSASignature.rawRepresentation`. NOT DER: Apple's
//!   `SecKeyCreateSignature` hands back DER, convert with [`der_to_raw_rs`].
//! - No low-S normalization anywhere: the Secure Enclave emits both S
//!   polarities (measured ~50/50) and the server's `p256 v0.13` verify accepts
//!   both; this module's tests pin that acceptance so a future crate upgrade
//!   that started rejecting high-S would fail loudly here, not in the field.
//!
//! Everything is fail-closed (06 §0.5): no panics on hostile input, and any
//! decode/parse/verify failure is an explicit [`SigningError`]. Callers must
//! treat `Err` and "verification failed" identically (both mean reject) -
//! same rule as F-03 in `docs/PHASE0.md`.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::canonical_request;

/// A signing/verification ceremony failure. Never panic on bad input; never
/// treat any variant as "inconclusive, allow".
#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    #[error("key generation failed: {0}")]
    KeyGeneration(String),
    #[error("public-key export failed: {0}")]
    KeyExport(String),
    #[error("signing failed: {0}")]
    Signing(String),
    #[error("entropy source failed: {0}")]
    Entropy(String),
    #[error("malformed public key: {0}")]
    MalformedPublicKey(String),
    #[error("malformed signature: {0}")]
    MalformedSignature(String),
    #[error("signature verification failed")]
    VerificationFailed,
    #[error("system clock is before the unix epoch")]
    ClockBeforeEpoch,
}

/// What actually holds the private key, surfaced to the journal and UI
/// (06 §3): a hardware key is labelled `secure-enclave`; anything else is
/// honestly labelled `software-signed`, never upgraded by wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assurance {
    /// P-256 private key resident in the Apple Secure Enclave (non-extractable).
    SecureEnclave,
    /// Apple `SecKey` software key (keychain-free, exportable in principle).
    SoftwareSecKey,
    /// Pure-Rust `p256` key in process memory (portable fallback, e.g. Linux).
    SoftwareP256,
}

impl Assurance {
    /// The journal/UI label. Exactly two honest values by design.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Assurance::SecureEnclave => "secure-enclave",
            Assurance::SoftwareSecKey | Assurance::SoftwareP256 => "software-signed",
        }
    }

    /// Human detail for logs beside [`Assurance::label`].
    #[must_use]
    pub fn detail(self) -> &'static str {
        match self {
            Assurance::SecureEnclave => "P-256 in the Apple Secure Enclave (hardware)",
            Assurance::SoftwareSecKey => "Apple SecKey software P-256 (no hardware)",
            Assurance::SoftwareP256 => "pure-Rust p256 software key (no hardware)",
        }
    }

    #[must_use]
    pub fn is_hardware(self) -> bool {
        matches!(self, Assurance::SecureEnclave)
    }
}

/// An ES256 device signer. Implementation: [`SoftwareSigner`] (portable).
/// The macOS SecKey/Secure-Enclave implementations left with the desktop
/// shells; hardware assurance now arrives via WebAuthn assertions instead.
pub trait Es256Signer: Send + Sync {
    /// What holds the key - drives the honest journal/UI label.
    fn assurance(&self) -> Assurance;

    /// SEC1/X9.63 uncompressed public key bytes (65 bytes, `0x04 || X || Y`).
    fn public_key_x963(&self) -> Result<Vec<u8>, SigningError>;

    /// ES256 signature over `message` as raw 64-byte `r||s`.
    fn sign_raw(&self, message: &[u8]) -> Result<[u8; 64], SigningError>;

    /// The public key in the wire format `POST /v1/pair` expects (`pubkey_b64`).
    fn public_key_b64(&self) -> Result<String, SigningError> {
        Ok(B64.encode(self.public_key_x963()?))
    }

    /// The signature in the wire format `X-Fuse-Sig` carries.
    fn sign_b64(&self, message: &[u8]) -> Result<String, SigningError> {
        Ok(B64.encode(self.sign_raw(message)?))
    }
}

/// The three signed-mutation header values the signer mints (the caller adds
/// `X-Fuse-Device` from pairing and the device token as the bearer).
#[derive(Debug, Clone)]
pub struct SignedMutation {
    /// `X-Fuse-TS`: unix seconds as a decimal string (server allows 120s skew).
    pub ts: String,
    /// `X-Fuse-Nonce`: single-use per device; fresh OS randomness here.
    pub nonce: String,
    /// `X-Fuse-Sig`: base64 raw `r||s` over the canonical string.
    pub sig_b64: String,
}

/// Sign a Cloud mutation with a fresh timestamp and nonce.
pub fn sign_mutation(
    signer: &dyn Es256Signer,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<SignedMutation, SigningError> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| SigningError::ClockBeforeEpoch)?
        .as_secs()
        .to_string();
    let nonce = random_hex(16)?;
    sign_mutation_at(signer, method, path, body, &ts, &nonce)
}

/// Sign a Cloud mutation with an explicit timestamp and nonce (deterministic
/// canonical string; used by tests and by callers that manage their own nonces).
pub fn sign_mutation_at(
    signer: &dyn Es256Signer,
    method: &str,
    path: &str,
    body: &[u8],
    ts: &str,
    nonce: &str,
) -> Result<SignedMutation, SigningError> {
    let canonical = canonical_request(method, path, &body_sha256_hex(body), ts, nonce);
    Ok(SignedMutation {
        ts: ts.to_string(),
        nonce: nonce.to_string(),
        sig_b64: signer.sign_b64(canonical.as_bytes())?,
    })
}

/// Lowercase-hex SHA-256 of a request body - the third canonical-string line.
/// An empty body hashes to the well-known `e3b0c442...b855`.
#[must_use]
pub fn body_sha256_hex(body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body);
    hex_lower(&hasher.finalize())
}

/// Lowercase hex of arbitrary bytes (kept dependency-free on purpose).
#[must_use]
pub fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// `n_bytes` of OS randomness as lowercase hex (nonces, run ids).
pub fn random_hex(n_bytes: usize) -> Result<String, SigningError> {
    let mut buf = vec![0u8; n_bytes];
    getrandom::getrandom(&mut buf).map_err(|e| SigningError::Entropy(e.to_string()))?;
    Ok(hex_lower(&buf))
}

/// Convert a DER `ECDSA-Sig-Value` (what `SecKeyCreateSignature` returns) to
/// the raw 64-byte `r||s` form the wire protocol wants. Strict: anything that
/// is not a well-formed P-256 ECDSA DER signature is an error.
pub fn der_to_raw_rs(der: &[u8]) -> Result<[u8; 64], SigningError> {
    let sig =
        Signature::from_der(der).map_err(|e| SigningError::MalformedSignature(e.to_string()))?;
    Ok(sig.to_bytes().into())
}

/// Verify an ES256 raw `r||s` signature over `message` against an X9.63 public
/// key - the exact operation sequence `devices.rs::verify_signature` performs
/// (`VerifyingKey::from_sec1_bytes` + `Signature::from_slice` + `verify`), with
/// the failures made explicit instead of collapsed to `false`.
pub fn verify_es256(
    pubkey_x963: &[u8],
    message: &[u8],
    raw_sig: &[u8],
) -> Result<(), SigningError> {
    let vk = VerifyingKey::from_sec1_bytes(pubkey_x963)
        .map_err(|e| SigningError::MalformedPublicKey(e.to_string()))?;
    let sig = Signature::from_slice(raw_sig)
        .map_err(|e| SigningError::MalformedSignature(e.to_string()))?;
    vk.verify(message, &sig)
        .map_err(|_| SigningError::VerificationFailed)
}

/// [`verify_es256`] over the base64 wire forms (`pubkey_b64` from pairing,
/// `sig_b64` from `X-Fuse-Sig`).
pub fn verify_es256_b64(
    pubkey_b64: &str,
    message: &[u8],
    sig_b64: &str,
) -> Result<(), SigningError> {
    let pk = B64
        .decode(pubkey_b64)
        .map_err(|e| SigningError::MalformedPublicKey(e.to_string()))?;
    let sig = B64
        .decode(sig_b64)
        .map_err(|e| SigningError::MalformedSignature(e.to_string()))?;
    verify_es256(&pk, message, &sig)
}

/// Portable software ES256 signer (pure-Rust `p256`, key in process memory).
/// This is the sanctioned no-hardware fallback (06 §3): it signs the same
/// protocol but its [`Assurance`] is honestly `software-signed`.
pub struct SoftwareSigner {
    key: SigningKey,
}

impl SoftwareSigner {
    /// Generate a fresh software P-256 key from OS randomness.
    pub fn generate() -> Result<Self, SigningError> {
        // `from_slice` rejects out-of-range scalars (probability ~2^-32 per
        // draw for P-256); retry a few times, then fail closed rather than loop.
        for _ in 0..16 {
            let mut buf = [0u8; 32];
            getrandom::getrandom(&mut buf).map_err(|e| SigningError::Entropy(e.to_string()))?;
            if let Ok(key) = SigningKey::from_slice(&buf) {
                return Ok(Self { key });
            }
        }
        Err(SigningError::KeyGeneration(
            "no valid P-256 scalar in 16 draws (entropy source broken?)".into(),
        ))
    }

    /// Deterministic construction from raw scalar bytes (tests, and reloading
    /// a persisted software key -- see [`SoftwareSigner::to_scalar_bytes`]).
    pub fn from_scalar(bytes: &[u8]) -> Result<Self, SigningError> {
        let key = SigningKey::from_slice(bytes)
            .map_err(|e| SigningError::KeyGeneration(e.to_string()))?;
        Ok(Self { key })
    }

    /// The raw 32-byte private scalar, so a software key can be PERSISTED and
    /// reloaded across restarts, for the case where the key IS the identity a
    /// remote party already knows and re-minting it on every boot would throw
    /// that identity away. Round-trips through
    /// [`SoftwareSigner::from_scalar`]. Deliberately only on the SOFTWARE
    /// signer: a Secure Enclave key is non-extractable by construction, and its
    /// [`Assurance`] says so. Callers MUST write the result with owner-only
    /// permissions and never log it.
    #[must_use]
    pub fn to_scalar_bytes(&self) -> [u8; 32] {
        let field = self.key.to_bytes();
        let mut out = [0u8; 32];
        out.copy_from_slice(&field);
        out
    }
}

impl Es256Signer for SoftwareSigner {
    fn assurance(&self) -> Assurance {
        Assurance::SoftwareP256
    }

    fn public_key_x963(&self) -> Result<Vec<u8>, SigningError> {
        Ok(self
            .key
            .verifying_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec())
    }

    fn sign_raw(&self, message: &[u8]) -> Result<[u8; 64], SigningError> {
        let sig: Signature = self
            .key
            .try_sign(message)
            .map_err(|e| SigningError::Signing(e.to_string()))?;
        Ok(sig.to_bytes().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-language pinned vector #1: a real `/v1/runs/{run}/budget` mutation
    /// with a multibyte UTF-8 body. The Swift harness
    /// (`enclave-smoke --vector`) pins the SAME constants, so both languages
    /// are byte-identical iff both match these SHA-256s.
    const V1_METHOD: &str = "POST";
    const V1_PATH: &str = "/v1/runs/spike2-e2e/budget";
    const V1_BODY: &str = "{\"budget_usd\":12.5,\"note\":\"обмеження діє\"}";
    const V1_TS: &str = "1758000000";
    const V1_NONCE: &str = "genaryx-spike2-nonce";
    const V1_BODY_SHA256: &str = "94443f9c3dbe6095049a04c7c23436f246d12566f1108d6c1c5df1bf373405b9";
    const V1_CANONICAL_SHA256: &str =
        "66c4919da908f16b8ea5a7cdc2a51c7a271653d4a6a0cb9f634ff64de9ef9f2a";

    /// Cross-language pinned vector #2: an empty-body kill.
    const V2_PATH: &str = "/v1/runs/spike2-e2e/kill";
    const V2_CANONICAL_SHA256: &str =
        "4bbe4ceedc64d8bf1191a48cd8a98b9b8482ce5ecb948a1df65d6dd29ed27aa8";

    fn v1_canonical() -> String {
        canonical_request(
            V1_METHOD,
            V1_PATH,
            &body_sha256_hex(V1_BODY.as_bytes()),
            V1_TS,
            V1_NONCE,
        )
    }

    #[test]
    fn cross_language_vector_1_budget_utf8_body() {
        assert_eq!(V1_BODY.len(), 54, "body must be the exact 54 UTF-8 bytes");
        assert_eq!(body_sha256_hex(V1_BODY.as_bytes()), V1_BODY_SHA256);
        let canonical = v1_canonical();
        assert_eq!(
            hex_lower(&Sha256::digest(canonical.as_bytes())),
            V1_CANONICAL_SHA256,
            "canonical bytes drifted from the pinned cross-language vector"
        );
    }

    #[test]
    fn cross_language_vector_2_empty_body_kill() {
        let canonical =
            canonical_request(V1_METHOD, V2_PATH, &body_sha256_hex(b""), V1_TS, V1_NONCE);
        assert!(
            canonical.contains("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        assert_eq!(
            hex_lower(&Sha256::digest(canonical.as_bytes())),
            V2_CANONICAL_SHA256
        );
    }

    #[test]
    fn der_round_trips_to_raw_rs() {
        let signer = SoftwareSigner::from_scalar(&[0x11u8; 32]).unwrap();
        let sig: Signature = signer.key.try_sign(b"payload").unwrap();
        let raw = der_to_raw_rs(sig.to_der().as_bytes()).unwrap();
        assert_eq!(raw, <[u8; 64]>::from(sig.to_bytes()));
    }

    #[test]
    fn malformed_der_is_an_error_not_a_panic() {
        assert!(matches!(
            der_to_raw_rs(b"not-der"),
            Err(SigningError::MalformedSignature(_))
        ));
        assert!(matches!(
            der_to_raw_rs(&[]),
            Err(SigningError::MalformedSignature(_))
        ));
    }

    #[test]
    fn software_signer_round_trips_through_devices_rs_verify_path() {
        let signer = SoftwareSigner::generate().unwrap();
        let m = sign_mutation_at(
            &signer,
            V1_METHOD,
            V1_PATH,
            V1_BODY.as_bytes(),
            V1_TS,
            V1_NONCE,
        )
        .unwrap();
        let canonical = v1_canonical();
        verify_es256_b64(
            &signer.public_key_b64().unwrap(),
            canonical.as_bytes(),
            &m.sig_b64,
        )
        .expect("genuine signature must verify");
        assert_eq!(signer.assurance().label(), "software-signed");
    }

    #[test]
    fn tampered_message_signature_and_wrong_key_all_reject() {
        let signer = SoftwareSigner::from_scalar(&[0x11u8; 32]).unwrap();
        let pk = signer.public_key_b64().unwrap();
        let canonical = v1_canonical();
        let sig_b64 = signer.sign_b64(canonical.as_bytes()).unwrap();

        // Tampered message (a different path) must reject.
        let other = canonical_request(
            V1_METHOD,
            V2_PATH,
            &body_sha256_hex(V1_BODY.as_bytes()),
            V1_TS,
            V1_NONCE,
        );
        assert!(verify_es256_b64(&pk, other.as_bytes(), &sig_b64).is_err());

        // Tampered signature bytes must reject (either decode error or
        // verification failure; both mean reject, per F-03).
        let mut raw = signer.sign_raw(canonical.as_bytes()).unwrap();
        raw[10] ^= 0x01;
        assert!(
            verify_es256(
                &signer.public_key_x963().unwrap(),
                canonical.as_bytes(),
                &raw
            )
            .is_err()
        );

        // A different key must reject.
        let stranger = SoftwareSigner::from_scalar(&[0x22u8; 32]).unwrap();
        assert!(
            verify_es256_b64(
                &stranger.public_key_b64().unwrap(),
                canonical.as_bytes(),
                &sig_b64
            )
            .is_err()
        );

        // Garbage wire inputs are errors, never panics.
        assert!(verify_es256_b64("not-base64!!", b"x", "also!!").is_err());
        assert!(verify_es256(&[], b"x", &[]).is_err());
    }

    /// Pin that the server's verify path accepts BOTH S polarities. CryptoKit
    /// and the Secure Enclave emit high-S about half the time (measured 15/30
    /// on this box) and never normalize; if a p256 upgrade ever started
    /// enforcing low-S, half of all hardware signatures would fail - this
    /// test makes that break loudly at build time instead.
    #[test]
    fn both_s_polarities_verify() {
        let signer = SoftwareSigner::from_scalar(&[0x11u8; 32]).unwrap();
        let msg = v1_canonical();
        let sig: Signature = signer.key.try_sign(msg.as_bytes()).unwrap();
        let pk = signer.public_key_x963().unwrap();
        verify_es256(&pk, msg.as_bytes(), &sig.to_bytes()).expect("original polarity");

        let flipped_s = p256::NonZeroScalar::new(-*sig.s()).expect("nonzero");
        let flipped = Signature::from_scalars(*sig.r(), *flipped_s).expect("valid scalars");
        verify_es256(&pk, msg.as_bytes(), &flipped.to_bytes())
            .expect("opposite polarity must also verify (no low-S enforcement)");
    }

    #[test]
    fn sign_mutation_mints_fresh_ts_and_nonce() {
        let signer = SoftwareSigner::generate().unwrap();
        let a = sign_mutation(&signer, "POST", V2_PATH, b"").unwrap();
        let b = sign_mutation(&signer, "POST", V2_PATH, b"").unwrap();
        assert_ne!(a.nonce, b.nonce, "nonces must be single-use");
        assert_eq!(a.nonce.len(), 32, "16 random bytes as hex");
        let ts: i64 = a.ts.parse().expect("ts is decimal unix seconds");
        assert!(ts > 1_700_000_000, "ts is a current unix timestamp");
        // And the minted headers verify against the reconstructed canonical.
        let canonical = canonical_request("POST", V2_PATH, &body_sha256_hex(b""), &a.ts, &a.nonce);
        verify_es256_b64(
            &signer.public_key_b64().unwrap(),
            canonical.as_bytes(),
            &a.sig_b64,
        )
        .unwrap();
    }

    #[test]
    fn scalar_bytes_round_trip_preserves_the_key() {
        let a = SoftwareSigner::generate().unwrap();
        let scalar = a.to_scalar_bytes();
        let b = SoftwareSigner::from_scalar(&scalar).expect("reload from persisted scalar");
        // Same key => same public point => a signature from one verifies under
        // the other's public key.
        let msg = b"acme account key persistence";
        let sig = a.sign_raw(msg).unwrap();
        verify_es256(&b.public_key_x963().unwrap(), msg, &sig)
            .expect("reloaded key is the same key");
        assert_eq!(a.public_key_x963().unwrap(), b.public_key_x963().unwrap());
    }

    #[test]
    fn assurance_labels_are_honest() {
        assert_eq!(Assurance::SecureEnclave.label(), "secure-enclave");
        assert_eq!(Assurance::SoftwareSecKey.label(), "software-signed");
        assert_eq!(Assurance::SoftwareP256.label(), "software-signed");
        assert!(Assurance::SecureEnclave.is_hardware());
        assert!(!Assurance::SoftwareSecKey.is_hardware());
    }
}
