//! genaryx-signing: signing and verification ceremonies.
//!
//! The core prepares the canonical string; the shell's Signer adapter signs
//! (SwiftUI natively via CryptoKit/Secure Enclave; Tauri via
//! `security-framework`; YubiKey PIV cross-platform). Server verification
//! already exists in `tokenfuse-cloud` device-pairing: ES256 over
//! `METHOD\nPATH\nsha256(body)hex\nTS\nNONCE` (06 §2, 07 §4.2).
//!
//! Spike #4 (06 §7) landed in [`mldsa`] (ML-DSA verify). Spike #2 landed in
//! [`es256`] (portable ES256 signing/verify, honest [`es256::Assurance`]
//! labels). The macOS Secure-Enclave/SecKey signers that once lived beside it
//! left with the native desktop shells (web-only pivot); the enum keeps their
//! assurance vocabulary because the labels describe KEY RESIDENCY, not a
//! shell. Live-cloud driver: `examples/verify_es256_blob.rs`.

pub mod es256;
pub mod mldsa;

pub use es256::{
    Assurance, Es256Signer, SignedMutation, SigningError, SoftwareSigner, body_sha256_hex,
    der_to_raw_rs, sign_mutation, sign_mutation_at, verify_es256, verify_es256_b64,
};

/// Build the canonical string a device signs for a Cloud mutation (07 §4.2).
/// `body_sha256_hex` is the lowercase hex SHA-256 of the request body ("" -> empty body).
pub fn canonical_request(
    method: &str,
    path: &str,
    body_sha256_hex: &str,
    ts: &str,
    nonce: &str,
) -> String {
    format!("{method}\n{path}\n{body_sha256_hex}\n{ts}\n{nonce}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_shape_matches_devices_rs() {
        let s = canonical_request(
            "POST",
            "/v1/runs/run-1/kill",
            "abc123",
            "1700000000",
            "n0nce",
        );
        assert_eq!(s, "POST\n/v1/runs/run-1/kill\nabc123\n1700000000\nn0nce");
        assert_eq!(s.lines().count(), 5);
    }
}
