//! The approver statement: the console's signed proof that a person, not a
//! bearer key, decided an approval.
//!
//! The shape is the one agreed with wardryx (`approval-statement-proposal`,
//! 2026-10-19): a compact JWS, ES256, signed by the console device key only
//! after the person's passkey assertion has verified. The header pins
//! `alg` and `typ` and names the key by its RFC 7638 thumbprint, and carries
//! no key of its own. The claim names below are the names wardryx checks, so
//! changing one is a change to the proposal first.

use crate::es256::{Es256Signer, SigningError, verify_es256};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// The JWS `typ` a statement pins.
pub const TYP: &str = "approver-statement+jwt";

/// The longest a statement may live, from issue time to expiry.
pub const MAX_LIFETIME_SECS: i64 = 300;

/// How far ahead of the verifier's clock an issue time may sit.
pub const MAX_FUTURE_SKEW_SECS: i64 = 30;

/// The claims a statement carries, in the names wardryx checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claims {
    /// The approver: the console's stable subject for the person.
    pub sub: String,
    /// The WebAuthn credential id the passkey assertion verified against.
    pub cred: String,
    /// The approval id, which must equal the `{id}` of the decide path.
    pub aid: String,
    /// `grant` or `deny`, which must equal the decision being made.
    pub dec: String,
    /// Issue time, unix seconds.
    pub iat: i64,
    /// Expiry, unix seconds.
    pub exp: i64,
    /// A unique id for this statement.
    pub jti: String,
}

/// Why a statement could not be made or did not verify.
#[derive(Debug, thiserror::Error)]
pub enum StatementError {
    /// The claims are outside what wardryx accepts; no statement is made.
    #[error("the approver statement's claims are refused: {0}")]
    Claims(&'static str),
    /// The statement is not a well-formed statement of this shape.
    #[error("the approver statement is malformed: {0}")]
    Malformed(&'static str),
    /// The signature does not verify under the console key.
    #[error("the approver statement does not verify under the console key")]
    Unverified,
    /// The console key could not be used.
    #[error(transparent)]
    Key(#[from] SigningError),
}

/// RFC 7638 thumbprint of a P-256 public key given as SEC1 uncompressed bytes.
///
/// Red-first scaffold: returns an error until the fix lands.
pub fn thumbprint(_x963: &[u8]) -> Result<String, StatementError> {
    Err(StatementError::Malformed(
        "red-first scaffold: not implemented",
    ))
}

/// The public half of the console key as a JWKS, the file wardryx reads as its
/// approver key set.
///
/// Red-first scaffold: returns an error until the fix lands.
pub fn public_jwks(_x963: &[u8]) -> Result<serde_json::Value, StatementError> {
    Err(StatementError::Malformed(
        "red-first scaffold: not implemented",
    ))
}

/// Make a statement with the console key. The claims are checked first, so a
/// statement wardryx would refuse is never made.
///
/// Red-first scaffold: returns an error until the fix lands.
pub fn sign(_signer: &dyn Es256Signer, _claims: &Claims) -> Result<String, StatementError> {
    Err(StatementError::Malformed(
        "red-first scaffold: not implemented",
    ))
}

/// Verify a statement against the console public key at time `now`, and
/// return its claims.
///
/// Red-first scaffold: returns an error until the fix lands.
pub fn verify(_x963: &[u8], _statement: &str, _now: i64) -> Result<Claims, StatementError> {
    Err(StatementError::Malformed(
        "red-first scaffold: not implemented",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::es256::SoftwareSigner;

    fn key() -> SoftwareSigner {
        SoftwareSigner::generate().expect("a test key")
    }

    fn claims() -> Claims {
        Claims {
            sub: "person://acme/alice".into(),
            cred: "Y3JlZC0x".into(),
            aid: "appr-7".into(),
            dec: "grant".into(),
            iat: 1_000,
            exp: 1_120,
            jti: "stmt-1".into(),
        }
    }

    fn parts(jws: &str) -> Vec<String> {
        jws.split('.').map(str::to_owned).collect()
    }

    #[test]
    fn a_statement_verifies_under_the_console_key_and_carries_its_claims() {
        let k = key();
        let x = k.public_key_x963().unwrap();
        let jws = sign(&k, &claims()).expect("a statement with valid claims is made");
        let got = verify(&x, &jws, 1_010).expect("the statement verifies under its own key");
        assert_eq!(got, claims());
    }

    #[test]
    fn the_header_pins_es256_the_statement_type_and_the_console_thumbprint() {
        let k = key();
        let x = k.public_key_x963().unwrap();
        let jws = sign(&k, &claims()).unwrap();
        let p = parts(&jws);
        assert_eq!(p.len(), 3, "a compact JWS has three parts");
        let header: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(&p[0]).unwrap()).unwrap();
        assert_eq!(header["alg"], "ES256");
        assert_eq!(header["typ"], "approver-statement+jwt");
        assert_eq!(header["kid"], thumbprint(&x).unwrap());
        assert!(
            header.get("jwk").is_none(),
            "the header carries no key of its own"
        );
        assert!(
            header.get("x5c").is_none(),
            "the header carries no certificate"
        );
    }

    #[test]
    fn the_thumbprint_is_the_sha256_of_the_canonical_ec_members() {
        // RFC 7638 section 3.2: the required members of an EC key, sorted,
        // with no whitespace, hashed with SHA-256 and base64url encoded.
        let x = key().public_key_x963().unwrap();
        let xc = URL_SAFE_NO_PAD.encode(&x[1..33]);
        let yc = URL_SAFE_NO_PAD.encode(&x[33..65]);
        let canonical = format!(r#"{{"crv":"P-256","kty":"EC","x":"{xc}","y":"{yc}"}}"#);
        let want = URL_SAFE_NO_PAD.encode(Sha256::digest(canonical.as_bytes()));
        assert_eq!(thumbprint(&x).unwrap(), want);
        assert_ne!(
            thumbprint(&x).unwrap(),
            thumbprint(&key().public_key_x963().unwrap()).unwrap()
        );
    }

    #[test]
    fn a_tampered_payload_does_not_verify() {
        let k = key();
        let x = k.public_key_x963().unwrap();
        let jws = sign(&k, &claims()).unwrap();
        let p = parts(&jws);
        let mut payload: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(&p[1]).unwrap()).unwrap();
        payload["dec"] = serde_json::json!("deny");
        let forged = format!(
            "{}.{}.{}",
            p[0],
            URL_SAFE_NO_PAD.encode(payload.to_string()),
            p[2]
        );
        assert!(matches!(
            verify(&x, &forged, 1_010),
            Err(StatementError::Unverified)
        ));
    }

    #[test]
    fn a_statement_under_another_console_key_does_not_verify() {
        let k = key();
        let jws = sign(&k, &claims()).unwrap();
        let other = key().public_key_x963().unwrap();
        assert!(verify(&other, &jws, 1_010).is_err());
    }

    #[test]
    fn claims_outside_what_wardryx_accepts_are_refused_before_anything_is_signed() {
        let k = key();
        let mut no_subject = claims();
        no_subject.sub = String::new();
        let mut control_in_subject = claims();
        control_in_subject.sub = "person://acme/a\u{7}lice".into();
        let mut bidi_in_subject = claims();
        bidi_in_subject.sub = "person://acme/\u{202e}alice".into();
        let mut cred_not_base64url = claims();
        cred_not_base64url.cred = "cred+1/2".into();
        let mut other_decision = claims();
        other_decision.dec = "maybe".into();
        let mut too_long = claims();
        too_long.exp = too_long.iat + MAX_LIFETIME_SECS + 1;
        let mut no_jti = claims();
        no_jti.jti = String::new();
        for (name, bad) in [
            ("no subject", no_subject),
            ("a control character in the subject", control_in_subject),
            ("a bidi override in the subject", bidi_in_subject),
            ("a credential id that is not base64url", cred_not_base64url),
            ("a decision other than grant or deny", other_decision),
            ("a lifetime longer than five minutes", too_long),
            ("no statement id", no_jti),
        ] {
            assert!(
                matches!(sign(&k, &bad), Err(StatementError::Claims(_))),
                "{name} must be refused at signing"
            );
        }
    }

    #[test]
    fn a_statement_outside_its_window_does_not_verify() {
        let k = key();
        let x = k.public_key_x963().unwrap();
        let jws = sign(&k, &claims()).unwrap();
        // Expired: the clock has reached exp (1120).
        assert!(matches!(
            verify(&x, &jws, 1_120),
            Err(StatementError::Claims(_))
        ));
        // Issued in the future: iat (1000) is more than 30 s ahead of 900.
        assert!(matches!(
            verify(&x, &jws, 900),
            Err(StatementError::Claims(_))
        ));
    }

    #[test]
    fn the_public_jwks_publishes_the_key_the_statement_names() {
        let k = key();
        let x = k.public_key_x963().unwrap();
        let jwks = public_jwks(&x).unwrap();
        let entry = &jwks["keys"][0];
        assert_eq!(entry["kty"], "EC");
        assert_eq!(entry["crv"], "P-256");
        assert_eq!(entry["alg"], "ES256");
        assert_eq!(entry["x"], URL_SAFE_NO_PAD.encode(&x[1..33]));
        assert_eq!(entry["y"], URL_SAFE_NO_PAD.encode(&x[33..65]));
        assert_eq!(entry["kid"], thumbprint(&x).unwrap());
        let jws = sign(&k, &claims()).unwrap();
        assert_eq!(verify(&x, &jws, 1_010).unwrap(), claims());
    }
}
