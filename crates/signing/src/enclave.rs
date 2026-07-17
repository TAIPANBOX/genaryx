//! macOS `SecKey`-backed ES256 signers for the Tauri shell (06 §7 spike #2):
//! a P-256 key generated in the **Apple Secure Enclave** via
//! `kSecAttrTokenIDSecureEnclave`, or a keychain-free software `SecKey` as the
//! same-API fallback. The SwiftUI shell reaches the enclave natively through
//! CryptoKit; this module is the Rust route to the same hardware.
//!
//! Two facts this spike established empirically on real hardware (M1 Pro,
//! macOS 26.5, see docs/PHASE0.md spike row 2):
//! - An **ephemeral** enclave key (`kSecAttrIsPermanent = false`, no keychain
//!   item, which is what no `set_location` means) generates and signs from a
//!   plain unsigned/ad-hoc CLI process - no entitlements, no signed app bundle,
//!   no GUI session needed. Persisting enclave keys to the data-protection
//!   keychain is where entitlement requirements appear; Phase 0 does not
//!   persist.
//! - `SecKeyCreateSignature(..., ecdsaSignatureMessageX962SHA256, ...)`
//!   returns **DER**; the wire wants raw 64-byte `r||s`, hence
//!   [`der_to_raw_rs`] on every signature.
//!
//! Fail-closed: generation, export, and signing failures are explicit
//! [`SigningError`]s; requesting the enclave never silently downgrades - the
//! only fallback path is [`SecKeySigner::generate_preferring_enclave`], which
//! reports the downgrade reason and labels the result `software-signed`.

use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use security_framework::key::{Algorithm, GenerateKeyOptions, KeyType, SecKey, Token};

use crate::es256::{Assurance, Es256Signer, SigningError, der_to_raw_rs};

/// The `kSecAttrTokenID` attribute key and its Secure-Enclave value, as fixed
/// by Apple's Security framework headers (`SecItem.h`). Read back after
/// generation to *prove* enclave residency instead of assuming it.
const ATTR_TOKEN_ID: &str = "tkid";
const TOKEN_ID_SECURE_ENCLAVE: &str = "com.apple.setoken";

/// Which `SecKey` backing to request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backing {
    /// Non-extractable P-256 in the Secure Enclave.
    SecureEnclave,
    /// Software `SecKey` (no hardware; honest `software-signed` label).
    SoftwareSecKey,
}

/// An ES256 signer over an Apple `SecKey` (enclave-resident or software).
/// The key is ephemeral: it lives exactly as long as this value and is never
/// written to any keychain.
pub struct SecKeySigner {
    key: SecKey,
    assurance: Assurance,
}

impl SecKeySigner {
    /// Generate a fresh ephemeral P-256 `SecKey` with the requested backing.
    ///
    /// For [`Backing::SecureEnclave`] this fails closed twice over: the OS
    /// call itself errors where the enclave is unavailable (VMs, entitlement
    /// contexts that require them), and on success the key's `tkid` attribute
    /// is read back and must be `com.apple.setoken` - a hardware claim is
    /// never taken on faith.
    pub fn generate(backing: Backing) -> Result<Self, SigningError> {
        let mut opts = GenerateKeyOptions::default();
        opts.set_key_type(KeyType::ec_sec_prime_random())
            .set_size_in_bits(256);
        // No `set_location`: kSecAttrIsPermanent stays false, so the key is an
        // in-process handle only - nothing is written to any keychain.
        let assurance = match backing {
            Backing::SecureEnclave => {
                opts.set_token(Token::SecureEnclave);
                Assurance::SecureEnclave
            }
            Backing::SoftwareSecKey => {
                opts.set_token(Token::Software);
                Assurance::SoftwareSecKey
            }
        };
        let key = SecKey::new(&opts).map_err(|e| SigningError::KeyGeneration(e.to_string()))?;
        if backing == Backing::SecureEnclave {
            match token_id(&key) {
                Some(t) if t == TOKEN_ID_SECURE_ENCLAVE => {}
                other => {
                    return Err(SigningError::KeyGeneration(format!(
                        "requested a Secure Enclave key but its token attribute is {other:?}"
                    )));
                }
            }
        }
        Ok(Self { key, assurance })
    }

    /// Generate an enclave key, falling back to a software `SecKey` when the
    /// enclave is unavailable. Returns the signer plus, on fallback, the
    /// enclave's refusal message - callers must journal it next to the
    /// `software-signed` label (06 §3), not swallow it.
    pub fn generate_preferring_enclave() -> Result<(Self, Option<String>), SigningError> {
        match Self::generate(Backing::SecureEnclave) {
            Ok(signer) => Ok((signer, None)),
            Err(enclave_err) => {
                let signer = Self::generate(Backing::SoftwareSecKey)?;
                Ok((signer, Some(enclave_err.to_string())))
            }
        }
    }
}

impl Es256Signer for SecKeySigner {
    fn assurance(&self) -> Assurance {
        self.assurance
    }

    fn public_key_x963(&self) -> Result<Vec<u8>, SigningError> {
        let public = self
            .key
            .public_key()
            .ok_or_else(|| SigningError::KeyExport("SecKeyCopyPublicKey returned NULL".into()))?;
        let data = public.external_representation().ok_or_else(|| {
            SigningError::KeyExport("SecKeyCopyExternalRepresentation returned NULL".into())
        })?;
        // For EC keys this is the X9.63 uncompressed point, already the wire form.
        Ok(data.to_vec())
    }

    fn sign_raw(&self, message: &[u8]) -> Result<[u8; 64], SigningError> {
        let der = self
            .key
            .create_signature(Algorithm::ECDSASignatureMessageX962SHA256, message)
            .map_err(|e| SigningError::Signing(e.to_string()))?;
        der_to_raw_rs(&der)
    }
}

/// The key's `kSecAttrTokenID` attribute, if any (`com.apple.setoken` for an
/// enclave-resident key; absent for software keys).
fn token_id(key: &SecKey) -> Option<String> {
    let attrs = key.attributes();
    let attr_key = CFString::from_static_string(ATTR_TOKEN_ID);
    let value = attrs.find(attr_key.as_CFTypeRef().cast())?;
    // SAFETY: `kSecAttrTokenID` is documented to hold a CFString; the wrap uses
    // the get rule on a value the `attrs` dictionary keeps alive for this scope.
    let s = unsafe { CFString::wrap_under_get_rule((*value).cast()) };
    Some(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::es256::verify_es256;

    const MSG: &[u8] =
        b"POST\n/v1/runs/r1/kill\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n100\nn1";

    #[test]
    fn software_seckey_round_trips_and_is_labelled_honestly() {
        let signer = SecKeySigner::generate(Backing::SoftwareSecKey).expect("software SecKey");
        assert_eq!(signer.assurance().label(), "software-signed");
        let pk = signer.public_key_x963().unwrap();
        assert_eq!(pk.len(), 65, "X9.63 uncompressed point");
        assert_eq!(pk[0], 0x04);
        let sig = signer.sign_raw(MSG).unwrap();
        verify_es256(&pk, MSG, &sig).expect("SecKey signature must verify via devices.rs path");
        // And the signature must bind the message.
        assert!(verify_es256(&pk, b"tampered", &sig).is_err());
    }

    /// Environment-dependent by nature: on real Apple-silicon hardware the
    /// enclave path must fully work; where there is no enclave (VMs, CI
    /// runners) the *documented failure contract* is what must hold. Either
    /// way the test passes only when behavior is the one we promise.
    #[test]
    fn secure_enclave_works_or_fails_closed() {
        match SecKeySigner::generate(Backing::SecureEnclave) {
            Ok(signer) => {
                assert_eq!(signer.assurance(), Assurance::SecureEnclave);
                assert!(signer.assurance().is_hardware());
                let pk = signer.public_key_x963().unwrap();
                let sig = signer.sign_raw(MSG).unwrap();
                verify_es256(&pk, MSG, &sig)
                    .expect("enclave signature must verify via devices.rs path");
                assert!(verify_es256(&pk, b"tampered", &sig).is_err());
                println!("secure enclave: AVAILABLE (hardware-backed signature verified)");
            }
            Err(e @ SigningError::KeyGeneration(_)) => {
                println!("secure enclave: unavailable here, failed closed with: {e}");
            }
            Err(other) => panic!("enclave generation must fail as KeyGeneration, got: {other}"),
        }
    }

    #[test]
    fn preferring_enclave_reports_fallback_honestly() {
        let (signer, fallback_reason) =
            SecKeySigner::generate_preferring_enclave().expect("some SecKey signer");
        let pk = signer.public_key_x963().unwrap();
        let sig = signer.sign_raw(MSG).unwrap();
        verify_es256(&pk, MSG, &sig).unwrap();
        match fallback_reason {
            None => assert!(
                signer.assurance().is_hardware(),
                "no fallback reported, so the key must be enclave-resident"
            ),
            Some(reason) => {
                assert!(
                    !signer.assurance().is_hardware(),
                    "fallback must carry the software-signed label"
                );
                assert_eq!(signer.assurance().label(), "software-signed");
                println!("enclave fallback reason: {reason}");
            }
        }
    }
}
