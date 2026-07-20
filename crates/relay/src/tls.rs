//! Self-signed TLS identity for the public phone-facing listener
//! (itrat-console/13 D12.1: "SPKI pin delivered via QR"; D12.2 step 3: "The
//! SPKI pin is the relay's self-signed TLS key fingerprint: no public CA, no
//! domain requirement, works on IP-only enterprise networks. The pin riding
//! a QR that the user physically scans off the desktop screen is the trust
//! root of the whole channel.").
//!
//! A P-256 key pair + self-signed cert are generated once and persisted
//! (`{tls_cert_dir}/cert.pem` + `key.pem`); later runs reload the same
//! identity so restarts don't change the pin under already-paired phones
//! (D12.3's key-inventory note: "Regenerate = re-pair (pin changes)").

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use sha2::{Digest, Sha256};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TlsError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("cert generation failed: {0}")]
    Generate(String),
    #[error("failed to load private key: {0}")]
    LoadKey(String),
    #[error("failed to build rustls server config: {0}")]
    RustlsConfig(String),
}

/// The relay's public-listener TLS identity: the cert+key PEM axum-server
/// serves, plus the SPKI-SHA256 pin (base64) the QR carries.
pub struct RelayIdentity {
    cert_pem: Vec<u8>,
    key_pem: Vec<u8>,
    spki_sha256_b64: String,
}

impl RelayIdentity {
    /// Load `{dir}/cert.pem` + `{dir}/key.pem` if both already exist;
    /// otherwise generate a fresh self-signed P-256 identity and persist it
    /// there (the private key written with owner-only permissions on unix).
    pub fn load_or_generate(dir: &Path) -> Result<Self, TlsError> {
        let cert_path = dir.join("cert.pem");
        let key_path = dir.join("key.pem");
        if cert_path.exists() && key_path.exists() {
            return Self::load(&cert_path, &key_path);
        }
        std::fs::create_dir_all(dir)?;
        let identity = Self::generate()?;
        write_private_pem(&key_path, &identity.key_pem)?;
        std::fs::write(&cert_path, &identity.cert_pem)?;
        Ok(identity)
    }

    /// Load an already-persisted `{dir}/cert.pem` + `{dir}/key.pem`, erroring
    /// if either is absent. PUBLIC-CA mode uses this (never
    /// `load_or_generate`): a relay that is meant to serve a CA-signed cert
    /// must NEVER silently fall back to a self-signed one, so "not there yet"
    /// is an obtain trigger the caller handles, not a generate-in-place.
    pub fn load_existing(dir: &Path) -> Result<Self, TlsError> {
        let cert_path = dir.join("cert.pem");
        let key_path = dir.join("key.pem");
        if !cert_path.exists() || !key_path.exists() {
            return Err(TlsError::LoadKey(format!(
                "no persisted cert/key in {}",
                dir.display()
            )));
        }
        Self::load(&cert_path, &key_path)
    }

    /// Persist an externally obtained certificate + key (from ACME, `acme.rs`)
    /// into `dir` as `cert.pem`/`key.pem` (key written owner-only on unix),
    /// then load the resulting identity. The SPKI pin is re-derived from the
    /// key exactly as [`RelayIdentity::load`] does, so a PUBLIC-CA relay still
    /// has a valid pin available even though the phone trusts the hostname's
    /// public chain instead of pinning.
    pub fn install(dir: &Path, cert_pem: &[u8], key_pem: &[u8]) -> Result<Self, TlsError> {
        std::fs::create_dir_all(dir)?;
        let cert_path = dir.join("cert.pem");
        let key_path = dir.join("key.pem");
        write_private_pem(&key_path, key_pem)?;
        std::fs::write(&cert_path, cert_pem)?;
        Self::load(&cert_path, &key_path)
    }

    fn load(cert_path: &Path, key_path: &Path) -> Result<Self, TlsError> {
        let cert_pem = std::fs::read(cert_path)?;
        let key_pem = std::fs::read(key_path)?;
        let key_pem_str = String::from_utf8(key_pem.clone())
            .map_err(|e| TlsError::LoadKey(format!("key file is not valid UTF-8 PEM: {e}")))?;
        // Re-derive the pin from the persisted private key rather than
        // parsing the cert's X.509 structure: `rcgen::KeyPair` already knows
        // how to hand back the SPKI DER it originally signed, so this needs
        // no extra X.509-parsing dependency.
        let key_pair = rcgen::KeyPair::from_pem(&key_pem_str)
            .map_err(|e| TlsError::LoadKey(format!("failed to parse persisted key: {e}")))?;
        let spki_sha256_b64 = spki_pin(&key_pair);
        Ok(Self {
            cert_pem,
            key_pem,
            spki_sha256_b64,
        })
    }

    fn generate() -> Result<Self, TlsError> {
        let key_pair = rcgen::KeyPair::generate().map_err(|e| TlsError::Generate(e.to_string()))?;
        let spki_sha256_b64 = spki_pin(&key_pair);

        let params = rcgen::CertificateParams::new(vec!["genaryx-relay".to_string()])
            .map_err(|e| TlsError::Generate(e.to_string()))?;
        let cert = params
            .self_signed(&key_pair)
            .map_err(|e| TlsError::Generate(e.to_string()))?;

        Ok(Self {
            cert_pem: cert.pem().into_bytes(),
            key_pem: key_pair.serialize_pem().into_bytes(),
            spki_sha256_b64,
        })
    }

    /// The SPKI-SHA256 pin, base64-encoded -- the phone-channel trust root
    /// the QR carries (D12.2 step 3).
    pub fn spki_sha256_b64(&self) -> &str {
        &self.spki_sha256_b64
    }

    /// Build the rustls server config axum-server's public listener serves.
    pub async fn rustls_config(&self) -> Result<axum_server::tls_rustls::RustlsConfig, TlsError> {
        axum_server::tls_rustls::RustlsConfig::from_pem(self.cert_pem.clone(), self.key_pem.clone())
            .await
            .map_err(|e| TlsError::RustlsConfig(e.to_string()))
    }
}

/// SHA-256 of the key pair's SubjectPublicKeyInfo DER, base64-encoded --
/// exactly the "SPKI-SHA256 pin (base64)" the crate's module docs and
/// docs/PHASE5.md call for.
fn spki_pin(key_pair: &rcgen::KeyPair) -> String {
    B64.encode(Sha256::digest(key_pair.public_key_der()))
}

#[cfg(unix)]
fn write_private_pem(path: &Path, bytes: &[u8]) -> Result<(), TlsError> {
    use std::fs::OpenOptions;
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private_pem(path: &Path, bytes: &[u8]) -> Result<(), TlsError> {
    std::fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_produces_a_stable_pin_and_pem_material() {
        let identity = RelayIdentity::generate().expect("self-signed generation");
        assert!(identity.cert_pem.starts_with(b"-----BEGIN CERTIFICATE"));
        assert!(!identity.spki_sha256_b64().is_empty());
        // Base64 of a 32-byte SHA-256 digest is 44 chars with one '=' pad.
        assert_eq!(identity.spki_sha256_b64().len(), 44);
    }

    #[test]
    fn load_or_generate_persists_and_reload_yields_the_same_pin() {
        // A random suffix (not a counter) keeps parallel test runs from
        // colliding on the same directory; `genaryx_signing::es256` is
        // already the workspace's one OS-randomness-as-hex helper (used
        // elsewhere for mutation nonces), reused here rather than hand-rolled.
        let suffix = genaryx_signing::es256::random_hex(8).expect("os rng");
        let dir = std::env::temp_dir().join(format!("genaryx-relay-tls-test-{suffix}"));
        let first = RelayIdentity::load_or_generate(&dir).expect("first generation");
        let pin1 = first.spki_sha256_b64().to_string();

        let second = RelayIdentity::load_or_generate(&dir).expect("reload from disk");
        let pin2 = second.spki_sha256_b64().to_string();

        assert_eq!(
            pin1, pin2,
            "reloading the persisted identity keeps the pin stable"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
