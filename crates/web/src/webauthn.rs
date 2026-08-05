//! WebAuthn per-action ceremony (D15 B3 part 2, docs/CONSOLE-IDP.md).
//!
//! A signed-in session gets you the console; it does not get you the kill.
//! The five privileged commands (kill, break-glass carriers, budget mutation,
//! policy write, approval grant) additionally require a fresh, per-action
//! WebAuthn assertion: the operator's passkey (Touch ID, Windows Hello, a
//! roaming key) signs a challenge this server minted FOR THAT ONE COMMAND,
//! and the assertion's algorithm + credential id are recorded into the same
//! `CommandRecord` the action journals. This is the web console's twin of the
//! removed desktop shell's Secure-Enclave signed kill, and it reuses the same
//! verification primitive (`genaryx_signing::verify_es256`, the p256 path
//! device-pairing already trusts).
//!
//! Deliberately NOT `webauthn-rs`: that crate hard-depends on OpenSSL
//! (via `webauthn-attestation-ca`), a second crypto backend this pure-Rust
//! workspace does not want. The scope that keeps hand-parsing honest:
//! **ES256 only** (`-7`, the one algorithm every passkey provider ships) and
//! **attestation "none"** (we bind actions to a key the operator enrolled in
//! an authenticated session; WHICH vendor made the authenticator is an
//! enterprise policy question deferred until a pilot asks). Everything here
//! is fail-closed: any parse or verify failure is a refusal, never a pass.
//!
//! Deployment note (browser rules, not ours): `navigator.credentials` exists
//! only in a secure context, so the ceremony works when the console is
//! reached as `localhost` (the default loopback bind, or an `ssh -L` forward
//! over the operator's tunnel) or behind TLS. Reaching it as a bare
//! `http://10.x.x.x` renders the ceremony unavailable; the UI says so
//! honestly rather than silently downgrading.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a minted challenge stays redeemable. Long enough for a human to
/// touch the sensor, short enough that a leaked challenge is stale by the
/// time anyone could replay it (and it is one-shot regardless).
const CEREMONY_TTL: Duration = Duration::from_secs(120);

/// WebAuthn authenticator-data flag bits (WebAuthn L2 §6.1).
const FLAG_UP: u8 = 0x01; // user present
const FLAG_UV: u8 = 0x04; // user verified (biometric/PIN)
const FLAG_AT: u8 = 0x40; // attested credential data included

// ---------------------------------------------------------------------------
// errors
// ---------------------------------------------------------------------------

/// A ceremony failure. Every variant means "refuse the action"; none is ever
/// "inconclusive, allow" (same F-03 rule as `genaryx_signing`).
#[derive(Debug, PartialEq, Eq)]
pub enum WebAuthnError {
    /// Malformed base64 / JSON / CBOR anywhere in the envelope.
    Malformed(&'static str),
    /// The envelope parsed but a bound value did not match (origin, rp id,
    /// challenge, ceremony type, command binding).
    Mismatch(&'static str),
    /// The authenticator did not assert user presence.
    UserNotPresent,
    /// Attestation format other than "none" (out of scope by design).
    UnsupportedAttestation,
    /// COSE key is not the ES256 / P-256 shape this module supports.
    UnsupportedKey(&'static str),
    /// The ECDSA signature did not verify against the enrolled key.
    BadSignature,
    /// Signature counter regressed: possible cloned authenticator.
    CloneSuspected,
    /// No pending ceremony for this challenge (expired, replayed, or never
    /// minted). The caller must not distinguish which.
    UnknownChallenge,
    /// The session user does not own the ceremony or the credential.
    WrongUser,
    /// No enrolled passkey with that credential id (removal of something the
    /// caller does not have).
    UnknownCredential,
    /// This removal would leave the user with no passkey at all, and the
    /// caller did not carry the authority for that (see `main.rs`'s removal
    /// policy: the last one goes only to the operator password).
    LastPasskey,
    /// Reading/writing the passkey store failed.
    Store(String),
}

impl std::fmt::Display for WebAuthnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebAuthnError::Malformed(what) => write!(f, "malformed {what}"),
            WebAuthnError::Mismatch(what) => write!(f, "{what} mismatch"),
            WebAuthnError::UserNotPresent => write!(f, "user presence not asserted"),
            WebAuthnError::UnsupportedAttestation => {
                write!(f, "unsupported attestation format (only \"none\")")
            }
            WebAuthnError::UnsupportedKey(what) => write!(f, "unsupported key: {what}"),
            WebAuthnError::BadSignature => write!(f, "signature verification failed"),
            WebAuthnError::CloneSuspected => {
                write!(
                    f,
                    "signature counter regressed (possible cloned authenticator)"
                )
            }
            WebAuthnError::UnknownChallenge => write!(f, "unknown or expired challenge"),
            WebAuthnError::WrongUser => write!(f, "ceremony does not belong to this user"),
            WebAuthnError::UnknownCredential => write!(f, "no passkey with that credential id"),
            WebAuthnError::LastPasskey => write!(
                f,
                "this is the last enrolled passkey; removing it needs the operator password"
            ),
            WebAuthnError::Store(e) => write!(f, "passkey store: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// relying-party configuration
// ---------------------------------------------------------------------------

/// The relying-party identity the browser binds credentials to.
///
/// `rp_id` must be the DOMAIN the operator's browser addresses (WebAuthn
/// scopes credentials to it); `origin` is the exact scheme://host[:port] the
/// browser reports in `clientDataJSON.origin`. Defaults fit the documented
/// loopback/`ssh -L` deployment; a TLS-fronted console overrides both.
#[derive(Debug, Clone)]
pub struct RpConfig {
    pub rp_id: String,
    pub origin: String,
}

impl RpConfig {
    /// Resolve from `GENARYX_WEB_RP_ID` / `GENARYX_WEB_ORIGIN`, defaulting to
    /// the loopback deployment (`localhost`, `http://localhost:<port>`).
    pub fn from_env(bind_port: u16) -> Self {
        let rp_id = std::env::var("GENARYX_WEB_RP_ID").unwrap_or_else(|_| "localhost".to_string());
        let origin = std::env::var("GENARYX_WEB_ORIGIN")
            .unwrap_or_else(|_| format!("http://localhost:{bind_port}"));
        Self { rp_id, origin }
    }
}

// ---------------------------------------------------------------------------
// passkey store
// ---------------------------------------------------------------------------

/// One enrolled passkey. The public half only; the private key never leaves
/// the operator's authenticator.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PasskeyRecord {
    /// The credential id, base64url (as the browser reports it). This is what
    /// `CommandRecord.sig_fpr` carries, so an auditor can say WHICH enrolled
    /// key confirmed an action.
    pub credential_id: String,
    /// SEC1/X9.63 uncompressed P-256 point, standard base64 - the exact wire
    /// form `genaryx_signing::verify_es256_b64` takes.
    pub public_key_x963: String,
    /// Last accepted signature counter (0 = authenticator does not count;
    /// Apple passkeys report 0 forever, which is allowed).
    pub sign_count: u32,
    /// RFC 3339, informational.
    pub created_at: String,
    /// Operator-chosen label ("MacBook Touch ID"), informational.
    pub label: String,
}

/// The on-disk store: user -> enrolled passkeys. A plain owner-only JSON file
/// beside `operator.json`, same posture: worth protecting, no reason for a
/// database. We store AUTHENTICATORS, not people - the customer's IdP remains
/// the identity registry (docs/CONSOLE-IDP.md).
pub struct PasskeyStore {
    path: PathBuf,
    inner: Mutex<HashMap<String, Vec<PasskeyRecord>>>,
}

impl PasskeyStore {
    /// Open (or start empty at) `path`. A missing file is an empty store; an
    /// unreadable/corrupt file is an ERROR at first use, never silently empty
    /// (an attacker deleting enrollments must not un-gate actions - callers
    /// treat `Store` errors as refusal).
    pub fn open(path: PathBuf) -> Result<Self, WebAuthnError> {
        let inner = match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw)
                .map_err(|e| WebAuthnError::Store(format!("corrupt {}: {e}", path.display())))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => {
                return Err(WebAuthnError::Store(format!(
                    "cannot read {}: {e}",
                    path.display()
                )));
            }
        };
        Ok(Self {
            path,
            inner: Mutex::new(inner),
        })
    }

    /// Every enrolled passkey for `user` (empty when none).
    pub fn for_user(&self, user: &str) -> Vec<PasskeyRecord> {
        self.inner
            .lock()
            .expect("passkey store poisoned")
            .get(user)
            .cloned()
            .unwrap_or_default()
    }

    /// Whether `user` has at least one enrolled passkey (drives the gate:
    /// enrolled means the ceremony is REQUIRED for privileged commands).
    pub fn has_any(&self, user: &str) -> bool {
        !self.for_user(user).is_empty()
    }

    /// Enroll a new passkey for `user` and persist. Refuses a duplicate
    /// credential id (one credential belongs to one enrollment).
    pub fn add(&self, user: &str, record: PasskeyRecord) -> Result<(), WebAuthnError> {
        let mut guard = self.inner.lock().expect("passkey store poisoned");
        let list = guard.entry(user.to_string()).or_default();
        if list.iter().any(|r| r.credential_id == record.credential_id) {
            return Err(WebAuthnError::Store("credential already enrolled".into()));
        }
        list.push(record);
        self.persist(&guard)
    }

    /// Remove one enrolled passkey and persist, returning how many the user
    /// has left.
    ///
    /// `allow_last` is the caller's answer to "may this empty the set?", and
    /// it is checked HERE rather than only at the route because the count and
    /// the removal have to be one decision: two removals arriving together
    /// would otherwise each see another key remaining and, between them,
    /// leave none. Refuses [`WebAuthnError::UnknownCredential`] for a
    /// credential this user never enrolled, so a removal is never reported as
    /// done when nothing was removed.
    pub fn remove(
        &self,
        user: &str,
        credential_id: &str,
        allow_last: bool,
    ) -> Result<usize, WebAuthnError> {
        let mut guard = self.inner.lock().expect("passkey store poisoned");
        let list = guard
            .get_mut(user)
            .ok_or(WebAuthnError::UnknownCredential)?;
        let at = list
            .iter()
            .position(|r| r.credential_id == credential_id)
            .ok_or(WebAuthnError::UnknownCredential)?;
        if list.len() == 1 && !allow_last {
            return Err(WebAuthnError::LastPasskey);
        }
        list.remove(at);
        let remaining = list.len();
        self.persist(&guard)?;
        Ok(remaining)
    }

    /// Update the stored signature counter after an accepted assertion.
    pub fn update_sign_count(
        &self,
        user: &str,
        credential_id: &str,
        sign_count: u32,
    ) -> Result<(), WebAuthnError> {
        let mut guard = self.inner.lock().expect("passkey store poisoned");
        if let Some(rec) = guard
            .get_mut(user)
            .and_then(|l| l.iter_mut().find(|r| r.credential_id == credential_id))
        {
            rec.sign_count = sign_count;
        }
        self.persist(&guard)
    }

    fn persist(&self, map: &HashMap<String, Vec<PasskeyRecord>>) -> Result<(), WebAuthnError> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| {
                WebAuthnError::Store(format!("cannot create {}: {e}", dir.display()))
            })?;
        }
        let body =
            serde_json::to_string_pretty(map).map_err(|e| WebAuthnError::Store(e.to_string()))?;
        std::fs::write(&self.path, body).map_err(|e| {
            WebAuthnError::Store(format!("cannot write {}: {e}", self.path.display()))
        })?;
        restrict(&self.path);
        Ok(())
    }
}

#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict(_path: &Path) {}

// ---------------------------------------------------------------------------
// pending ceremonies (one-shot challenges)
// ---------------------------------------------------------------------------

/// What a minted challenge is FOR. An action challenge is bound to the exact
/// command name + args digest, so an assertion for "kill run A" can never be
/// replayed to authorize "kill run B" - the same binding idea as the desktop
/// canonical string (`genaryx_signing::canonical_request`), carried by the
/// server-side pending entry instead of the signed bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purpose {
    Register,
    Action {
        command: String,
        /// Lowercase-hex SHA-256 of the exact args JSON the command will
        /// dispatch with (`genaryx_signing::body_sha256_hex`).
        args_sha256: String,
    },
}

struct Pending {
    user: String,
    purpose: Purpose,
    expires: Instant,
}

/// In-memory one-shot challenge table. Restart drops it (matching the session
/// table's posture: restarting the console cuts access, re-minting is cheap).
#[derive(Default)]
pub struct PendingCeremonies {
    inner: Mutex<HashMap<String, Pending>>,
}

impl PendingCeremonies {
    /// Mint a fresh challenge for `user`/`purpose`. Returns the challenge as
    /// base64url of 32 random bytes - the exact string the browser will echo
    /// in `clientDataJSON.challenge`.
    pub fn mint(&self, user: &str, purpose: Purpose) -> Result<String, WebAuthnError> {
        let hex = genaryx_signing::es256::random_hex(32)
            .map_err(|e| WebAuthnError::Store(format!("entropy: {e}")))?;
        let raw = hex_to_bytes(&hex).expect("random_hex emits valid hex");
        let challenge = B64URL.encode(raw);
        let mut guard = self.inner.lock().expect("pending ceremonies poisoned");
        guard.retain(|_, p| p.expires > Instant::now());
        guard.insert(
            challenge.clone(),
            Pending {
                user: user.to_string(),
                purpose,
                expires: Instant::now() + CEREMONY_TTL,
            },
        );
        Ok(challenge)
    }

    /// Redeem a challenge: exists, not expired, owned by `user` - and GONE
    /// afterwards either way (one-shot). Returns its purpose for the caller
    /// to check the action binding.
    pub fn take(&self, user: &str, challenge: &str) -> Result<Purpose, WebAuthnError> {
        let mut guard = self.inner.lock().expect("pending ceremonies poisoned");
        let pending = guard
            .remove(challenge)
            .ok_or(WebAuthnError::UnknownChallenge)?;
        if pending.expires <= Instant::now() {
            return Err(WebAuthnError::UnknownChallenge);
        }
        if pending.user != user {
            return Err(WebAuthnError::WrongUser);
        }
        Ok(pending.purpose)
    }
}

fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

// ---------------------------------------------------------------------------
// envelope parsing (clientDataJSON / attestationObject / authenticatorData)
// ---------------------------------------------------------------------------

/// The three fields of `clientDataJSON` this ceremony binds. Unknown fields
/// are ignored by design (the browser adds more).
#[derive(Debug, Deserialize)]
struct ClientData {
    #[serde(rename = "type")]
    kind: String,
    challenge: String,
    origin: String,
}

fn parse_client_data(client_data_json: &[u8]) -> Result<ClientData, WebAuthnError> {
    serde_json::from_slice(client_data_json).map_err(|_| WebAuthnError::Malformed("clientDataJSON"))
}

/// Parsed authenticatorData (WebAuthn L2 §6.1).
#[derive(Debug)]
struct AuthData {
    rp_id_hash: [u8; 32],
    flags: u8,
    sign_count: u32,
    /// Present only when FLAG_AT is set (registration).
    credential: Option<AttestedCredential>,
}

#[derive(Debug)]
struct AttestedCredential {
    credential_id: Vec<u8>,
    /// SEC1/X9.63 uncompressed point extracted from the COSE key.
    public_key_x963: [u8; 65],
}

fn parse_authenticator_data(bytes: &[u8]) -> Result<AuthData, WebAuthnError> {
    if bytes.len() < 37 {
        return Err(WebAuthnError::Malformed("authenticatorData (too short)"));
    }
    let mut rp_id_hash = [0u8; 32];
    rp_id_hash.copy_from_slice(&bytes[0..32]);
    let flags = bytes[32];
    let sign_count = u32::from_be_bytes([bytes[33], bytes[34], bytes[35], bytes[36]]);

    let credential = if flags & FLAG_AT != 0 {
        // aaguid(16) || credIdLen(2 BE) || credId || COSE key (one CBOR value)
        if bytes.len() < 37 + 18 {
            return Err(WebAuthnError::Malformed(
                "attestedCredentialData (too short)",
            ));
        }
        let cred_len = u16::from_be_bytes([bytes[53], bytes[54]]) as usize;
        let cred_start: usize = 55;
        let cred_end = cred_start
            .checked_add(cred_len)
            .filter(|&e| e <= bytes.len())
            .ok_or(WebAuthnError::Malformed("credentialId length"))?;
        let credential_id = bytes[cred_start..cred_end].to_vec();
        // The COSE key is the next single CBOR value; any bytes after it are
        // extensions, which this ceremony does not use and safely ignores.
        let mut cursor = std::io::Cursor::new(&bytes[cred_end..]);
        let cose: ciborium::Value = ciborium::de::from_reader(&mut cursor)
            .map_err(|_| WebAuthnError::Malformed("COSE key CBOR"))?;
        Some(AttestedCredential {
            credential_id,
            public_key_x963: cose_key_to_x963(&cose)?,
        })
    } else {
        None
    };

    Ok(AuthData {
        rp_id_hash,
        flags,
        sign_count,
        credential,
    })
}

/// Extract an ES256/P-256 public key from a COSE_Key map into the X9.63
/// uncompressed form `verify_es256` takes. Anything that is not exactly
/// {kty: EC2, alg: ES256, crv: P-256, 32-byte x, 32-byte y} is refused.
fn cose_key_to_x963(cose: &ciborium::Value) -> Result<[u8; 65], WebAuthnError> {
    let map = cose
        .as_map()
        .ok_or(WebAuthnError::Malformed("COSE key (not a map)"))?;
    let get = |label: i64| -> Option<&ciborium::Value> {
        map.iter()
            .find(|(k, _)| k.as_integer() == Some(label.into()))
            .map(|(_, v)| v)
    };
    let int_of =
        |v: &ciborium::Value| -> Option<i64> { v.as_integer().and_then(|i| i.try_into().ok()) };

    match get(1).and_then(int_of) {
        Some(2) => {}
        _ => return Err(WebAuthnError::UnsupportedKey("kty is not EC2")),
    }
    match get(3).and_then(int_of) {
        Some(-7) => {}
        _ => return Err(WebAuthnError::UnsupportedKey("alg is not ES256 (-7)")),
    }
    match get(-1).and_then(int_of) {
        Some(1) => {}
        _ => return Err(WebAuthnError::UnsupportedKey("crv is not P-256")),
    }
    let x = get(-2)
        .and_then(|v| v.as_bytes())
        .filter(|b| b.len() == 32)
        .ok_or(WebAuthnError::UnsupportedKey("x is not 32 bytes"))?;
    let y = get(-3)
        .and_then(|v| v.as_bytes())
        .filter(|b| b.len() == 32)
        .ok_or(WebAuthnError::UnsupportedKey("y is not 32 bytes"))?;

    let mut out = [0u8; 65];
    out[0] = 0x04;
    out[1..33].copy_from_slice(x);
    out[33..65].copy_from_slice(y);
    Ok(out)
}

/// Split an attestationObject into (fmt, authData bytes). Only fmt "none" is
/// accepted; the attStmt of any other format would go unverified, and an
/// unverified check must not exist ("honesty is a feature").
fn parse_attestation_object(bytes: &[u8]) -> Result<Vec<u8>, WebAuthnError> {
    let value: ciborium::Value = ciborium::de::from_reader(std::io::Cursor::new(bytes))
        .map_err(|_| WebAuthnError::Malformed("attestationObject CBOR"))?;
    let map = value
        .as_map()
        .ok_or(WebAuthnError::Malformed("attestationObject (not a map)"))?;
    let field = |name: &str| -> Option<&ciborium::Value> {
        map.iter()
            .find(|(k, _)| k.as_text() == Some(name))
            .map(|(_, v)| v)
    };
    match field("fmt").and_then(|v| v.as_text()) {
        Some("none") => {}
        _ => return Err(WebAuthnError::UnsupportedAttestation),
    }
    field("authData")
        .and_then(|v| v.as_bytes())
        .map(|b| b.to_vec())
        .ok_or(WebAuthnError::Malformed("authData missing"))
}

// ---------------------------------------------------------------------------
// ceremony verification
// ---------------------------------------------------------------------------

/// A verified registration, ready to enroll.
#[derive(Debug)]
pub struct VerifiedRegistration {
    pub credential_id: String,
    pub public_key_x963: String,
    pub sign_count: u32,
    /// Whether the authenticator asserted user VERIFICATION (biometric/PIN),
    /// beyond presence. Recorded for honesty, not (yet) required.
    pub user_verified: bool,
}

/// Verify a registration response (`navigator.credentials.create`).
///
/// `expected_challenge` is the base64url string minted by [`PendingCeremonies`];
/// the caller has already redeemed it (one-shot) and checked `Purpose::Register`.
pub fn verify_registration(
    rp: &RpConfig,
    expected_challenge: &str,
    client_data_json: &[u8],
    attestation_object: &[u8],
) -> Result<VerifiedRegistration, WebAuthnError> {
    let client = parse_client_data(client_data_json)?;
    if client.kind != "webauthn.create" {
        return Err(WebAuthnError::Mismatch("ceremony type"));
    }
    if client.challenge != expected_challenge {
        return Err(WebAuthnError::Mismatch("challenge"));
    }
    if client.origin != rp.origin {
        return Err(WebAuthnError::Mismatch("origin"));
    }

    let auth_data_bytes = parse_attestation_object(attestation_object)?;
    let auth = parse_authenticator_data(&auth_data_bytes)?;
    if auth.rp_id_hash != rp_id_hash(&rp.rp_id) {
        return Err(WebAuthnError::Mismatch("rp id"));
    }
    if auth.flags & FLAG_UP == 0 {
        return Err(WebAuthnError::UserNotPresent);
    }
    let cred = auth
        .credential
        .ok_or(WebAuthnError::Malformed("attestedCredentialData missing"))?;

    Ok(VerifiedRegistration {
        credential_id: B64URL.encode(&cred.credential_id),
        public_key_x963: B64.encode(cred.public_key_x963),
        sign_count: auth.sign_count,
        user_verified: auth.flags & FLAG_UV != 0,
    })
}

/// A verified per-action assertion.
#[derive(Debug)]
pub struct VerifiedAssertion {
    /// New signature counter to persist.
    pub sign_count: u32,
    pub user_verified: bool,
}

/// Verify an assertion response (`navigator.credentials.get`) against ONE
/// enrolled passkey.
///
/// The caller has already: redeemed the challenge one-shot, matched
/// `Purpose::Action{command, args_sha256}` against the command actually being
/// dispatched, and located `record` by the credential id the response names.
/// What remains here is the cryptographic core: the signature is ES256 over
/// `authenticatorData || SHA-256(clientDataJSON)` (WebAuthn L2 §7.2), checked
/// with the exact `verify_es256` the device-pairing path trusts.
pub fn verify_assertion(
    rp: &RpConfig,
    expected_challenge: &str,
    record: &PasskeyRecord,
    client_data_json: &[u8],
    authenticator_data: &[u8],
    signature_der: &[u8],
) -> Result<VerifiedAssertion, WebAuthnError> {
    let client = parse_client_data(client_data_json)?;
    if client.kind != "webauthn.get" {
        return Err(WebAuthnError::Mismatch("ceremony type"));
    }
    if client.challenge != expected_challenge {
        return Err(WebAuthnError::Mismatch("challenge"));
    }
    if client.origin != rp.origin {
        return Err(WebAuthnError::Mismatch("origin"));
    }

    let auth = parse_authenticator_data(authenticator_data)?;
    if auth.rp_id_hash != rp_id_hash(&rp.rp_id) {
        return Err(WebAuthnError::Mismatch("rp id"));
    }
    if auth.flags & FLAG_UP == 0 {
        return Err(WebAuthnError::UserNotPresent);
    }

    // message = authenticatorData || SHA-256(clientDataJSON)
    let mut message = authenticator_data.to_vec();
    message.extend_from_slice(&Sha256::digest(client_data_json));

    // Browsers deliver an ASN.1/DER ECDSA signature; convert to the raw r||s
    // form and verify with the shared primitive. Both steps are fail-closed.
    let pubkey = B64
        .decode(&record.public_key_x963)
        .map_err(|_| WebAuthnError::Malformed("stored public key"))?;
    let raw = genaryx_signing::der_to_raw_rs(signature_der)
        .map_err(|_| WebAuthnError::Malformed("signature DER"))?;
    genaryx_signing::verify_es256(&pubkey, &message, &raw)
        .map_err(|_| WebAuthnError::BadSignature)?;

    // Counter regression = a second physical device holding the same key.
    // A perpetual 0 means "this authenticator does not count" (Apple
    // passkeys), which cannot be distinguished from anything and is allowed.
    if record.sign_count > 0 && auth.sign_count > 0 && auth.sign_count <= record.sign_count {
        return Err(WebAuthnError::CloneSuspected);
    }

    Ok(VerifiedAssertion {
        sign_count: auth.sign_count,
        user_verified: auth.flags & FLAG_UV != 0,
    })
}

fn rp_id_hash(rp_id: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&Sha256::digest(rp_id.as_bytes()));
    out
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

/// Test-only builders for the BROWSER/authenticator half of the ceremony,
/// shared with `main.rs`'s gate tests (which play a whole browser against the
/// real router). `pub(crate)` and `cfg(test)`: never compiled into the
/// shipped binary.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use genaryx_signing::es256::{Es256Signer, SoftwareSigner};

    pub(crate) fn client_data(kind: &str, challenge: &str, origin: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "type": kind, "challenge": challenge, "origin": origin,
            "crossOrigin": false
        }))
        .unwrap()
    }

    /// Build a COSE_Key CBOR value from an X9.63 public key.
    pub(crate) fn cose_key(x963: &[u8]) -> ciborium::Value {
        assert_eq!(x963.len(), 65);
        ciborium::Value::Map(vec![
            (
                ciborium::Value::Integer(1.into()),
                ciborium::Value::Integer(2.into()),
            ),
            (
                ciborium::Value::Integer(3.into()),
                ciborium::Value::Integer((-7).into()),
            ),
            (
                ciborium::Value::Integer((-1).into()),
                ciborium::Value::Integer(1.into()),
            ),
            (
                ciborium::Value::Integer((-2).into()),
                ciborium::Value::Bytes(x963[1..33].to_vec()),
            ),
            (
                ciborium::Value::Integer((-3).into()),
                ciborium::Value::Bytes(x963[33..65].to_vec()),
            ),
        ])
    }

    pub(crate) fn auth_data(
        rp_id: &str,
        flags: u8,
        sign_count: u32,
        cred: Option<(&[u8], &[u8])>,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&rp_id_hash(rp_id));
        out.push(flags);
        out.extend_from_slice(&sign_count.to_be_bytes());
        if let Some((cred_id, x963)) = cred {
            out.extend_from_slice(&[0u8; 16]); // aaguid
            out.extend_from_slice(&(cred_id.len() as u16).to_be_bytes());
            out.extend_from_slice(cred_id);
            let mut cose = Vec::new();
            ciborium::ser::into_writer(&cose_key(x963), &mut cose).unwrap();
            out.extend_from_slice(&cose);
        }
        out
    }

    pub(crate) fn attestation_object(fmt: &str, auth_data: &[u8]) -> Vec<u8> {
        let value = ciborium::Value::Map(vec![
            (
                ciborium::Value::Text("fmt".into()),
                ciborium::Value::Text(fmt.into()),
            ),
            (
                ciborium::Value::Text("attStmt".into()),
                ciborium::Value::Map(vec![]),
            ),
            (
                ciborium::Value::Text("authData".into()),
                ciborium::Value::Bytes(auth_data.to_vec()),
            ),
        ]);
        let mut out = Vec::new();
        ciborium::ser::into_writer(&value, &mut out).unwrap();
        out
    }

    pub(crate) fn signer() -> SoftwareSigner {
        SoftwareSigner::from_scalar(&[0x42u8; 32]).unwrap()
    }

    pub(crate) fn enrolled(signer: &SoftwareSigner, sign_count: u32) -> PasskeyRecord {
        enrolled_with_id(signer, b"cred-1", sign_count)
    }

    /// An enrollment under a chosen credential id, for the tests that need
    /// TWO enrolled authenticators (removal: one key confirms the removal of
    /// the other; enrollment: an enrolled key authorizes adding the next).
    pub(crate) fn enrolled_with_id(
        signer: &SoftwareSigner,
        credential_id: &[u8],
        sign_count: u32,
    ) -> PasskeyRecord {
        PasskeyRecord {
            credential_id: B64URL.encode(credential_id),
            public_key_x963: B64.encode(signer.public_key_x963().unwrap()),
            sign_count,
            created_at: "2026-07-24T00:00:00Z".into(),
            label: "test key".into(),
        }
    }

    /// The browser half of a REGISTRATION: the `clientDataJSON` and
    /// `attestationObject` a `navigator.credentials.create` would hand back
    /// for `credential_id`, under this module's test authenticator key. Kept
    /// here rather than in `main.rs`'s tests so the signer trait import and
    /// the flag constants stay in the one module that owns them.
    pub(crate) fn registration_response(
        rp_id: &str,
        origin: &str,
        challenge: &str,
        credential_id: &[u8],
    ) -> (Vec<u8>, Vec<u8>) {
        let s = signer();
        let x963 = s.public_key_x963().unwrap();
        let ad = auth_data(
            rp_id,
            FLAG_UP | FLAG_UV | FLAG_AT,
            0,
            Some((credential_id, &x963)),
        );
        (
            client_data("webauthn.create", challenge, origin),
            attestation_object("none", &ad),
        )
    }

    /// Sign like a browser authenticator: DER ECDSA over authData || sha256(clientData).
    pub(crate) fn assert_sign(
        signer: &SoftwareSigner,
        auth_data: &[u8],
        client_data: &[u8],
    ) -> Vec<u8> {
        let mut message = auth_data.to_vec();
        message.extend_from_slice(&Sha256::digest(client_data));
        let raw = signer.sign_raw(&message).unwrap();
        p256::ecdsa::Signature::from_slice(&raw)
            .unwrap()
            .to_der()
            .as_bytes()
            .to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use genaryx_signing::es256::{Es256Signer, SoftwareSigner};

    const RP: &str = "localhost";
    const ORIGIN: &str = "http://localhost:7420";

    fn rp() -> RpConfig {
        RpConfig {
            rp_id: RP.into(),
            origin: ORIGIN.into(),
        }
    }

    // ---- registration ----

    #[test]
    fn registration_round_trips_the_public_key() {
        let s = signer();
        let x963 = s.public_key_x963().unwrap();
        let ad = auth_data(RP, FLAG_UP | FLAG_UV | FLAG_AT, 7, Some((b"cred-1", &x963)));
        let cd = client_data("webauthn.create", "chal-abc", ORIGIN);
        let reg = verify_registration(&rp(), "chal-abc", &cd, &attestation_object("none", &ad))
            .expect("genuine registration verifies");
        assert_eq!(reg.credential_id, B64URL.encode(b"cred-1"));
        assert_eq!(reg.public_key_x963, B64.encode(&x963));
        assert_eq!(reg.sign_count, 7);
        assert!(reg.user_verified);
    }

    #[test]
    fn registration_rejects_every_tampered_binding() {
        let s = signer();
        let x963 = s.public_key_x963().unwrap();
        let ad = auth_data(RP, FLAG_UP | FLAG_AT, 0, Some((b"cred-1", &x963)));
        let ao = attestation_object("none", &ad);

        // wrong ceremony type
        let cd = client_data("webauthn.get", "chal", ORIGIN);
        assert_eq!(
            verify_registration(&rp(), "chal", &cd, &ao).unwrap_err(),
            WebAuthnError::Mismatch("ceremony type")
        );
        // wrong challenge
        let cd = client_data("webauthn.create", "other", ORIGIN);
        assert_eq!(
            verify_registration(&rp(), "chal", &cd, &ao).unwrap_err(),
            WebAuthnError::Mismatch("challenge")
        );
        // wrong origin
        let cd = client_data("webauthn.create", "chal", "https://evil.example");
        assert_eq!(
            verify_registration(&rp(), "chal", &cd, &ao).unwrap_err(),
            WebAuthnError::Mismatch("origin")
        );
        // wrong rp id
        let bad = auth_data(
            "evil.example",
            FLAG_UP | FLAG_AT,
            0,
            Some((b"cred-1", &x963)),
        );
        let cd = client_data("webauthn.create", "chal", ORIGIN);
        assert_eq!(
            verify_registration(&rp(), "chal", &cd, &attestation_object("none", &bad)).unwrap_err(),
            WebAuthnError::Mismatch("rp id")
        );
        // user not present
        let bad = auth_data(RP, FLAG_AT, 0, Some((b"cred-1", &x963)));
        assert_eq!(
            verify_registration(&rp(), "chal", &cd, &attestation_object("none", &bad)).unwrap_err(),
            WebAuthnError::UserNotPresent
        );
        // non-"none" attestation refused, never half-verified
        assert_eq!(
            verify_registration(&rp(), "chal", &cd, &attestation_object("packed", &ad))
                .unwrap_err(),
            WebAuthnError::UnsupportedAttestation
        );
        // garbage never panics
        assert!(verify_registration(&rp(), "chal", b"not json", &ao).is_err());
        assert!(verify_registration(&rp(), "chal", &cd, b"not cbor").is_err());
    }

    #[test]
    fn cose_key_rejects_non_es256_shapes() {
        let s = signer();
        let x963 = s.public_key_x963().unwrap();
        let mut wrong_alg = cose_key(&x963);
        if let ciborium::Value::Map(entries) = &mut wrong_alg {
            entries[1].1 = ciborium::Value::Integer((-257).into()); // RS256
        }
        assert_eq!(
            cose_key_to_x963(&wrong_alg).unwrap_err(),
            WebAuthnError::UnsupportedKey("alg is not ES256 (-7)")
        );
    }

    // ---- assertion ----

    #[test]
    fn assertion_round_trips_and_updates_the_counter() {
        let s = signer();
        let rec = enrolled(&s, 5);
        let ad = auth_data(RP, FLAG_UP | FLAG_UV, 6, None);
        let cd = client_data("webauthn.get", "chal-77", ORIGIN);
        let sig = assert_sign(&s, &ad, &cd);
        let v = verify_assertion(&rp(), "chal-77", &rec, &cd, &ad, &sig)
            .expect("genuine assertion verifies");
        assert_eq!(v.sign_count, 6);
        assert!(v.user_verified);
    }

    #[test]
    fn assertion_rejects_tampering_wrong_key_and_replay_shapes() {
        let s = signer();
        let rec = enrolled(&s, 5);
        let ad = auth_data(RP, FLAG_UP, 6, None);
        let cd = client_data("webauthn.get", "chal", ORIGIN);
        let sig = assert_sign(&s, &ad, &cd);

        // signature over different authData must fail
        let other_ad = auth_data(RP, FLAG_UP, 99, None);
        assert_eq!(
            verify_assertion(&rp(), "chal", &rec, &cd, &other_ad, &sig).unwrap_err(),
            WebAuthnError::BadSignature
        );
        // a stranger's signature must fail
        let stranger = SoftwareSigner::from_scalar(&[0x77u8; 32]).unwrap();
        let forged = assert_sign(&stranger, &ad, &cd);
        assert_eq!(
            verify_assertion(&rp(), "chal", &rec, &cd, &ad, &forged).unwrap_err(),
            WebAuthnError::BadSignature
        );
        // tampered client data (origin swap) fails the binding check first
        let evil_cd = client_data("webauthn.get", "chal", "https://evil.example");
        assert_eq!(
            verify_assertion(&rp(), "chal", &rec, &evil_cd, &ad, &sig).unwrap_err(),
            WebAuthnError::Mismatch("origin")
        );
        // user presence is mandatory
        let no_up = auth_data(RP, 0, 6, None);
        let sig_no_up = assert_sign(&s, &no_up, &cd);
        assert_eq!(
            verify_assertion(&rp(), "chal", &rec, &cd, &no_up, &sig_no_up).unwrap_err(),
            WebAuthnError::UserNotPresent
        );
        // malformed DER is an error, not a panic
        assert!(verify_assertion(&rp(), "chal", &rec, &cd, &ad, b"not der").is_err());
    }

    #[test]
    fn counter_regression_is_clone_suspected_but_zero_is_allowed() {
        let s = signer();
        // regression: stored 5, asserted 5 (not greater) -> refuse
        let rec = enrolled(&s, 5);
        let ad = auth_data(RP, FLAG_UP, 5, None);
        let cd = client_data("webauthn.get", "chal", ORIGIN);
        let sig = assert_sign(&s, &ad, &cd);
        assert_eq!(
            verify_assertion(&rp(), "chal", &rec, &cd, &ad, &sig).unwrap_err(),
            WebAuthnError::CloneSuspected
        );
        // an authenticator that never counts (0) stays acceptable
        let rec0 = enrolled(&s, 0);
        let ad0 = auth_data(RP, FLAG_UP, 0, None);
        let sig0 = assert_sign(&s, &ad0, &cd);
        assert!(verify_assertion(&rp(), "chal", &rec0, &cd, &ad0, &sig0).is_ok());
    }

    // ---- pending ceremonies ----

    #[test]
    fn challenges_are_one_shot_and_user_bound() {
        let p = PendingCeremonies::default();
        let ch = p.mint("alice", Purpose::Register).unwrap();
        // wrong user cannot redeem (and the attempt burns nothing for alice?
        // no - one-shot means REMOVED; a cross-user probe consuming another
        // user's pending is still a refusal for both, fail closed)
        assert_eq!(
            p.take("mallory", &ch).unwrap_err(),
            WebAuthnError::WrongUser
        );
        // gone now (one-shot), even for the right user
        assert_eq!(
            p.take("alice", &ch).unwrap_err(),
            WebAuthnError::UnknownChallenge
        );

        // action purpose survives the round trip
        let ch2 = p
            .mint(
                "alice",
                Purpose::Action {
                    command: "money_kill_run".into(),
                    args_sha256: "abc".into(),
                },
            )
            .unwrap();
        match p.take("alice", &ch2).unwrap() {
            Purpose::Action {
                command,
                args_sha256,
            } => {
                assert_eq!(command, "money_kill_run");
                assert_eq!(args_sha256, "abc");
            }
            other => panic!("wrong purpose: {other:?}"),
        }
        // never-minted challenge is unknown
        assert_eq!(
            p.take("alice", "bogus").unwrap_err(),
            WebAuthnError::UnknownChallenge
        );
    }

    // ---- store ----

    #[test]
    fn store_round_trips_enrollments_and_refuses_duplicates() {
        let dir = std::env::temp_dir().join(format!("gw-webauthn-{}", std::process::id()));
        let path = dir.join("passkeys.json");
        let _ = std::fs::remove_file(&path);

        let s = signer();
        let store = PasskeyStore::open(path.clone()).unwrap();
        assert!(!store.has_any("alice"));
        store.add("alice", enrolled(&s, 0)).unwrap();
        assert!(store.has_any("alice"));
        assert_eq!(
            store.add("alice", enrolled(&s, 0)).unwrap_err(),
            WebAuthnError::Store("credential already enrolled".into())
        );
        store
            .update_sign_count("alice", &B64URL.encode(b"cred-1"), 9)
            .unwrap();

        // a fresh open reads the same state back
        let reopened = PasskeyStore::open(path.clone()).unwrap();
        let list = reopened.for_user("alice");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].sign_count, 9);

        // corrupt file is an ERROR, never silently empty
        std::fs::write(&path, "{ not json").unwrap();
        assert!(matches!(
            PasskeyStore::open(path.clone()),
            Err(WebAuthnError::Store(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
