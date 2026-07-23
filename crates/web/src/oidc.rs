//! Offline OIDC / JWT bearer verification for the console's IdP login
//! (docs/CONSOLE-IDP.md, B3/1).
//!
//! A conservative, default-off alternative to the local Argon2id account: the
//! customer hands the box a static JWKS from their IdP and an operator signs
//! in with an OIDC ID-token instead of a password. This mirrors, almost line
//! for line, tokenfuse-cloud's own `crates/cloud/src/oidc.rs` - the same crate
//! (`jsonwebtoken`), the same offline-only posture, the same alg-confusion
//! defense - because that module was already reviewed and is the house
//! pattern for "verify an enterprise JWT without phoning home".
//!
//! * **Offline only.** The JWKS is supplied statically (env var holding the
//!   JWKS JSON, or a path to a file). There is NO network fetch of the
//!   issuer's `.well-known` or its keys. Rotating keys means updating the
//!   env/file. This is what keeps the air-gapped box air-gapped.
//! * **Default off.** [`OidcConfig::from_env`] returns `None` unless issuer,
//!   audience and JWKS are all configured. When `None`, the login route never
//!   calls in here, so a password-only box is byte-for-byte unchanged.
//! * **Local account always wins as break-glass.** OIDC is additive; the
//!   Argon2id owner account is never removed by turning OIDC on.
//! * **Least privilege.** A verified token is a [`Role::Viewer`] unless the
//!   roles claim explicitly names the approver or admin role.
//!
//! ## What is validated (any failure => token rejected)
//!
//! 1. Well-formed JWS with a `kid` header.
//! 2. `kid` matches a key in the configured JWKS.
//! 3. Signature verifies with algorithms derived from the JWK KEY TYPE, never
//!    the attacker-controlled token header (closes RS256->HS256 alg confusion).
//! 4. `exp`, `iss`, `aud` are present (`set_required_spec_claims`) and valid.
//! 5. `sub` is present and non-empty (it names the human for the audit trail).

use crate::roles::Role;
use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use std::collections::HashMap;

const DEFAULT_SUB_CLAIM: &str = "sub";
const DEFAULT_ROLES_CLAIM: &str = "roles";
const DEFAULT_ADMIN_ROLE: &str = "genaryx-admin";
const DEFAULT_APPROVER_ROLE: &str = "genaryx-approver";

/// Static, offline OIDC config, built once at startup and held on the app
/// state. No env or file I/O happens per request.
#[derive(Debug, Clone)]
pub struct OidcConfig {
    issuer: String,
    audience: String,
    jwks: JwkSet,
    sub_claim: String,
    roles_claim: String,
    admin_role: String,
    approver_role: String,
}

/// A verified token: the human's console username, mapped role, and the audit
/// actor id. The raw token never leaves this module (it is a bearer secret).
pub struct Verified {
    /// The `sub` claim: the username shown in the UI and stored on the session.
    pub username: String,
    pub role: Role,
}

impl OidcConfig {
    /// Build from explicit parts, parsing `jwks_json`. `None` (OIDC disabled)
    /// if issuer/audience is empty or the JWKS is missing/empty/unparseable -
    /// a misconfiguration fails SAFE (no token is ever accepted). Exposed so
    /// tests can build a config around an in-test signing key.
    pub fn new(
        issuer: impl Into<String>,
        audience: impl Into<String>,
        jwks_json: &str,
        sub_claim: impl Into<String>,
        roles_claim: impl Into<String>,
        admin_role: impl Into<String>,
        approver_role: impl Into<String>,
    ) -> Option<OidcConfig> {
        let issuer = issuer.into();
        let audience = audience.into();
        if issuer.is_empty() || audience.is_empty() {
            return None;
        }
        let jwks: JwkSet = serde_json::from_str(jwks_json).ok()?;
        if jwks.keys.is_empty() {
            return None;
        }
        Some(OidcConfig {
            issuer,
            audience,
            jwks,
            sub_claim: non_empty(sub_claim.into(), DEFAULT_SUB_CLAIM),
            roles_claim: non_empty(roles_claim.into(), DEFAULT_ROLES_CLAIM),
            admin_role: non_empty(admin_role.into(), DEFAULT_ADMIN_ROLE),
            approver_role: non_empty(approver_role.into(), DEFAULT_APPROVER_ROLE),
        })
    }

    /// Build from the environment, or `None` when OIDC is unconfigured.
    ///
    /// Required (all three) - absent => OIDC disabled:
    /// `GENARYX_WEB_OIDC_ISSUER`, `GENARYX_WEB_OIDC_AUDIENCE`,
    /// `GENARYX_WEB_OIDC_JWKS` (inline JSON, or a path to a file holding it).
    /// Optional: `GENARYX_WEB_OIDC_SUB_CLAIM` (default `sub`),
    /// `GENARYX_WEB_OIDC_ROLES_CLAIM` (default `roles`),
    /// `GENARYX_WEB_OIDC_ADMIN_ROLE` (default `genaryx-admin`),
    /// `GENARYX_WEB_OIDC_APPROVER_ROLE` (default `genaryx-approver`).
    pub fn from_env() -> Option<OidcConfig> {
        let issuer = env_nonempty("GENARYX_WEB_OIDC_ISSUER")?;
        let audience = env_nonempty("GENARYX_WEB_OIDC_AUDIENCE")?;
        let jwks_raw = env_nonempty("GENARYX_WEB_OIDC_JWKS")?;
        let jwks_json = load_jwks(&jwks_raw)?;
        OidcConfig::new(
            issuer,
            audience,
            &jwks_json,
            std::env::var("GENARYX_WEB_OIDC_SUB_CLAIM").unwrap_or_default(),
            std::env::var("GENARYX_WEB_OIDC_ROLES_CLAIM").unwrap_or_default(),
            std::env::var("GENARYX_WEB_OIDC_ADMIN_ROLE").unwrap_or_default(),
            std::env::var("GENARYX_WEB_OIDC_APPROVER_ROLE").unwrap_or_default(),
        )
    }
}

#[derive(Deserialize)]
struct Claims {
    #[serde(flatten)]
    extra: HashMap<String, serde_json::Value>,
}

impl Claims {
    fn string(&self, key: &str) -> Option<String> {
        match self.extra.get(key)? {
            serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
            _ => None,
        }
    }

    /// Whether the roles claim contains `role`. Accepts a JSON array of
    /// strings or a single space-separated string - the two shapes IdPs emit.
    fn has_role(&self, key: &str, role: &str) -> bool {
        match self.extra.get(key) {
            Some(serde_json::Value::Array(items)) => items.iter().any(|v| v.as_str() == Some(role)),
            Some(serde_json::Value::String(s)) => s.split_whitespace().any(|r| r == role),
            _ => false,
        }
    }
}

/// Verify an OIDC ID-token and map it to a username + role, or `None` on any
/// validation failure. See the module docs for the exact checks. Role is
/// `admin` if the roles claim contains the admin role, else `approver` if it
/// contains the approver role, else `viewer` (least privilege).
pub fn verify(cfg: &OidcConfig, token: &str) -> Option<Verified> {
    // 1. Well-formed header with a key id.
    let header = decode_header(token).ok()?;
    let kid = header.kid?;

    // 2. Key id matches a configured JWKS key.
    let jwk = cfg.jwks.find(&kid)?;

    // 3. Allowed algorithms come from the key TYPE, never the token header -
    //    prevents an attacker downgrading an RSA/EC key to HS256 and forging a
    //    signature ("alg confusion"). Symmetric / OKP keys are rejected.
    let algorithms: Vec<Algorithm> = match &jwk.algorithm {
        AlgorithmParameters::RSA(_) => vec![Algorithm::RS256, Algorithm::RS384, Algorithm::RS512],
        AlgorithmParameters::EllipticCurve(_) => vec![Algorithm::ES256, Algorithm::ES384],
        _ => return None,
    };
    let key = DecodingKey::from_jwk(jwk).ok()?;

    // 4. Signature + exp + iss + aud. `set_required_spec_claims` makes a token
    //    that OMITS exp/iss/aud a rejection, not a pass (jsonwebtoken only
    //    checks iss/aud when present by default - an audience-confusion risk
    //    if an IdP reuses a signing key across services).
    let mut validation = Validation::new(algorithms[0]);
    validation.algorithms = algorithms;
    validation.validate_exp = true;
    validation.set_required_spec_claims(&["exp", "iss", "aud"]);
    validation.set_issuer(&[&cfg.issuer]);
    validation.set_audience(&[&cfg.audience]);
    let data = decode::<Claims>(token, &key, &validation).ok()?;
    let claims = data.claims;

    // 5. `sub` is mandatory - it names the human for the audit trail.
    let username = claims.string(&cfg.sub_claim)?;

    let role = if claims.has_role(&cfg.roles_claim, &cfg.admin_role) {
        Role::Admin
    } else if claims.has_role(&cfg.roles_claim, &cfg.approver_role) {
        Role::Approver
    } else {
        Role::Viewer
    };

    Some(Verified { username, role })
}

fn non_empty(value: String, default: &str) -> String {
    if value.trim().is_empty() {
        default.to_string()
    } else {
        value
    }
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

/// `raw` is either the JWKS JSON itself or a path to a file holding it. A
/// value that starts with `{` is treated as inline JSON; anything else is
/// read as a file path. Never a URL: nothing is fetched.
fn load_jwks(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') {
        Some(trimmed.to_string())
    } else {
        std::fs::read_to_string(trimmed).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};

    // A fixed ES256 test keypair (PKCS#8 PEM private + the matching public JWK
    // with kid "test-key"), generated once with openssl for tests; never a
    // real key. The JWK x/y are the base64url P-256 coords of PRIV_PEM.
    const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgSF30BcU02a19uA3P\n1YcMrDkSfQWiKtjy4jLGGykVcHOhRANCAASCUZm+yqkv9wBNDdveC0nYLscslVCb\nFPNbeKd6A+DgxTZdKiFLdC1NbkWHNPq8FzEyh/aiC356Mqz7iF1L42Ve\n-----END PRIVATE KEY-----\n";
    // Matching public key as a JWK set (x/y are the base64url EC coords).
    const JWKS: &str = r#"{"keys":[{"kty":"EC","crv":"P-256","kid":"test-key","x":"glGZvsqpL_cATQ3b3gtJ2C7HLJVQmxTzW3inegPg4MU","y":"Nl0qIUt0LU1uRYc0-rwXMTKH9qILfnoyrPuIXUvjZV4"}]}"#;

    fn cfg() -> OidcConfig {
        OidcConfig::new(
            "https://idp.example",
            "genaryx-console",
            JWKS,
            "",
            "",
            "",
            "",
        )
        .expect("valid config")
    }

    fn token_with(claims: serde_json::Value) -> String {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("test-key".to_string());
        let key = EncodingKey::from_ec_pem(PRIV_PEM.as_bytes()).expect("valid key");
        encode(&header, &claims, &key).expect("encode")
    }

    fn future() -> i64 {
        // A fixed far-future exp so the test never depends on the clock beyond
        // "not expired". jsonwebtoken compares against the real now(), so this
        // must be an absolute timestamp well ahead.
        4_102_444_800 // 2100-01-01
    }

    #[test]
    fn a_valid_admin_token_verifies_and_maps_admin() {
        let tok = token_with(serde_json::json!({
            "iss": "https://idp.example",
            "aud": "genaryx-console",
            "sub": "alice",
            "roles": ["genaryx-admin", "something-else"],
            "exp": future(),
        }));
        let v = verify(&cfg(), &tok).expect("verifies");
        assert_eq!(v.username, "alice");
        assert_eq!(v.role, Role::Admin);
    }

    #[test]
    fn approver_and_viewer_roles_map_least_privilege() {
        let approver = token_with(serde_json::json!({
            "iss": "https://idp.example", "aud": "genaryx-console",
            "sub": "bob", "roles": "genaryx-approver", "exp": future(),
        }));
        assert_eq!(verify(&cfg(), &approver).unwrap().role, Role::Approver);

        // No known role => viewer, never a default promotion.
        let viewer = token_with(serde_json::json!({
            "iss": "https://idp.example", "aud": "genaryx-console",
            "sub": "carol", "roles": ["unrelated"], "exp": future(),
        }));
        assert_eq!(verify(&cfg(), &viewer).unwrap().role, Role::Viewer);
    }

    #[test]
    fn wrong_issuer_audience_or_missing_sub_are_rejected() {
        for bad in [
            serde_json::json!({"iss":"https://evil","aud":"genaryx-console","sub":"x","exp":future()}),
            serde_json::json!({"iss":"https://idp.example","aud":"other","sub":"x","exp":future()}),
            serde_json::json!({"iss":"https://idp.example","aud":"genaryx-console","exp":future()}),
            // Expired.
            serde_json::json!({"iss":"https://idp.example","aud":"genaryx-console","sub":"x","exp":1}),
        ] {
            assert!(verify(&cfg(), &token_with(bad)).is_none());
        }
    }

    #[test]
    fn a_token_signed_by_an_unknown_key_is_rejected() {
        // Re-sign with a different kid the JWKS does not contain.
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("not-in-jwks".to_string());
        let key = EncodingKey::from_ec_pem(PRIV_PEM.as_bytes()).unwrap();
        let tok = encode(
            &header,
            &serde_json::json!({
                "iss":"https://idp.example","aud":"genaryx-console","sub":"x","exp":future()
            }),
            &key,
        )
        .unwrap();
        assert!(verify(&cfg(), &tok).is_none());
    }

    #[test]
    fn disabled_config_when_parts_missing() {
        assert!(OidcConfig::new("", "aud", JWKS, "", "", "", "").is_none());
        assert!(OidcConfig::new("iss", "aud", "{}", "", "", "", "").is_none());
        assert!(OidcConfig::new("iss", "aud", "not json", "", "", "", "").is_none());
    }
}
