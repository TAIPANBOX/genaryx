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
    /// What the person was shown (wardryx invariant 51): `Some` makes a
    /// version 2 statement carrying `tcd` and `pvd`, `None` the version 1
    /// statement every wardryx before 2.7 reads.
    pub shown: Option<Shown>,
}

/// The call a person decided and the preview of its arguments they were shown,
/// each a lower-case hex sha-256 or [`NONE`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// `tcd`: the digest of the held call, or `none` for a hold with no call.
    pub tcd: String,
    /// `pvd`: the digest of the preview the person was shown, or `none`.
    pub pvd: String,
}

/// The value `tcd` or `pvd` carries when there is no call, or no preview.
pub const NONE: &str = "none";

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
    let mut payload = serde_json::json!({
        "v": 1,
        "sub": claims.sub,
        "cred": claims.cred,
        "aid": claims.aid,
        "dec": claims.dec,
        "iat": claims.iat,
        "exp": claims.exp,
        "jti": claims.jti,
    });
    if let Some(shown) = &claims.shown {
        payload["v"] = serde_json::json!(2);
        payload["tcd"] = serde_json::json!(shown.tcd);
        payload["pvd"] = serde_json::json!(shown.pvd);
    }
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
    let shown = match payload.get("v").and_then(|v| v.as_i64()) {
        Some(1) => {
            if payload.get("tcd").is_some() || payload.get("pvd").is_some() {
                return Err(StatementError::Malformed(
                    "a version 1 statement carries tcd or pvd",
                ));
            }
            None
        }
        Some(2) => Some(Shown {
            tcd: text_claim(&payload, "tcd")?,
            pvd: text_claim(&payload, "pvd")?,
        }),
        _ => return Err(StatementError::Malformed("the claim version is not 1 or 2")),
    };
    let claims = Claims {
        sub: text_claim(&payload, "sub")?,
        cred: text_claim(&payload, "cred")?,
        aid: text_claim(&payload, "aid")?,
        dec: text_claim(&payload, "dec")?,
        iat: number_claim(&payload, "iat")?,
        exp: number_claim(&payload, "exp")?,
        jti: text_claim(&payload, "jti")?,
        shown,
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
    if let Some(shown) = &c.shown
        && (!digest_or_none(&shown.tcd) || !digest_or_none(&shown.pvd))
    {
        return Err(StatementError::Claims(
            "tcd and pvd must each be a lower-case hex sha-256 or none",
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

/// A lower-case hex sha-256, or [`NONE`].
fn digest_or_none(s: &str) -> bool {
    s == NONE
        || (s.len() == 64
            && s.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)))
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
            shown: None,
        }
    }

    fn payload_of(jws: &str) -> serde_json::Value {
        serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(jws.split('.').nth(1).unwrap())
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn a_version_2_statement_names_the_call_and_the_preview_the_person_was_shown() {
        let k = key();
        let x = k.public_key_x963().unwrap();
        let mut c = claims();
        c.shown = Some(Shown {
            tcd: "a".repeat(64),
            pvd: "b".repeat(64),
        });
        let jws = sign(&k, &c).expect("signed");
        let p = payload_of(&jws);
        assert_eq!(p["v"], 2);
        assert_eq!(p["tcd"], "a".repeat(64));
        assert_eq!(p["pvd"], "b".repeat(64));
        assert_eq!(verify(&x, &jws, 1_010).expect("verifies"), c);
        // none is how a statement says no preview was shown.
        c.shown = Some(Shown {
            tcd: "a".repeat(64),
            pvd: NONE.into(),
        });
        assert_eq!(verify(&x, &sign(&k, &c).unwrap(), 1_010).unwrap(), c);
    }

    #[test]
    fn a_version_1_statement_carries_no_tcd_or_pvd() {
        let k = key();
        let jws = sign(&k, &claims()).expect("signed");
        let p = payload_of(&jws);
        assert_eq!(p["v"], 1);
        assert!(p.get("tcd").is_none() && p.get("pvd").is_none());
    }

    #[test]
    fn a_malformed_tcd_or_pvd_is_refused_before_anything_is_signed() {
        let k = key();
        for bad in [
            "",
            "None",
            &"A".repeat(64),
            &"a".repeat(63),
            &"z".repeat(64),
        ] {
            for which in 0..2 {
                let mut c = claims();
                let (tcd, pvd) = if which == 0 {
                    (bad.to_string(), NONE.to_string())
                } else {
                    (NONE.to_string(), bad.to_string())
                };
                c.shown = Some(Shown { tcd, pvd });
                assert!(
                    matches!(sign(&k, &c), Err(StatementError::Claims(_))),
                    "{bad:?} accepted"
                );
            }
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

    // ---- forged statements: each signed with the REAL console key, so the
    // only thing wrong with it is the thing the test names ------------------

    fn b64(v: &serde_json::Value) -> String {
        URL_SAFE_NO_PAD.encode(v.to_string())
    }

    /// A compact JWS over exactly this header and payload, signed with `k`.
    fn forge(k: &SoftwareSigner, header: &serde_json::Value, payload: &serde_json::Value) -> String {
        let input = format!("{}.{}", b64(header), b64(payload));
        let sig = k.sign_raw(input.as_bytes()).unwrap();
        format!("{input}.{}", URL_SAFE_NO_PAD.encode(sig))
    }

    fn good_header(k: &SoftwareSigner) -> serde_json::Value {
        let kid = thumbprint(&k.public_key_x963().unwrap()).unwrap();
        serde_json::json!({ "alg": "ES256", "kid": kid, "typ": TYP })
    }

    fn good_payload() -> serde_json::Value {
        serde_json::json!({
            "v": 1, "sub": "person://acme/alice", "cred": "Y3JlZC0x", "aid": "appr-7",
            "dec": "grant", "iat": 1_000, "exp": 1_120, "jti": "stmt-1",
        })
    }

    fn malformed_reason(r: Result<Claims, StatementError>) -> &'static str {
        match r {
            Err(StatementError::Malformed(m)) => m,
            other => panic!("expected a malformed statement, got {other:?}"),
        }
    }

    #[test]
    fn the_untampered_forge_is_accepted_so_every_refusal_below_is_about_its_one_change() {
        let k = key();
        let x = k.public_key_x963().unwrap();
        let jws = forge(&k, &good_header(&k), &good_payload());
        assert_eq!(verify(&x, &jws, 1_010).unwrap(), claims());
    }

    #[test]
    fn a_header_that_names_another_algorithm_type_or_key_is_refused() {
        let k = key();
        let x = k.public_key_x963().unwrap();
        let mut cases: Vec<(serde_json::Value, &str)> = Vec::new();
        for (field, value, why) in [
            ("alg", serde_json::json!("none"), "the algorithm is not ES256"),
            ("alg", serde_json::json!("HS256"), "the algorithm is not ES256"),
            ("typ", serde_json::json!("JWT"), "the type is not the approver statement type"),
            (
                "kid",
                serde_json::json!("another-key"),
                "the key id is not the console key's thumbprint",
            ),
            (
                "jwk",
                serde_json::json!({"kty": "EC"}),
                "the header carries a key of its own",
            ),
            (
                "x5c",
                serde_json::json!(["MIIB"]),
                "the header carries a key of its own",
            ),
        ] {
            let mut h = good_header(&k);
            h[field] = value;
            cases.push((h, why));
        }
        let mut no_alg = good_header(&k);
        no_alg.as_object_mut().unwrap().remove("alg");
        cases.push((no_alg, "the algorithm is not ES256"));
        for (header, why) in cases {
            let jws = forge(&k, &header, &good_payload());
            assert_eq!(malformed_reason(verify(&x, &jws, 1_010)), why, "{header}");
        }
    }

    #[test]
    fn a_statement_that_is_not_three_base64url_json_parts_is_refused() {
        let k = key();
        let x = k.public_key_x963().unwrap();
        let good = forge(&k, &good_header(&k), &good_payload());
        let p = parts(&good);
        for (jws, why) in [
            ("a.b".to_string(), "not a compact JWS of three parts"),
            (format!("{good}.extra"), "not a compact JWS of three parts"),
            (String::new(), "not a compact JWS of three parts"),
            (format!("!!.{}.{}", p[1], p[2]), "the header is not base64url"),
            (
                format!("{}.{}.{}", URL_SAFE_NO_PAD.encode("not json"), p[1], p[2]),
                "the header is not JSON",
            ),
            (format!("{}.{}.%%", p[0], p[1]), "the signature is not base64url"),
        ] {
            assert_eq!(malformed_reason(verify(&x, &jws, 1_010)), why, "{jws}");
        }
        // A signature that decodes but is not one for this input.
        let wrong_sig = format!("{}.{}.{}", p[0], p[1], URL_SAFE_NO_PAD.encode([0u8; 64]));
        assert!(matches!(
            verify(&x, &wrong_sig, 1_010),
            Err(StatementError::Unverified)
        ));
        let short_sig = format!("{}.{}.{}", p[0], p[1], URL_SAFE_NO_PAD.encode([1u8; 10]));
        assert!(matches!(
            verify(&x, &short_sig, 1_010),
            Err(StatementError::Unverified)
        ));
    }

    #[test]
    fn a_validly_signed_payload_that_is_not_a_statement_is_still_refused() {
        // Each of these carries a GOOD signature: the signer alone is never
        // enough, the payload must also be a statement of this shape.
        let k = key();
        let x = k.public_key_x963().unwrap();
        let h = good_header(&k);
        let input = format!("{}.{}", b64(&h), URL_SAFE_NO_PAD.encode("not json"));
        let sig = URL_SAFE_NO_PAD.encode(k.sign_raw(input.as_bytes()).unwrap());
        assert_eq!(
            malformed_reason(verify(&x, &format!("{input}.{sig}"), 1_010)),
            "the payload is not JSON"
        );
        let input = format!("{}.%%%", b64(&h));
        let sig = URL_SAFE_NO_PAD.encode(k.sign_raw(input.as_bytes()).unwrap());
        assert_eq!(
            malformed_reason(verify(&x, &format!("{input}.{sig}"), 1_010)),
            "the payload is not base64url"
        );

        let mut cases: Vec<(serde_json::Value, &str)> = Vec::new();
        let mut v3 = good_payload();
        v3["v"] = serde_json::json!(3);
        cases.push((v3, "the claim version is not 1 or 2"));
        let mut no_v = good_payload();
        no_v.as_object_mut().unwrap().remove("v");
        cases.push((no_v, "the claim version is not 1 or 2"));
        let mut v1_with_tcd = good_payload();
        v1_with_tcd["tcd"] = serde_json::json!(NONE);
        cases.push((v1_with_tcd, "a version 1 statement carries tcd or pvd"));
        let mut v1_with_pvd = good_payload();
        v1_with_pvd["pvd"] = serde_json::json!(NONE);
        cases.push((v1_with_pvd, "a version 1 statement carries tcd or pvd"));
        let mut v2_without_pvd = good_payload();
        v2_without_pvd["v"] = serde_json::json!(2);
        v2_without_pvd["tcd"] = serde_json::json!(NONE);
        cases.push((v2_without_pvd, "a required text claim is missing"));
        for claim in ["sub", "cred", "aid", "dec", "jti"] {
            let mut p = good_payload();
            p[claim] = serde_json::json!(7);
            cases.push((p, "a required text claim is missing"));
        }
        for claim in ["iat", "exp"] {
            let mut p = good_payload();
            p[claim] = serde_json::json!("1000");
            cases.push((p, "a required number claim is missing"));
        }
        for (payload, why) in cases {
            let jws = forge(&k, &h, &payload);
            assert_eq!(malformed_reason(verify(&x, &jws, 1_010)), why, "{payload}");
        }
    }

    #[test]
    fn signed_claims_outside_the_rules_are_refused_on_the_verifying_side_too() {
        // A statement wardryx would refuse is refused here even though the
        // console key signed it (a different console build, or a forged one).
        let k = key();
        let x = k.public_key_x963().unwrap();
        let h = good_header(&k);
        let mut maybe = good_payload();
        maybe["dec"] = serde_json::json!("maybe");
        let mut long_life = good_payload();
        long_life["exp"] = serde_json::json!(1_000 + MAX_LIFETIME_SECS + 1);
        let mut zero_life = good_payload();
        zero_life["exp"] = serde_json::json!(1_000);
        let mut bad_digest = good_payload();
        bad_digest["v"] = serde_json::json!(2);
        bad_digest["tcd"] = serde_json::json!("A".repeat(64));
        bad_digest["pvd"] = serde_json::json!(NONE);
        for p in [maybe, long_life, zero_life, bad_digest] {
            let jws = forge(&k, &h, &p);
            assert!(
                matches!(verify(&x, &jws, 1_010), Err(StatementError::Claims(_))),
                "{p}"
            );
        }
        // Thirty seconds of clock skew is allowed, and not a second more.
        let jws = forge(&k, &h, &good_payload());
        assert!(verify(&x, &jws, 1_000 - MAX_FUTURE_SKEW_SECS).is_ok());
        assert!(matches!(
            verify(&x, &jws, 1_000 - MAX_FUTURE_SKEW_SECS - 1),
            Err(StatementError::Claims("the statement is issued in the future"))
        ));
        assert!(verify(&x, &jws, 1_119).is_ok(), "valid until the second before exp");
    }

    #[test]
    fn every_claim_rule_has_its_own_boundary() {
        let k = key();
        let mut ok = claims();
        ok.sub = "s".repeat(256);
        ok.cred = "c".repeat(512);
        ok.jti = "j".repeat(128);
        ok.exp = ok.iat + MAX_LIFETIME_SECS;
        assert!(sign(&k, &ok).is_ok(), "every limit is inclusive");
        let mut cases: Vec<(Claims, &str)> = Vec::new();
        let mut c = claims();
        c.sub = "s".repeat(257);
        cases.push((c, "a 257-byte subject"));
        let mut c = claims();
        c.cred = String::new();
        cases.push((c, "an empty credential id"));
        let mut c = claims();
        c.cred = "c".repeat(513);
        cases.push((c, "a 513-byte credential id"));
        let mut c = claims();
        c.aid = String::new();
        cases.push((c, "no approval id"));
        let mut c = claims();
        c.jti = "j".repeat(129);
        cases.push((c, "a 129-byte statement id"));
        let mut c = claims();
        c.exp = c.iat;
        cases.push((c, "a zero lifetime"));
        let mut c = claims();
        c.exp = c.iat - 1;
        cases.push((c, "a negative lifetime"));
        let mut c = claims();
        c.sub = "person://acme/\u{2066}alice".into();
        cases.push((c, "an isolate in the subject"));
        let mut c = claims();
        c.sub = "person://acme/\u{061C}alice".into();
        cases.push((c, "an Arabic letter mark in the subject"));
        for (c, name) in cases {
            assert!(
                matches!(sign(&k, &c), Err(StatementError::Claims(_))),
                "{name} must be refused"
            );
        }
        let mut deny = claims();
        deny.dec = "deny".into();
        assert!(sign(&k, &deny).is_ok(), "deny is a decision too");
    }

    #[test]
    fn a_console_key_that_is_not_an_uncompressed_p256_point_is_refused() {
        let x = key().public_key_x963().unwrap();
        let mut compressed = x[..33].to_vec();
        compressed[0] = 0x02;
        let mut wrong_tag = x.clone();
        wrong_tag[0] = 0x05;
        let mut off_curve = x;
        off_curve[64] ^= 0x01;
        assert_eq!(
            thumbprint(&compressed).unwrap_err().to_string(),
            "the console key is not an uncompressed P-256 point"
        );
        assert!(matches!(
            thumbprint(&wrong_tag),
            Err(StatementError::Malformed(_))
        ));
        assert!(matches!(
            public_jwks(&[]),
            Err(StatementError::Malformed(_))
        ));
        assert_eq!(
            thumbprint(&off_curve).unwrap_err().to_string(),
            "the console key is not a point on P-256"
        );
        // A statement checked against a key that is not a key is refused,
        // never verified.
        let jws = sign(&key(), &claims()).unwrap();
        assert!(matches!(
            verify(&off_curve, &jws, 1_010),
            Err(StatementError::Malformed(_))
        ));
    }

    #[test]
    fn statement_errors_say_what_failed() {
        assert_eq!(
            StatementError::Claims("dec must be grant or deny").to_string(),
            "the approver statement's claims are refused: dec must be grant or deny"
        );
        assert_eq!(
            StatementError::Unverified.to_string(),
            "the approver statement does not verify under the console key"
        );
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


    // ---- the DER reader, held element by element --------------------------

    fn info(version: &[u8], alg_oid: &[u8], curve: &[u8], ec: &[u8]) -> Vec<u8> {
        let mut algorithm = der_tlv(0x06, alg_oid);
        algorithm.extend(der_tlv(0x06, curve));
        let mut info = der_tlv(0x02, version);
        info.extend(der_tlv(0x30, &algorithm));
        info.extend(der_tlv(0x04, &der_tlv(0x30, ec)));
        der_tlv(0x30, &info)
    }

    fn ec(version: &[u8], scalar: &[u8], extra: &[u8]) -> Vec<u8> {
        let mut e = der_tlv(0x02, version);
        e.extend(der_tlv(0x04, scalar));
        e.extend_from_slice(extra);
        e
    }

    fn public_of(scalar: [u8; 32]) -> Vec<u8> {
        SoftwareSigner::from_scalar(&scalar)
            .unwrap()
            .public_key_x963()
            .unwrap()
    }

    fn public_field(bits: &[u8]) -> Vec<u8> {
        der_tlv(0xA1, &der_tlv(0x03, bits))
    }

    #[test]
    fn der_lengths_round_trip_in_the_short_and_both_long_forms() {
        for n in [0usize, 127, 128, 255, 256, 300, 65_535] {
            let body = vec![0x5a; n];
            let tlv = der_tlv(0x04, &body);
            let (tag, value, rest) = der_read(&tlv).expect("readable");
            assert_eq!((tag, value.len(), rest.len()), (0x04, n, 0), "length {n}");
        }
        assert_eq!(&der_tlv(0x04, &[0; 200])[..3], &[0x04, 0x81, 200]);
        assert_eq!(&der_tlv(0x04, &[0; 300])[..4], &[0x04, 0x82, 0x01, 0x2c]);
    }

    #[test]
    fn der_that_lies_about_its_length_is_unreadable() {
        for (name, bad) in [
            ("nothing", &[][..]),
            ("a tag alone", &[0x30][..]),
            ("a long form naming no length bytes", &[0x30, 0x80][..]),
            ("a long form of five length bytes", &[0x30, 0x85, 0, 0, 0, 0, 1][..]),
            ("length bytes that are not there", &[0x30, 0x82, 0x01][..]),
            ("a length past the end", &[0x30, 0x03, 0x01][..]),
            ("a long length past the end", &[0x30, 0x81, 0x80, 0x00][..]),
        ] {
            assert!(der_read(bad).is_none(), "{name} was read");
        }
        assert_eq!(der_read(&[0x05, 0x00, 0xff]), Some((0x05, &[][..], &[0xff][..])));
    }

    #[test]
    fn the_hand_built_structure_is_the_encoder_s_so_each_change_below_is_the_only_one() {
        let good = info(&[0], OID_EC_PUBLIC_KEY, OID_PRIME256V1, &ec(&[1], &[7u8; 32], &[]));
        assert_eq!(good, pkcs8_der(&[7u8; 32], None));
        let (scalar, public) = parse_p256_pkcs8(&good).expect("the encoder's own output parses");
        assert_eq!((scalar, public), ([7u8; 32], None));
    }

    #[test]
    fn a_pkcs8_structure_off_its_shape_anywhere_is_refused() {
        let scalar = [7u8; 32];
        let public = public_of(scalar);
        let mut unused_bits = vec![1u8];
        unused_bits.extend_from_slice(&public);
        let mut short_point = vec![0u8];
        short_point.extend_from_slice(&public[..64]);
        let mut compressed = vec![0u8, 0x02];
        compressed.extend_from_slice(&public[1..33]);
        let mut good = info(&[0], OID_EC_PUBLIC_KEY, OID_PRIME256V1, &ec(&[1], &scalar, &[]));
        good.push(0x00);
        let rsa: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
        let p384: &[u8] = &[0x2b, 0x81, 0x04, 0x00, 0x22];
        let mut bits_in_octets = vec![0u8];
        bits_in_octets.extend_from_slice(&public);
        let cases = [
            ("trailing bytes after the key", good),
            (
                "PrivateKeyInfo version 1",
                info(&[1], OID_EC_PUBLIC_KEY, OID_PRIME256V1, &ec(&[1], &scalar, &[])),
            ),
            ("an RSA algorithm", info(&[0], rsa, OID_PRIME256V1, &ec(&[1], &scalar, &[]))),
            ("the P-384 curve", info(&[0], OID_EC_PUBLIC_KEY, p384, &ec(&[1], &scalar, &[]))),
            (
                "a 31-byte scalar",
                info(&[0], OID_EC_PUBLIC_KEY, OID_PRIME256V1, &ec(&[1], &scalar[..31], &[])),
            ),
            (
                "ECPrivateKey version 2",
                info(&[0], OID_EC_PUBLIC_KEY, OID_PRIME256V1, &ec(&[2], &scalar, &[])),
            ),
            (
                "a public key with unused bits",
                info(
                    &[0],
                    OID_EC_PUBLIC_KEY,
                    OID_PRIME256V1,
                    &ec(&[1], &scalar, &public_field(&unused_bits)),
                ),
            ),
            (
                "a public point one byte short",
                info(
                    &[0],
                    OID_EC_PUBLIC_KEY,
                    OID_PRIME256V1,
                    &ec(&[1], &scalar, &public_field(&short_point)),
                ),
            ),
            (
                "a compressed public point",
                info(
                    &[0],
                    OID_EC_PUBLIC_KEY,
                    OID_PRIME256V1,
                    &ec(&[1], &scalar, &public_field(&compressed)),
                ),
            ),
            (
                "a public key that is not a BIT STRING",
                info(
                    &[0],
                    OID_EC_PUBLIC_KEY,
                    OID_PRIME256V1,
                    &ec(&[1], &scalar, &der_tlv(0xA1, &der_tlv(0x04, &bits_in_octets))),
                ),
            ),
            (
                "an unknown trailing element",
                info(
                    &[0],
                    OID_EC_PUBLIC_KEY,
                    OID_PRIME256V1,
                    &ec(&[1], &scalar, &der_tlv(0x05, &[])),
                ),
            ),
            ("not DER at all", vec![0xff]),
        ];
        for (name, der) in cases {
            assert!(
                matches!(parse_p256_pkcs8(&der), Err(StatementError::Malformed(_))),
                "{name} was accepted"
            );
        }
    }

    #[test]
    fn named_curve_parameters_are_skipped_and_the_public_half_is_still_read() {
        let scalar = [7u8; 32];
        let public = public_of(scalar);
        let params = der_tlv(0xA0, &der_tlv(0x06, OID_PRIME256V1));
        let mut bits = vec![0u8];
        bits.extend_from_slice(&public);
        let mut extra = params.clone();
        extra.extend(public_field(&bits));
        let with_both = info(&[0], OID_EC_PUBLIC_KEY, OID_PRIME256V1, &ec(&[1], &scalar, &extra));
        let (got_scalar, got_public) = parse_p256_pkcs8(&with_both).expect("parses");
        assert_eq!(got_scalar, scalar);
        assert_eq!(got_public.as_deref(), Some(public.as_slice()));
        let params_only = info(&[0], OID_EC_PUBLIC_KEY, OID_PRIME256V1, &ec(&[1], &scalar, &params));
        assert_eq!(parse_p256_pkcs8(&params_only).expect("parses").1, None);
    }

    #[test]
    fn a_pem_whose_body_or_scalar_is_wrong_is_refused_by_name() {
        let not_base64 = "-----BEGIN PRIVATE KEY-----\n%%%%\n-----END PRIVATE KEY-----\n";
        assert_eq!(
            signer_from_pkcs8_pem(not_base64).err().map(|e| e.to_string()),
            Some("the approver key file's PEM body is not base64".to_string())
        );
        // A zero scalar is well-formed DER and is not a P-256 private key.
        let zero = pkcs8_pem(&[0u8; 32], None);
        assert_eq!(
            signer_from_pkcs8_pem(&zero).err().map(|e| e.to_string()),
            Some("the approver key's scalar is not a P-256 scalar".to_string())
        );
        // Line breaks inside the body, as every PEM writer wraps it, are fine.
        let pem = pkcs8_pem(&[9u8; 32], None);
        let body = pem
            .trim()
            .strip_prefix("-----BEGIN PRIVATE KEY-----")
            .and_then(|r| r.strip_suffix("-----END PRIVATE KEY-----"))
            .unwrap()
            .trim()
            .to_string();
        let (head, tail) = body.split_at(20);
        let wrapped = format!("-----BEGIN PRIVATE KEY-----\n{head}\n{tail}\n-----END PRIVATE KEY-----");
        assert_eq!(
            signer_from_pkcs8_pem(&wrapped).expect("loads").to_scalar_bytes(),
            [9u8; 32]
        );
    }
}
