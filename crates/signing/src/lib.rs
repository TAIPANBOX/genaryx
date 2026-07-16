//! genaryx-signing: signing and verification ceremonies.
//!
//! Phase-0 placeholder. The core prepares the canonical string; the shell's Signer
//! adapter signs (SwiftUI natively via CryptoKit/Secure Enclave; Tauri via
//! `security-framework`; YubiKey PIV cross-platform). Server verification already
//! exists in `tokenfuse-cloud` device-pairing: ES256 over
//! `METHOD\nPATH\nsha256(body)hex\nTS\nNONCE` (06 §2, 07 §4.2).
//!
//! Spike #2 and #4 (06 §7) land here: Secure Enclave both paths, and ML-DSA verify.

pub mod mldsa;

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
