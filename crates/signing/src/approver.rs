//! The approver statement: the console's signed proof that a person, not a
//! bearer key, decided an approval.
//!
//! The shape is the one agreed with wardryx (`approval-statement-proposal`,
//! 2026-10-19): a compact JWS, ES256, signed by the console device key only
//! after the person's passkey assertion has verified. The header pins
//! `alg` and `typ` and names the key by its RFC 7638 thumbprint, and carries
//! no key of its own. The claim names below are the names wardryx checks, so
//! changing one is a change to the proposal first.

use crate::es256::{Es256Signer, SigningError, SoftwareSigner, verify_es256};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::ecdsa::VerifyingKey;
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
    #[error("{0}")]
    Malformed(&'static str),
    /// The signature does not verify under the console key.
    #[error("the approver statement does not verify under the console key")]
    Unverified,
    /// The console key could not be used.
    #[error(transparent)]
    Key(#[from] SigningError),
}

/// Check that a SEC1 uncompressed P-256 point is one, and return its affine
/// coordinates as base64url text.
fn coordinates(x963: &[u8]) -> Result<(String, String), StatementError> {
    if x963.len() != 65 || x963[0] != 0x04 {
        return Err(StatementError::Malformed(
            "the console key is not an uncompressed P-256 point",
        ));
    }
    VerifyingKey::from_sec1_bytes(x963)
        .map_err(|_| StatementError::Malformed("the console key is not a point on P-256"))?;
    Ok((
        URL_SAFE_NO_PAD.encode(&x963[1..33]),
        URL_SAFE_NO_PAD.encode(&x963[33..65]),
    ))
}

/// RFC 7638 thumbprint of a P-256 public key given as SEC1 uncompressed bytes.
pub fn thumbprint(x963: &[u8]) -> Result<String, StatementError> {
    let (x, y) = coordinates(x963)?;
    // RFC 7638 section 3.2: the required members of an EC key, in
    // lexicographic order, with no whitespace, hashed with SHA-256.
    let canonical = format!(r#"{{"crv":"P-256","kty":"EC","x":"{x}","y":"{y}"}}"#);
    Ok(URL_SAFE_NO_PAD.encode(Sha256::digest(canonical.as_bytes())))
}

/// The public half of the console key as a JWKS, the file wardryx reads as its
/// approver key set.
pub fn public_jwks(x963: &[u8]) -> Result<serde_json::Value, StatementError> {
    let (x, y) = coordinates(x963)?;
    let kid = thumbprint(x963)?;
    Ok(serde_json::json!({
        "keys": [{
            "kty": "EC",
            "crv": "P-256",
            "alg": "ES256",
            "use": "sig",
            "kid": kid,
            "x": x,
            "y": y,
        }]
    }))
}

/// Make a statement with the console key. The claims are checked first, so a
/// statement wardryx would refuse is never made.
pub fn sign(signer: &dyn Es256Signer, claims: &Claims) -> Result<String, StatementError> {
    check_claims(claims)?;
    let kid = thumbprint(&signer.public_key_x963()?)?;
    let header = serde_json::json!({ "alg": "ES256", "kid": kid, "typ": TYP });
    let payload = serde_json::json!({
        "v": 1,
        "sub": claims.sub,
        "cred": claims.cred,
        "aid": claims.aid,
        "dec": claims.dec,
        "iat": claims.iat,
        "exp": claims.exp,
        "jti": claims.jti,
    });
    let signing_input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(payload.to_string())
    );
    let raw = signer.sign_raw(signing_input.as_bytes())?;
    Ok(format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(raw)))
}

/// Verify a statement against the console public key at time `now`, and
/// return its claims.
pub fn verify(x963: &[u8], statement: &str, now: i64) -> Result<Claims, StatementError> {
    let (header_b64, payload_b64, sig_b64) =
        match statement.split('.').collect::<Vec<_>>().as_slice() {
            [h, p, s] => (*h, *p, *s),
            _ => {
                return Err(StatementError::Malformed(
                    "not a compact JWS of three parts",
                ));
            }
        };

    let header: serde_json::Value =
        serde_json::from_slice(&decode(header_b64, "the header is not base64url")?)
            .map_err(|_| StatementError::Malformed("the header is not JSON"))?;
    if header.get("alg").and_then(|v| v.as_str()) != Some("ES256") {
        return Err(StatementError::Malformed("the algorithm is not ES256"));
    }
    if header.get("typ").and_then(|v| v.as_str()) != Some(TYP) {
        return Err(StatementError::Malformed(
            "the type is not the approver statement type",
        ));
    }
    if header.get("jwk").is_some() || header.get("x5c").is_some() {
        return Err(StatementError::Malformed(
            "the header carries a key of its own",
        ));
    }
    let want_kid = thumbprint(x963)?;
    if header.get("kid").and_then(|v| v.as_str()) != Some(want_kid.as_str()) {
        return Err(StatementError::Malformed(
            "the key id is not the console key's thumbprint",
        ));
    }

    let sig = decode(sig_b64, "the signature is not base64url")?;
    verify_es256(x963, format!("{header_b64}.{payload_b64}").as_bytes(), &sig)
        .map_err(|_| StatementError::Unverified)?;

    let payload: serde_json::Value =
        serde_json::from_slice(&decode(payload_b64, "the payload is not base64url")?)
            .map_err(|_| StatementError::Malformed("the payload is not JSON"))?;
    if payload.get("v").and_then(|v| v.as_i64()) != Some(1) {
        return Err(StatementError::Malformed("the claim version is not 1"));
    }
    let claims = Claims {
        sub: text_claim(&payload, "sub")?,
        cred: text_claim(&payload, "cred")?,
        aid: text_claim(&payload, "aid")?,
        dec: text_claim(&payload, "dec")?,
        iat: number_claim(&payload, "iat")?,
        exp: number_claim(&payload, "exp")?,
        jti: text_claim(&payload, "jti")?,
    };
    check_claims(&claims)?;
    if claims.iat > now + MAX_FUTURE_SKEW_SECS {
        return Err(StatementError::Claims(
            "the statement is issued in the future",
        ));
    }
    if claims.exp <= now {
        return Err(StatementError::Claims("the statement has expired"));
    }
    Ok(claims)
}

fn decode(part: &str, why: &'static str) -> Result<Vec<u8>, StatementError> {
    URL_SAFE_NO_PAD
        .decode(part)
        .map_err(|_| StatementError::Malformed(why))
}

fn text_claim(payload: &serde_json::Value, name: &str) -> Result<String, StatementError> {
    payload
        .get(name)
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or(StatementError::Malformed(
            "a required text claim is missing",
        ))
}

fn number_claim(payload: &serde_json::Value, name: &str) -> Result<i64, StatementError> {
    payload
        .get(name)
        .and_then(|v| v.as_i64())
        .ok_or(StatementError::Malformed(
            "a required number claim is missing",
        ))
}

/// The rules wardryx applies to the claims, checked on both sides of the wire.
fn check_claims(c: &Claims) -> Result<(), StatementError> {
    let sub_ok = !c.sub.is_empty()
        && c.sub.len() <= 256
        && !c
            .sub
            .chars()
            .any(|ch| ch.is_control() || is_bidi_control(ch));
    if !sub_ok {
        return Err(StatementError::Claims(
            "sub must be a non-empty label of at most 256 bytes, with no control or bidi character",
        ));
    }
    let cred_ok = !c.cred.is_empty()
        && c.cred.len() <= 512
        && c.cred
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if !cred_ok {
        return Err(StatementError::Claims(
            "cred must be base64url text of at most 512 bytes",
        ));
    }
    if c.aid.is_empty() {
        return Err(StatementError::Claims("aid must name the approval"));
    }
    if c.dec != "grant" && c.dec != "deny" {
        return Err(StatementError::Claims("dec must be grant or deny"));
    }
    if c.jti.is_empty() || c.jti.len() > 128 {
        return Err(StatementError::Claims(
            "jti must be a non-empty id of at most 128 bytes",
        ));
    }
    let lifetime = c.exp - c.iat;
    if !(1..=MAX_LIFETIME_SECS).contains(&lifetime) {
        return Err(StatementError::Claims(
            "the lifetime must be positive and at most five minutes",
        ));
    }
    Ok(())
}

/// Bidirectional control characters, which can make a label read as something
/// other than what it is.
fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// Read a P-256 private key from a PKCS#8 PEM (`-----BEGIN PRIVATE KEY-----`).
///
/// Only that label and only P-256 are accepted: an RSA or SEC1 `EC PRIVATE KEY`
/// is refused rather than guessed at. When the PEM carries the public half, it
/// must be the public key of the scalar, or the key is refused.
pub fn signer_from_pkcs8_pem(pem: &str) -> Result<SoftwareSigner, StatementError> {
    let body = pem
        .trim()
        .strip_prefix("-----BEGIN PRIVATE KEY-----")
        .and_then(|rest| rest.strip_suffix("-----END PRIVATE KEY-----"))
        .ok_or(StatementError::Malformed(
            "the approver key file is not a PKCS#8 PRIVATE KEY PEM",
        ))?;
    let der = base64::engine::general_purpose::STANDARD
        .decode(body.split_whitespace().collect::<String>())
        .map_err(|_| StatementError::Malformed("the approver key file's PEM body is not base64"))?;
    let (scalar, public) = parse_p256_pkcs8(&der)?;
    let signer = SoftwareSigner::from_scalar(&scalar).map_err(|_| {
        StatementError::Malformed("the approver key's scalar is not a P-256 scalar")
    })?;
    if let Some(public) = public
        && signer.public_key_x963()? != public
    {
        return Err(StatementError::Malformed(
            "the approver key's public half does not match its scalar",
        ));
    }
    Ok(signer)
}

/// `ecPublicKey` 1.2.840.10045.2.1, as its DER content octets.
const OID_EC_PUBLIC_KEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
/// `prime256v1` 1.2.840.10045.3.1.7, as its DER content octets.
const OID_PRIME256V1: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];

/// One DER element, read strictly: its tag, its value, and what follows it.
fn der_read(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 || rest.len() < n {
            return None;
        }
        let len = rest[..n]
            .iter()
            .fold(0usize, |acc, &b| (acc << 8) | usize::from(b));
        (len, &rest[n..])
    };
    if rest.len() < len {
        return None;
    }
    let (value, tail) = rest.split_at(len);
    Some((tag, value, tail))
}

/// The scalar and the optional public half of a P-256 PKCS#8 key: RFC 5208's
/// PrivateKeyInfo around RFC 5915's ECPrivateKey. Every element is checked for
/// its tag, and anything outside that shape is refused.
fn parse_p256_pkcs8(der: &[u8]) -> Result<([u8; 32], Option<Vec<u8>>), StatementError> {
    let bad = || StatementError::Malformed("the approver key is not a P-256 PKCS#8 structure");
    // PrivateKeyInfo ::= SEQUENCE { version INTEGER 0, algorithm, privateKey }
    let (tag, info, rest) = der_read(der).ok_or_else(bad)?;
    if tag != 0x30 || !rest.is_empty() {
        return Err(bad());
    }
    let (tag, version, info) = der_read(info).ok_or_else(bad)?;
    if tag != 0x02 || version != &[0u8][..] {
        return Err(bad());
    }
    let (tag, algorithm, info) = der_read(info).ok_or_else(bad)?;
    if tag != 0x30 {
        return Err(bad());
    }
    let (tag, oid, algorithm) = der_read(algorithm).ok_or_else(bad)?;
    if tag != 0x06 || oid != OID_EC_PUBLIC_KEY {
        return Err(bad());
    }
    let (tag, curve, algorithm) = der_read(algorithm).ok_or_else(bad)?;
    if tag != 0x06 || curve != OID_PRIME256V1 || !algorithm.is_empty() {
        return Err(bad());
    }
    let (tag, key, info) = der_read(info).ok_or_else(bad)?;
    if tag != 0x04 || !info.is_empty() {
        return Err(bad());
    }
    // ECPrivateKey ::= SEQUENCE { version INTEGER 1, privateKey OCTET STRING,
    //   parameters [0] OPTIONAL, publicKey [1] OPTIONAL }
    let (tag, ec, key_rest) = der_read(key).ok_or_else(bad)?;
    if tag != 0x30 || !key_rest.is_empty() {
        return Err(bad());
    }
    let (tag, version, ec) = der_read(ec).ok_or_else(bad)?;
    if tag != 0x02 || version != &[1u8][..] {
        return Err(bad());
    }
    let (tag, scalar, ec) = der_read(ec).ok_or_else(bad)?;
    if tag != 0x04 || scalar.len() != 32 {
        return Err(bad());
    }
    let scalar: [u8; 32] = scalar.try_into().map_err(|_| bad())?;
    let mut ec = ec;
    if let Some((0xA0, _, after)) = der_read(ec) {
        ec = after;
    }
    let mut public = None;
    if let Some((0xA1, wrapped, after)) = der_read(ec) {
        // [1] EXPLICIT BIT STRING: no unused bits, then 0x04 || X || Y.
        let (tag, bits, _) = der_read(wrapped).ok_or_else(bad)?;
        if tag != 0x03 {
            return Err(bad());
        }
        match bits.split_first() {
            Some((0, point)) if point.len() == 65 && point[0] == 0x04 => {
                public = Some(point.to_vec());
            }
            _ => return Err(bad()),
        }
        ec = after;
    }
    if !ec.is_empty() {
        return Err(bad());
    }
    Ok((scalar, public))
}

/// Encode a P-256 private key as a PKCS#8 PEM, the inverse of
/// [`signer_from_pkcs8_pem`]. `public` is the SEC1 public key to carry beside
/// the scalar, or `None` to leave it out.
pub fn pkcs8_pem(scalar: &[u8; 32], public: Option<&[u8]>) -> String {
    let der = pkcs8_der(scalar, public);
    let body = base64::engine::general_purpose::STANDARD.encode(der);
    format!("-----BEGIN PRIVATE KEY-----\n{body}\n-----END PRIVATE KEY-----\n")
}

/// One DER element: a tag, its length in the short or long form, and its value.
fn der_tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    match body.len() {
        n if n < 0x80 => out.push(n as u8),
        n if n < 0x100 => out.extend_from_slice(&[0x81, n as u8]),
        n => out.extend_from_slice(&[0x82, (n >> 8) as u8, n as u8]),
    }
    out.extend_from_slice(body);
    out
}

/// RFC 5208 PrivateKeyInfo for a P-256 key, with the RFC 5915 ECPrivateKey
/// inside it, carrying the public key only when one is given.
fn pkcs8_der(scalar: &[u8; 32], public: Option<&[u8]>) -> Vec<u8> {
    let mut ec_private = der_tlv(0x02, &[1]);
    ec_private.extend(der_tlv(0x04, scalar));
    if let Some(p) = public {
        let mut bits = vec![0u8];
        bits.extend_from_slice(p);
        ec_private.extend(der_tlv(0xA1, &der_tlv(0x03, &bits)));
    }
    // ecPublicKey 1.2.840.10045.2.1 and prime256v1 1.2.840.10045.3.1.7.
    let mut algorithm = der_tlv(0x06, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]);
    algorithm.extend(der_tlv(
        0x06,
        &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07],
    ));
    let mut info = der_tlv(0x02, &[0]);
    info.extend(der_tlv(0x30, &algorithm));
    info.extend(der_tlv(0x04, &der_tlv(0x30, &ec_private)));
    der_tlv(0x30, &info)
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

#[cfg(test)]
mod loader_tests {
    use super::*;

    fn key_der_pem(scalar: [u8; 32], public: Option<&[u8]>) -> String {
        pkcs8_pem(&scalar, public)
    }

    #[test]
    fn a_pkcs8_pem_key_loads_to_the_scalar_it_holds() {
        let pem = key_der_pem([7u8; 32], None);
        let signer = signer_from_pkcs8_pem(&pem).expect("a P-256 PKCS#8 key loads");
        assert_eq!(signer.to_scalar_bytes(), [7u8; 32]);
    }

    #[test]
    fn a_pkcs8_key_with_its_public_half_loads_when_the_two_match() {
        let public = SoftwareSigner::from_scalar(&[7u8; 32])
            .unwrap()
            .public_key_x963()
            .unwrap();
        let pem = key_der_pem([7u8; 32], Some(&public));
        let signer = signer_from_pkcs8_pem(&pem).expect("the public half matches the scalar");
        assert_eq!(signer.public_key_x963().unwrap(), public);
    }

    #[test]
    fn a_key_whose_public_half_differs_from_its_scalar_is_refused() {
        let other = SoftwareSigner::from_scalar(&[8u8; 32])
            .unwrap()
            .public_key_x963()
            .unwrap();
        let pem = key_der_pem([7u8; 32], Some(&other));
        assert!(matches!(
            signer_from_pkcs8_pem(&pem),
            Err(StatementError::Malformed(_))
        ));
    }

    #[test]
    fn a_key_that_is_not_p256_pkcs8_pem_is_refused() {
        let pem = key_der_pem([7u8; 32], None);
        let rsa_label = pem.replace("BEGIN PRIVATE KEY", "BEGIN RSA PRIVATE KEY");
        let sec1_label = pem.replace("BEGIN PRIVATE KEY", "BEGIN EC PRIVATE KEY");
        for (name, bad) in [
            ("not PEM at all", "just some text".to_string()),
            ("an RSA label", rsa_label),
            ("a SEC1 EC label", sec1_label),
            (
                "an empty body",
                "-----BEGIN PRIVATE KEY-----\n-----END PRIVATE KEY-----\n".to_string(),
            ),
        ] {
            assert!(
                signer_from_pkcs8_pem(&bad).is_err(),
                "{name} must be refused, not loaded"
            );
        }
    }
}
