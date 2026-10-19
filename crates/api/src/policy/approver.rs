//! The approver key and the statement a console signs for a person's decision
//! (CLAUDE.md invariant 35).
//!
//! When the operator gives the console a key file (`GENARYX_APPROVER_KEY_FILE`),
//! an approval is decided only with a statement the console signs after the
//! person's passkey assertion has verified. The statement names the person,
//! the credential, the approval and the decision, and wardryx verifies it
//! against its own copy of the console's public key set. With no key file
//! configured, no statement is sent and the approval is recorded as decided by
//! a key, which wardryx may refuse.
//!
//! The decision is refused, and nothing is sent to wardryx, when a key file is
//! configured and a statement cannot be made: no passkey confirmed the
//! decision, the key file could not be read, or the statement's claims or
//! signature failed.

use genaryx_signing::Es256Signer;
use std::fmt;
use std::path::Path;

/// The console's approver key, read once at startup.
pub enum ApproverKey {
    /// No key file is configured: no statement is made.
    NotConfigured,
    /// The key file was read and holds a usable P-256 key.
    Ready(Box<dyn Es256Signer>),
    /// A key file is configured but could not be used. The reason names the
    /// file and what was wrong with it, never key material.
    Failed(String),
}

impl ApproverKey {
    /// Read the key file named by `path`, or report that none is configured.
    ///
    /// Red-first scaffold: reports `NotConfigured` until the fix lands.
    pub fn load(_path: Option<&Path>) -> Self {
        ApproverKey::NotConfigured
    }
}

/// How long a statement is valid after it is made. The statement is sent at
/// once, so this only has to cover clock skew between the console and wardryx.
pub const STATEMENT_LIFETIME_SECS: i64 = 120;

/// What a statement is made for.
pub struct Request<'a> {
    /// The approval id, which the statement's `aid` must equal.
    pub approval_id: &'a str,
    /// `grant` or `deny`, which the statement's `dec` must equal.
    pub decision: &'a str,
    /// The stable subject of the person deciding, the statement's `sub`.
    pub approver: &'a str,
    /// The credential id of the passkey whose assertion verified this
    /// decision, or `None` when no passkey confirmed it.
    pub passkey_credential: Option<&'a str>,
}

/// Why a decision was refused before anything was sent to wardryx.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A key file is configured, but no passkey assertion confirmed the decision.
    NoPasskeyConfirmed,
    /// A key file is configured, but it could not be used.
    KeyUnusable(String),
    /// The statement could not be made: its claims or its signing failed.
    Statement(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::NoPasskeyConfirmed => write!(
                f,
                "a key file is configured for approver statements, and no passkey confirmed this decision"
            ),
            Refusal::KeyUnusable(reason) => {
                write!(f, "the approver key file cannot be used: {reason}")
            }
            Refusal::Statement(reason) => {
                write!(f, "the approver statement could not be made: {reason}")
            }
        }
    }
}

/// The statement to send with a decision, or `None` when no key file is
/// configured. A configured key file that cannot make a statement refuses the
/// decision instead.
///
/// Red-first scaffold: makes no statement until the fix lands.
pub fn statement_for(
    _key: &ApproverKey,
    _request: &Request<'_>,
    _now: i64,
) -> Result<Option<String>, Refusal> {
    Ok(None)
}

/// Read a key file and return the public JWKS of its key, the file wardryx
/// reads as its approver key set (`WARDRYX_APPROVER_JWKS`).
///
/// Red-first scaffold: refuses every file until the fix lands.
pub fn jwks_for_key_file(_path: &Path) -> Result<String, String> {
    Err("red-first scaffold: not implemented".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use genaryx_signing::SoftwareSigner;
    use genaryx_signing::approver as statement;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch_file(contents: &str) -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "genaryx-approver-{}-{}.pem",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, contents).expect("write a scratch key file");
        path
    }

    /// A console key on scalar 7, with its public half for verification.
    fn ready() -> (ApproverKey, Vec<u8>) {
        let signer = SoftwareSigner::from_scalar(&[7u8; 32]).unwrap();
        let public = signer.public_key_x963().unwrap();
        (ApproverKey::Ready(Box::new(signer)), public)
    }

    fn request<'a>(passkey: Option<&'a str>) -> Request<'a> {
        Request {
            approval_id: "appr-7",
            decision: "grant",
            approver: "user://acme/alice",
            passkey_credential: passkey,
        }
    }

    #[test]
    fn a_confirmed_grant_is_signed_for_this_approval_and_decision() {
        let (key, public) = ready();
        let jws = statement_for(&key, &request(Some("Y3JlZC0x")), 1_000)
            .unwrap()
            .expect("a confirmed decision with a key file gets a statement");
        let claims = statement::verify(&public, &jws, 1_010).expect("the statement verifies");
        assert_eq!(claims.aid, "appr-7");
        assert_eq!(claims.dec, "grant");
        assert_eq!(claims.sub, "user://acme/alice");
        assert_eq!(claims.cred, "Y3JlZC0x");
    }

    #[test]
    fn a_confirmed_deny_is_signed_as_deny() {
        let (key, public) = ready();
        let mut deny = request(Some("Y3JlZC0x"));
        deny.decision = "deny";
        let jws = statement_for(&key, &deny, 1_000)
            .unwrap()
            .expect("a statement");
        assert_eq!(statement::verify(&public, &jws, 1_010).unwrap().dec, "deny");
    }

    #[test]
    fn with_a_key_file_a_decision_no_passkey_confirmed_is_refused() {
        let (key, _) = ready();
        assert_eq!(
            statement_for(&key, &request(None), 1_000),
            Err(Refusal::NoPasskeyConfirmed)
        );
    }

    #[test]
    fn an_unusable_key_file_refuses_the_decision_and_says_why() {
        let key = ApproverKey::Failed("the file is empty".to_string());
        assert!(matches!(
            statement_for(&key, &request(Some("Y3JlZC0x")), 1_000),
            Err(Refusal::KeyUnusable(_))
        ));
    }

    #[test]
    fn a_statement_the_claims_refuse_is_refused_not_sent() {
        let (key, _) = ready();
        let mut empty_approver = request(Some("Y3JlZC0x"));
        empty_approver.approver = "";
        assert!(matches!(
            statement_for(&key, &empty_approver, 1_000),
            Err(Refusal::Statement(_))
        ));
    }

    #[test]
    fn with_no_key_file_no_statement_is_made_and_nothing_is_refused() {
        let key = ApproverKey::NotConfigured;
        assert_eq!(
            statement_for(&key, &request(Some("Y3JlZC0x")), 1_000),
            Ok(None)
        );
        assert_eq!(statement_for(&key, &request(None), 1_000), Ok(None));
    }

    #[test]
    fn loading_without_a_path_is_not_configured() {
        assert!(matches!(
            ApproverKey::load(None),
            ApproverKey::NotConfigured
        ));
    }

    #[test]
    fn loading_a_pkcs8_key_file_is_ready() {
        let pem = statement::pkcs8_pem(&[7u8; 32], None);
        let path = scratch_file(&pem);
        let loaded = ApproverKey::load(Some(path.as_path()));
        let _ = std::fs::remove_file(&path);
        assert!(matches!(loaded, ApproverKey::Ready(_)));
    }

    #[test]
    fn loading_a_missing_or_malformed_key_file_is_failed_not_silent() {
        let missing = std::env::temp_dir().join("genaryx-approver-does-not-exist.pem");
        assert!(matches!(
            ApproverKey::load(Some(missing.as_path())),
            ApproverKey::Failed(_)
        ));
        let path = scratch_file("not a key");
        let loaded = ApproverKey::load(Some(path.as_path()));
        let _ = std::fs::remove_file(&path);
        assert!(matches!(loaded, ApproverKey::Failed(_)));
    }

    #[test]
    fn the_jwks_of_a_key_file_names_the_key_the_statements_use() {
        let public = SoftwareSigner::from_scalar(&[7u8; 32])
            .unwrap()
            .public_key_x963()
            .unwrap();
        let path = scratch_file(&statement::pkcs8_pem(&[7u8; 32], None));
        let json = jwks_for_key_file(&path);
        let _ = std::fs::remove_file(&path);
        let doc: serde_json::Value = serde_json::from_str(&json.expect("a JWKS")).unwrap();
        assert_eq!(
            doc["keys"][0]["kid"],
            statement::thumbprint(&public).unwrap()
        );
        assert_eq!(doc["keys"][0]["alg"], "ES256");
    }

    #[test]
    fn the_jwks_refuses_a_missing_key_file() {
        let missing = std::env::temp_dir().join("genaryx-approver-does-not-exist.pem");
        assert!(jwks_for_key_file(&missing).is_err());
    }
}
