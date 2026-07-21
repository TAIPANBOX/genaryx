//! ACME (RFC 8555) DNS-01 client, hand-rolled on this crate's existing
//! `reqwest` + `genaryx-signing` ES256 + `rcgen`, so the relay can obtain a
//! PUBLICLY-TRUSTED certificate for its `<relay-id>.pocket.it-rat.com` hostname
//! with no new dependency and without forking the workspace's single `rustls`
//! (the Cargo.toml's whole W1 note is about not adding a second rustls; an ACME
//! crate would drag one in, so the protocol lives here instead).
//!
//! Design (A) of the cert broker (itrat-console/14): the RELAY runs this. It
//! holds the ACME account key AND the certificate private key, and NEITHER ever
//! leaves it. The DNS-01 challenge is satisfied by the Pocket **broker**
//! ([`BrokerClient`]), which owns the DNS-zone credential the relay deliberately
//! does not have. The broker only ever sees the challenge token; it never sees a
//! key or a CSR. This is the paid/hostname trust mode; the free/local mode stays
//! self-signed + SPKI-pin (`tls.rs`), untouched.
//!
//! What is public vs private here:
//! - the ACME ACCOUNT key is an ES256 [`SoftwareSigner`] (persisted as its raw
//!   scalar, mode 0600, by the caller) -- it AUTHORIZES orders, it does not go
//!   into any certificate;
//! - the CERTIFICATE key is an `rcgen::KeyPair` (P-256) the caller persists next
//!   to the issued cert -- it is what the phone's TLS session terminates against.
//!
//! Only the pure, security-critical transforms (JWS encoding, the RFC 7638 JWK
//! thumbprint, the DNS-01 key-authorization digest, the CSR) are unit-tested
//! here; the network state machine is proven end to end against a Pebble ACME
//! server + the broker (see `tests/acme_pebble.rs`).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use genaryx_signing::es256::{Es256Signer, SigningError};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::time::Duration;

/// Every variant is a refusal, never a silent success (06 §0.5 fail-closed):
/// the caller keeps serving the existing cert (or self-signed) rather than
/// serving something unproven.
#[derive(Debug, thiserror::Error)]
pub enum AcmeError {
    #[error("acme http: {0}")]
    Http(String),
    #[error("acme protocol: {0}")]
    Protocol(String),
    /// A structured ACME `application/problem+json` error from the CA.
    #[error("acme server error [{typ}]: {detail}")]
    Server { typ: String, detail: String },
    #[error("account key: {0}")]
    Signing(#[from] SigningError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("csr: {0}")]
    Csr(String),
    #[error("cert broker: {0}")]
    Broker(String),
    #[error("timed out waiting for {0} to become valid")]
    Timeout(String),
}

/// What the relay needs to ask a CA for one hostname's certificate.
#[derive(Clone, Debug)]
pub struct AcmeConfig {
    /// The CA's ACME directory URL (Let's Encrypt in production, a Pebble
    /// server in the proof).
    pub directory_url: String,
    /// The single hostname to certify, e.g. `abc123.pocket.it-rat.com`.
    pub hostname: String,
    /// Registration/recovery contact, becomes `mailto:<email>`.
    pub contact_email: String,
    /// Upper bound on how long to keep polling an authorization/order for
    /// `valid` before failing closed.
    pub poll_timeout: Duration,
    /// Gap between polls.
    pub poll_interval: Duration,
}

impl AcmeConfig {
    /// The `_acme-challenge.<hostname>` name whose TXT record proves control.
    fn challenge_fqdn(&self) -> String {
        format!("_acme-challenge.{}", self.hostname)
    }
}

// ---------------------------------------------------------------------------
// The broker client: the relay's only channel to the DNS zone. It hands the
// broker a challenge token and gets back "it's set"; it never hands over a key.
// Wire format is lego's `httpreq` provider (default mode) so the same broker
// serves both this client and lego during bring-up: POST {fqdn, value}.
// ---------------------------------------------------------------------------

/// A client for the Pocket cert broker (`/present` + `/cleanup`).
#[derive(Clone)]
pub struct BrokerClient {
    http: reqwest::Client,
    base_url: String,
    /// The relay's broker identity (HTTP Basic user = relay id).
    user: String,
    /// The relay's broker token (HTTP Basic password). Never logged.
    token: String,
}

impl BrokerClient {
    pub fn new(
        http: reqwest::Client,
        base_url: impl Into<String>,
        user: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            user: user.into(),
            token: token.into(),
        }
    }

    async fn call(&self, route: &str, fqdn: &str, value: &str) -> Result<(), AcmeError> {
        let resp = self
            .http
            .post(format!("{}/{route}", self.base_url))
            .basic_auth(&self.user, Some(&self.token))
            .json(&serde_json::json!({ "fqdn": fqdn, "value": value }))
            .send()
            .await
            .map_err(|e| AcmeError::Broker(format!("{route}: {e}")))?;
        if !resp.status().is_success() {
            return Err(AcmeError::Broker(format!(
                "{route}: broker refused with HTTP {}",
                resp.status().as_u16()
            )));
        }
        Ok(())
    }

    /// Ask the broker to publish the DNS-01 TXT record.
    pub async fn present(&self, fqdn: &str, value: &str) -> Result<(), AcmeError> {
        self.call("present", fqdn, value).await
    }

    /// Ask the broker to remove it (best-effort; a stale challenge TXT is
    /// harmless, so the caller does not fail issuance if cleanup fails).
    pub async fn cleanup(&self, fqdn: &str, value: &str) -> Result<(), AcmeError> {
        self.call("cleanup", fqdn, value).await
    }
}

// ---------------------------------------------------------------------------
// Pure transforms (unit-tested). No network, no `self`.
// ---------------------------------------------------------------------------

/// Every ACME request carries a User-Agent: RFC 8555 §6.1 says clients SHOULD,
/// and both Pebble and Let's Encrypt REJECT requests without one (400
/// `malformed`, "All requests MUST include a User-Agent header").
const USER_AGENT: &str = concat!(
    "genaryx-relay/",
    env!("CARGO_PKG_VERSION"),
    " (+acme-dns01)"
);

fn b64url(bytes: &[u8]) -> String {
    B64URL.encode(bytes)
}

/// The `(x, y)` base64url coordinates of an ES256 public key, from its
/// SEC1/X9.63 uncompressed point (`0x04 || X || Y`, 65 bytes).
fn jwk_coords(signer: &dyn Es256Signer) -> Result<(String, String), AcmeError> {
    let pt = signer.public_key_x963()?;
    if pt.len() != 65 || pt[0] != 0x04 {
        return Err(AcmeError::Protocol(
            "account public key is not a 65-byte uncompressed P-256 point".into(),
        ));
    }
    Ok((b64url(&pt[1..33]), b64url(&pt[33..65])))
}

/// The JWK object that rides the `newAccount` request's protected header.
fn jwk_json(x: &str, y: &str) -> serde_json::Value {
    serde_json::json!({ "crv": "P-256", "kty": "EC", "x": x, "y": y })
}

/// RFC 7638 JWK thumbprint: SHA-256 over the canonical JWK JSON (required
/// members only, lexicographically ordered `crv,kty,x,y`, no whitespace),
/// base64url. This is the stable account fingerprint that the DNS-01 key
/// authorization binds the token to.
fn jwk_thumbprint(x: &str, y: &str) -> String {
    let canonical = format!(r#"{{"crv":"P-256","kty":"EC","x":"{x}","y":"{y}"}}"#);
    b64url(&Sha256::digest(canonical.as_bytes()))
}

/// The DNS-01 TXT value: base64url(SHA-256(`token.thumbprint`)) (RFC 8555
/// §8.4). This is exactly what the broker publishes and the CA re-derives.
fn dns01_txt_value(token: &str, thumbprint: &str) -> String {
    b64url(&Sha256::digest(format!("{token}.{thumbprint}").as_bytes()))
}

/// Which key material identifies the signer in a JWS protected header:
/// `jwk` for the very first (`newAccount`) request, `kid` (the account URL)
/// for every request after (RFC 8555 §6.2).
#[derive(Clone, Copy)]
enum HeaderKey<'a> {
    Jwk { x: &'a str, y: &'a str },
    Kid(&'a str),
}

/// Build a flattened JWS (RFC 8555 §6.2): ES256 over
/// `base64url(protected) || "." || base64url(payload)`, signature as raw
/// 64-byte `r||s` base64url. An empty `payload` is the POST-as-GET form
/// (§6.3), whose payload is the empty string, not `{}`.
fn signed_body(
    signer: &dyn Es256Signer,
    url: &str,
    nonce: &str,
    payload: &str,
    key: HeaderKey<'_>,
) -> Result<String, AcmeError> {
    let protected = match key {
        HeaderKey::Jwk { x, y } => {
            serde_json::json!({ "alg": "ES256", "nonce": nonce, "url": url, "jwk": jwk_json(x, y) })
        }
        HeaderKey::Kid(kid) => {
            serde_json::json!({ "alg": "ES256", "nonce": nonce, "url": url, "kid": kid })
        }
    };
    let protected_b64 = b64url(&serde_json::to_vec(&protected)?);
    let payload_b64 = if payload.is_empty() {
        String::new()
    } else {
        b64url(payload.as_bytes())
    };
    let signing_input = format!("{protected_b64}.{payload_b64}");
    let sig = signer.sign_raw(signing_input.as_bytes())?;
    Ok(serde_json::to_string(&serde_json::json!({
        "protected": protected_b64,
        "payload": payload_b64,
        "signature": b64url(&sig),
    }))?)
}

/// The DER of a PKCS#10 CSR for `hostname`, signed by the certificate key.
/// This is the ONLY thing the CSR ties the eventual cert to; the CA fills the
/// rest. rcgen puts `hostname` in the SAN, which is what modern clients read.
fn build_csr_der(hostname: &str, cert_key: &rcgen::KeyPair) -> Result<Vec<u8>, AcmeError> {
    let params = rcgen::CertificateParams::new(vec![hostname.to_string()])
        .map_err(|e| AcmeError::Csr(e.to_string()))?;
    let csr = params
        .serialize_request(cert_key)
        .map_err(|e| AcmeError::Csr(e.to_string()))?;
    Ok(csr.der().to_vec())
}

// ---------------------------------------------------------------------------
// ACME wire DTOs.
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Directory {
    #[serde(rename = "newNonce")]
    new_nonce: String,
    #[serde(rename = "newAccount")]
    new_account: String,
    #[serde(rename = "newOrder")]
    new_order: String,
}

#[derive(Deserialize)]
struct OrderBody {
    status: String,
    #[serde(default)]
    authorizations: Vec<String>,
    finalize: String,
    #[serde(default)]
    certificate: Option<String>,
}

#[derive(Deserialize)]
struct AuthzBody {
    status: String,
    #[serde(default)]
    challenges: Vec<Challenge>,
    identifier: Identifier,
}

#[derive(Deserialize)]
struct Challenge {
    #[serde(rename = "type")]
    typ: String,
    url: String,
    #[serde(default)]
    token: String,
}

#[derive(Deserialize)]
struct Identifier {
    value: String,
}

#[derive(Deserialize, Default)]
struct Problem {
    #[serde(rename = "type", default)]
    typ: String,
    #[serde(default)]
    detail: String,
}

/// The parts of an ACME HTTP response the state machine reads.
struct Resp {
    status: u16,
    location: Option<String>,
    replay_nonce: Option<String>,
    body: String,
}

impl Resp {
    async fn read(r: reqwest::Response) -> Result<Self, AcmeError> {
        let status = r.status().as_u16();
        let header = |k: &str| {
            r.headers()
                .get(k)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let location = header("location");
        let replay_nonce = header("replay-nonce");
        let body = r.text().await.map_err(|e| AcmeError::Http(e.to_string()))?;
        Ok(Self {
            status,
            location,
            replay_nonce,
            body,
        })
    }
}

// ---------------------------------------------------------------------------
// The client + the order state machine.
// ---------------------------------------------------------------------------

/// A freshly issued certificate and the private key it terminates against.
/// `cert_pem` is the full chain the CA returned (leaf first); `key_pem` is the
/// certificate key, which never left the relay.
pub struct CertBundle {
    pub cert_pem: String,
    pub key_pem: String,
}

/// An ACME client bound to one CA directory + one broker.
pub struct AcmeClient {
    http: reqwest::Client,
    broker: BrokerClient,
    config: AcmeConfig,
}

impl AcmeClient {
    pub fn new(http: reqwest::Client, broker: BrokerClient, config: AcmeConfig) -> Self {
        Self {
            http,
            broker,
            config,
        }
    }

    /// Run the full DNS-01 issuance for `config.hostname` and return the
    /// certificate + key. The account key authorizes the order; the cert key
    /// backs the CSR. Both stay on this host.
    pub async fn obtain_certificate(
        &self,
        account: &dyn Es256Signer,
        cert_key: &rcgen::KeyPair,
    ) -> Result<CertBundle, AcmeError> {
        let dir: Directory = self.get_json(&self.config.directory_url).await?;
        let (x, y) = jwk_coords(account)?;
        let thumbprint = jwk_thumbprint(&x, &y);
        let jwk_key = HeaderKey::Jwk { x: &x, y: &y };

        let mut nonce = self.fresh_nonce(&dir.new_nonce).await?;

        // newAccount (jwk header). Location is this account's kid. Contact is
        // optional in ACME, so an empty email registers a contactless account
        // rather than sending `mailto:` with nothing after it.
        let acct_payload = if self.config.contact_email.trim().is_empty() {
            serde_json::json!({ "termsOfServiceAgreed": true }).to_string()
        } else {
            let contact = format!("mailto:{}", self.config.contact_email);
            serde_json::json!({ "termsOfServiceAgreed": true, "contact": [contact] }).to_string()
        };
        let acct = self
            .post_signed(
                account,
                &dir.new_account,
                &acct_payload,
                jwk_key,
                &mut nonce,
            )
            .await?;
        let kid = acct.location.clone().ok_or_else(|| {
            AcmeError::Protocol("newAccount returned no Location (account URL)".into())
        })?;
        let kid_key = HeaderKey::Kid(&kid);

        // newOrder for the one hostname. Location is the order URL we poll.
        let order_payload = serde_json::json!({
            "identifiers": [{ "type": "dns", "value": self.config.hostname }]
        })
        .to_string();
        let order_resp = self
            .post_signed(account, &dir.new_order, &order_payload, kid_key, &mut nonce)
            .await?;
        let order_url = order_resp.location.clone().ok_or_else(|| {
            AcmeError::Protocol("newOrder returned no Location (order URL)".into())
        })?;
        let order: OrderBody = serde_json::from_str(&order_resp.body)?;

        // Solve every authorization via DNS-01 through the broker.
        let fqdn = self.config.challenge_fqdn();
        let mut presented: Option<String> = None;
        for authz_url in &order.authorizations {
            let authz_resp = self
                .post_signed(account, authz_url, "", kid_key, &mut nonce)
                .await?;
            let authz: AuthzBody = serde_json::from_str(&authz_resp.body)?;
            if authz.status == "valid" {
                continue; // already authorized (a re-run inside the CA's cache window)
            }
            let challenge = authz
                .challenges
                .iter()
                .find(|c| c.typ == "dns-01")
                .ok_or_else(|| {
                    AcmeError::Protocol(format!(
                        "no dns-01 challenge offered for {}",
                        authz.identifier.value
                    ))
                })?;
            let txt = dns01_txt_value(&challenge.token, &thumbprint);
            self.broker.present(&fqdn, &txt).await?;
            presented = Some(txt);
            // Tell the CA to validate: POST the challenge URL with `{}`.
            self.post_signed(account, &challenge.url, "{}", kid_key, &mut nonce)
                .await?;
            self.poll_authz(account, authz_url, kid_key, &mut nonce)
                .await?;
        }

        // The TXT has done its job; pull it back down (best-effort).
        if let Some(txt) = &presented {
            let _ = self.broker.cleanup(&fqdn, txt).await;
        }

        // Finalize with the CSR, then wait for the order to carry a cert URL.
        let csr = build_csr_der(&self.config.hostname, cert_key)?;
        let finalize_payload = serde_json::json!({ "csr": b64url(&csr) }).to_string();
        self.post_signed(
            account,
            &order.finalize,
            &finalize_payload,
            kid_key,
            &mut nonce,
        )
        .await?;
        let cert_url = self
            .poll_order_for_cert(account, &order_url, kid_key, &mut nonce)
            .await?;

        // Download the chain (POST-as-GET; body is the PEM bundle).
        let cert_resp = self
            .post_signed(account, &cert_url, "", kid_key, &mut nonce)
            .await?;
        if cert_resp.status >= 400 {
            return Err(AcmeError::Protocol(format!(
                "certificate download returned HTTP {}",
                cert_resp.status
            )));
        }
        Ok(CertBundle {
            cert_pem: cert_resp.body,
            key_pem: cert_key.serialize_pem(),
        })
    }

    async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T, AcmeError> {
        let r = self
            .http
            .get(url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .await
            .map_err(|e| AcmeError::Http(e.to_string()))?;
        let text = r.text().await.map_err(|e| AcmeError::Http(e.to_string()))?;
        serde_json::from_str(&text).map_err(AcmeError::from)
    }

    /// A fresh anti-replay nonce from `newNonce` (RFC 8555 §7.2 allows GET).
    async fn fresh_nonce(&self, new_nonce_url: &str) -> Result<String, AcmeError> {
        let r = self
            .http
            .get(new_nonce_url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .await
            .map_err(|e| AcmeError::Http(e.to_string()))?;
        r.headers()
            .get("replay-nonce")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .ok_or_else(|| AcmeError::Protocol("newNonce returned no Replay-Nonce".into()))
    }

    /// POST a signed request, threading the nonce forward from the response.
    /// Retries ONCE on `badNonce` with the server's fresh nonce (RFC 8555
    /// §6.5 says a client SHOULD retry). Any other 4xx/5xx is a structured
    /// [`AcmeError::Server`].
    async fn post_signed(
        &self,
        account: &dyn Es256Signer,
        url: &str,
        payload: &str,
        key: HeaderKey<'_>,
        nonce: &mut String,
    ) -> Result<Resp, AcmeError> {
        for attempt in 0..2 {
            let body = signed_body(account, url, nonce, payload, key)?;
            let http_resp = self
                .http
                .post(url)
                .header(reqwest::header::CONTENT_TYPE, "application/jose+json")
                .header(reqwest::header::USER_AGENT, USER_AGENT)
                .body(body)
                .send()
                .await
                .map_err(|e| AcmeError::Http(e.to_string()))?;
            let resp = Resp::read(http_resp).await?;
            if let Some(n) = &resp.replay_nonce {
                *nonce = n.clone();
            }
            if resp.status >= 400 {
                let problem: Problem = serde_json::from_str(&resp.body).unwrap_or_default();
                let is_bad_nonce = problem.typ.ends_with("badNonce");
                if is_bad_nonce && attempt == 0 && resp.replay_nonce.is_some() {
                    continue;
                }
                let detail = if problem.detail.is_empty() {
                    resp.body.clone()
                } else {
                    problem.detail
                };
                return Err(AcmeError::Server {
                    typ: problem.typ,
                    detail,
                });
            }
            return Ok(resp);
        }
        Err(AcmeError::Protocol(
            "retried on badNonce and still failed".into(),
        ))
    }

    /// Poll an authorization (POST-as-GET) until `valid`; `invalid` fails fast.
    async fn poll_authz(
        &self,
        account: &dyn Es256Signer,
        authz_url: &str,
        key: HeaderKey<'_>,
        nonce: &mut String,
    ) -> Result<(), AcmeError> {
        let attempts = self.max_polls();
        for _ in 0..attempts {
            let resp = self.post_signed(account, authz_url, "", key, nonce).await?;
            let authz: AuthzBody = serde_json::from_str(&resp.body)?;
            match authz.status.as_str() {
                "valid" => return Ok(()),
                "invalid" => {
                    return Err(AcmeError::Protocol(format!(
                        "authorization for {} became invalid",
                        authz.identifier.value
                    )));
                }
                _ => tokio::time::sleep(self.config.poll_interval).await,
            }
        }
        Err(AcmeError::Timeout("authorization".into()))
    }

    /// Poll the order (POST-as-GET) until it is `valid` and exposes a
    /// certificate URL; `invalid` fails fast.
    async fn poll_order_for_cert(
        &self,
        account: &dyn Es256Signer,
        order_url: &str,
        key: HeaderKey<'_>,
        nonce: &mut String,
    ) -> Result<String, AcmeError> {
        let attempts = self.max_polls();
        for _ in 0..attempts {
            let resp = self.post_signed(account, order_url, "", key, nonce).await?;
            let order: OrderBody = serde_json::from_str(&resp.body)?;
            match order.status.as_str() {
                "valid" => {
                    return order.certificate.ok_or_else(|| {
                        AcmeError::Protocol("order is valid but carries no certificate URL".into())
                    });
                }
                "invalid" => {
                    return Err(AcmeError::Protocol("order became invalid".into()));
                }
                _ => tokio::time::sleep(self.config.poll_interval).await,
            }
        }
        Err(AcmeError::Timeout("order".into()))
    }

    fn max_polls(&self) -> u32 {
        let t = self.config.poll_timeout.as_millis();
        let i = self.config.poll_interval.as_millis().max(1);
        ((t / i) as u32).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genaryx_signing::es256::{SoftwareSigner, verify_es256};

    // A deterministic account key so the pinned vectors below are stable.
    fn account() -> SoftwareSigner {
        SoftwareSigner::from_scalar(&[0x11u8; 32]).unwrap()
    }

    #[test]
    fn b64url_is_url_safe_and_unpadded() {
        // 0xFB 0xFF would be "+/" in standard base64 and carry '=' padding;
        // url-safe-no-pad must give "-_" and no '='.
        assert_eq!(b64url(&[0xfb, 0xff]), "-_8");
        assert!(!b64url(&[0u8; 4]).contains('='));
    }

    #[test]
    fn jwk_coords_split_the_uncompressed_point() {
        let (x, y) = jwk_coords(&account()).unwrap();
        // Each coordinate is 32 bytes => 43 base64url chars, no padding.
        assert_eq!(x.len(), 43);
        assert_eq!(y.len(), 43);
        assert_ne!(x, y);
    }

    #[test]
    fn thumbprint_matches_an_independently_built_canonical_jwk() {
        let (x, y) = jwk_coords(&account()).unwrap();
        let thumb = jwk_thumbprint(&x, &y);
        // Rebuild the RFC 7638 canonical form by hand and hash it: this pins
        // both the field ORDER (crv,kty,x,y) and the no-whitespace formatting.
        let canonical = format!("{{\"crv\":\"P-256\",\"kty\":\"EC\",\"x\":\"{x}\",\"y\":\"{y}\"}}");
        let expected = b64url(&Sha256::digest(canonical.as_bytes()));
        assert_eq!(thumb, expected);
        assert_eq!(thumb.len(), 43); // SHA-256 => 32 bytes => 43 b64url chars
    }

    #[test]
    fn dns01_value_is_the_hashed_key_authorization() {
        let thumb = "PLACEHOLDER_THUMB";
        let token = "TOKEN-abc123";
        let got = dns01_txt_value(token, thumb);
        let expected = b64url(&Sha256::digest(b"TOKEN-abc123.PLACEHOLDER_THUMB"));
        assert_eq!(got, expected);
    }

    #[test]
    fn signed_body_is_a_verifiable_es256_jws() {
        let signer = account();
        let (x, y) = jwk_coords(&signer).unwrap();
        let body = signed_body(
            &signer,
            "https://ca.example/acme/new-order",
            "nonce-1",
            "{\"identifiers\":[]}",
            HeaderKey::Jwk { x: &x, y: &y },
        )
        .unwrap();
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        let protected_b64 = json["protected"].as_str().unwrap();
        let payload_b64 = json["payload"].as_str().unwrap();
        let sig_b64 = json["signature"].as_str().unwrap();

        // The signature must verify over exactly `protected.payload`.
        let signing_input = format!("{protected_b64}.{payload_b64}");
        let sig = B64URL.decode(sig_b64).unwrap();
        verify_es256(
            &signer.public_key_x963().unwrap(),
            signing_input.as_bytes(),
            &sig,
        )
        .expect("the JWS signature must verify under the account key");

        // The protected header must carry alg/nonce/url and the jwk.
        let protected: serde_json::Value =
            serde_json::from_slice(&B64URL.decode(protected_b64).unwrap()).unwrap();
        assert_eq!(protected["alg"], "ES256");
        assert_eq!(protected["nonce"], "nonce-1");
        assert_eq!(protected["url"], "https://ca.example/acme/new-order");
        assert_eq!(protected["jwk"]["crv"], "P-256");
        assert!(protected.get("kid").is_none());
    }

    #[test]
    fn post_as_get_has_an_empty_payload_but_kid_not_jwk() {
        let signer = account();
        let body = signed_body(
            &signer,
            "https://ca.example/acme/authz/1",
            "nonce-2",
            "", // POST-as-GET
            HeaderKey::Kid("https://ca.example/acme/acct/7"),
        )
        .unwrap();
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            json["payload"], "",
            "POST-as-GET payload is the empty string"
        );
        let protected: serde_json::Value =
            serde_json::from_slice(&B64URL.decode(json["protected"].as_str().unwrap()).unwrap())
                .unwrap();
        assert_eq!(protected["kid"], "https://ca.example/acme/acct/7");
        assert!(
            protected.get("jwk").is_none(),
            "kid and jwk are mutually exclusive"
        );
    }

    #[test]
    fn csr_carries_the_hostname_and_is_nonempty_der() {
        let key = rcgen::KeyPair::generate().unwrap();
        let der = build_csr_der("relay01.pocket.it-rat.com", &key).unwrap();
        assert!(der.len() > 100, "a real P-256 CSR is a few hundred bytes");
        // The hostname appears in the SAN extension bytes of the DER.
        let needle = b"relay01.pocket.it-rat.com";
        assert!(
            der.windows(needle.len()).any(|w| w == needle),
            "the CSR must request the hostname"
        );
    }

    #[test]
    fn order_and_authz_bodies_deserialize_from_pebble_shaped_json() {
        let order: OrderBody = serde_json::from_str(
            r#"{"status":"pending",
                "authorizations":["https://ca/acme/authz/x"],
                "finalize":"https://ca/acme/order/1/finalize"}"#,
        )
        .unwrap();
        assert_eq!(order.status, "pending");
        assert_eq!(order.authorizations.len(), 1);
        assert!(order.certificate.is_none());

        let authz: AuthzBody = serde_json::from_str(
            r#"{"status":"pending","identifier":{"type":"dns","value":"relay01.pocket.it-rat.com"},
                "challenges":[
                  {"type":"http-01","url":"https://ca/acme/chall/h","token":"t1"},
                  {"type":"dns-01","url":"https://ca/acme/chall/d","token":"t2"}]}"#,
        )
        .unwrap();
        let dns = authz.challenges.iter().find(|c| c.typ == "dns-01").unwrap();
        assert_eq!(dns.token, "t2");
        assert_eq!(authz.identifier.value, "relay01.pocket.it-rat.com");
    }

    #[test]
    fn acme_problem_json_parses_type_and_detail() {
        let p: Problem = serde_json::from_str(
            r#"{"type":"urn:ietf:params:acme:error:badNonce","detail":"JWS has an invalid anti-replay nonce"}"#,
        )
        .unwrap();
        assert!(p.typ.ends_with("badNonce"));
        assert!(p.detail.contains("anti-replay"));
    }

    #[test]
    fn challenge_fqdn_is_prefixed() {
        let cfg = AcmeConfig {
            directory_url: "https://ca/dir".into(),
            hostname: "relay01.pocket.it-rat.com".into(),
            contact_email: "ops@it-rat.com".into(),
            poll_timeout: Duration::from_secs(30),
            poll_interval: Duration::from_secs(2),
        };
        assert_eq!(
            cfg.challenge_fqdn(),
            "_acme-challenge.relay01.pocket.it-rat.com"
        );
    }

    /// End-to-end DNS-01 issuance against a REAL Pebble ACME server + the Pocket
    /// broker, exercising the exact production code path (not a copy). Ignored
    /// by default: it needs the box's Pebble/broker reachable (e.g. over an SSH
    /// tunnel to loopback). Run with:
    ///   RELAY_ACME_DIR=https://127.0.0.1:14000/dir \
    ///   RELAY_ACME_BROKER=http://127.0.0.1:9000 \
    ///   RELAY_ACME_BROKER_USER=proof01 RELAY_ACME_BROKER_TOKEN=proof-relay-token \
    ///   RELAY_ACME_HOST=proof01.pocket.it-rat.com \
    ///   RELAY_ACME_CA=/path/to/pebble-cert.pem \
    ///   cargo test -p genaryx-relay acme:: -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "needs a reachable Pebble ACME server + broker (see doc comment)"]
    async fn obtains_a_real_certificate_from_pebble_via_the_broker() {
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("set {k}"));
        let host = env("RELAY_ACME_HOST");

        let ca_pem = std::fs::read(env("RELAY_ACME_CA")).expect("read Pebble CA pem");
        let ca = reqwest::Certificate::from_pem(&ca_pem).expect("parse Pebble CA");
        let http = reqwest::Client::builder()
            .add_root_certificate(ca)
            .build()
            .expect("build reqwest client trusting Pebble");

        let broker = BrokerClient::new(
            http.clone(),
            env("RELAY_ACME_BROKER"),
            env("RELAY_ACME_BROKER_USER"),
            env("RELAY_ACME_BROKER_TOKEN"),
        );
        let config = AcmeConfig {
            directory_url: env("RELAY_ACME_DIR"),
            hostname: host.clone(),
            contact_email: "relay@it-rat.com".into(),
            poll_timeout: Duration::from_secs(30),
            poll_interval: Duration::from_secs(1),
        };
        let client = AcmeClient::new(http, broker, config);

        // Fresh account key + fresh certificate key, both born on this host.
        let account = SoftwareSigner::generate().expect("account key");
        let cert_key = rcgen::KeyPair::generate().expect("cert key");

        let bundle = client
            .obtain_certificate(&account, &cert_key)
            .await
            .expect("DNS-01 issuance against Pebble must succeed");

        assert!(
            bundle.cert_pem.contains("BEGIN CERTIFICATE"),
            "got a cert chain"
        );
        assert!(
            bundle.key_pem.contains("BEGIN PRIVATE KEY"),
            "kept the cert key"
        );
        // The leaf must actually certify our hostname: decode the first PEM
        // block to DER and find the SAN bytes.
        let leaf_der = first_pem_cert_der(&bundle.cert_pem);
        let needle = host.as_bytes();
        assert!(
            leaf_der.windows(needle.len()).any(|w| w == needle),
            "the issued leaf must certify {host}"
        );
    }

    fn first_pem_cert_der(pem: &str) -> Vec<u8> {
        let body: String = pem
            .lines()
            .skip_while(|l| !l.contains("BEGIN CERTIFICATE"))
            .skip(1)
            .take_while(|l| !l.contains("END CERTIFICATE"))
            .collect();
        base64::engine::general_purpose::STANDARD
            .decode(body.trim())
            .expect("valid base64 in the leaf PEM")
    }
}
