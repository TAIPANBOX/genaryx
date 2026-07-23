//! Who is allowed to drive this console, and for how long.
//!
//! One operator account, stored as an Argon2id PHC string in
//! `operator.json`. A security product cannot keep a console password in the
//! clear, and it cannot roll its own hash either, so this is stock Argon2id
//! with a per-password random salt and the crate's default parameters.
//!
//! Sessions live in memory only. That is deliberate: a restart of this
//! process invalidates every session, which is the behaviour an operator
//! actually wants from a control plane (restarting it is a way to cut
//! access), and it means there is no session table on disk to steal. The
//! cost, having to sign in again after a restart, is the right trade for
//! something whose whole job is to be able to stop things.
//!
//! What this is NOT: an identity provider. There is one operator per box,
//! because the box belongs to one customer and the console's privileged
//! actions are already individually re-signed (D11/D13: a destructive action
//! is confirmed with a hardware-backed signature, not with "you are logged
//! in"). Signing in gets you the console; it does not get you the kill.

use crate::roles::Role;
use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use rand::RngCore;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a session survives without being used.
const IDLE_TIMEOUT: Duration = Duration::from_secs(12 * 60 * 60);

/// Name of the session cookie.
pub const COOKIE: &str = "genaryx_session";

/// The one credential record, as stored on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operator {
    pub username: String,
    /// Argon2id PHC string. Never the password, never a reversible form of
    /// it, and never logged.
    pub phc: String,
}

/// Read the operator record, if one has been set.
pub fn load(path: &Path) -> Option<Operator> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Write the operator record, hashing the password with Argon2id.
///
/// Creates the state directory if needed and tightens the file's mode on
/// unix: the hash is not a secret you can reverse, but it is a secret you
/// can attack offline, so it does not get world-readable.
pub fn set_operator(path: &Path, username: &str, password: &str) -> Result<(), String> {
    if username.trim().is_empty() {
        return Err("username must not be empty".into());
    }
    if password.chars().count() < 12 {
        return Err("password must be at least 12 characters".into());
    }
    let salt = SaltString::generate(&mut OsRng);
    let phc = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| format!("hashing failed: {e}"))?
        .to_string();

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let rec = Operator {
        username: username.to_string(),
        phc,
    };
    let body = serde_json::to_string_pretty(&rec).map_err(|e| e.to_string())?;
    std::fs::write(path, body).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    restrict(path);
    Ok(())
}

#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict(_path: &Path) {}

/// Check a username and password against the stored record.
///
/// Verifies the hash even when the username does not match, so a wrong
/// username and a wrong password take the same time to fail and the endpoint
/// cannot be used to enumerate the operator's name.
pub fn verify(op: &Operator, username: &str, password: &str) -> bool {
    let parsed = match PasswordHash::new(&op.phc) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let password_ok = Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok();
    // Deliberately not short-circuiting on the username.
    let user_ok = op.username == username;
    password_ok && user_ok
}

/// How an operator authenticated for this session (docs/CONSOLE-IDP.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// The local Argon2id owner account (break-glass).
    Local,
    /// A verified OIDC ID-token from the customer's IdP.
    Oidc,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Local => "local",
            Method::Oidc => "oidc",
        }
    }
}

/// A resolved live session: who, at what role, by which method. Returned by
/// [`Sessions::touch`] so every caller (the session route, the command gate,
/// the actor binding) reads one consistent shape.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub user: String,
    pub role: Role,
    pub method: Method,
}

struct Session {
    user: String,
    role: Role,
    method: Method,
    last_seen: Instant,
}

/// In-memory session table.
#[derive(Default)]
pub struct Sessions {
    inner: Mutex<HashMap<String, Session>>,
}

impl Sessions {
    /// Mint a session id for a signed-in operator at a resolved role/method.
    pub fn create(&self, user: &str, role: Role, method: Method) -> String {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let id = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let mut guard = self.inner.lock().expect("session table poisoned");
        guard.retain(|_, s| s.last_seen.elapsed() < IDLE_TIMEOUT);
        guard.insert(
            id.clone(),
            Session {
                user: user.to_string(),
                role,
                method,
                last_seen: Instant::now(),
            },
        );
        id
    }

    /// Resolve a session id, refreshing its idle clock. `None` means expired
    /// or never existed, and the caller must not distinguish the two.
    pub fn touch(&self, id: &str) -> Option<SessionInfo> {
        let mut guard = self.inner.lock().expect("session table poisoned");
        match guard.get_mut(id) {
            Some(s) if s.last_seen.elapsed() < IDLE_TIMEOUT => {
                s.last_seen = Instant::now();
                Some(SessionInfo {
                    user: s.user.clone(),
                    role: s.role,
                    method: s.method,
                })
            }
            Some(_) => {
                guard.remove(id);
                None
            }
            None => None,
        }
    }

    /// Drop a session (sign out).
    pub fn revoke(&self, id: &str) {
        self.inner
            .lock()
            .expect("session table poisoned")
            .remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A written-then-read record, in a directory of this test's own.
    ///
    /// Named per test on purpose: cargo runs these on parallel threads in one
    /// process, so a directory keyed only by pid is shared, and the first
    /// test to finish deletes it out from under the others.
    fn rec(who: &str) -> Operator {
        let dir = std::env::temp_dir().join(format!("gwauth-{}-{}", std::process::id(), who));
        let p = dir.join("operator.json");
        set_operator(&p, "ops", "correct horse battery").unwrap();
        let op = load(&p).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        op
    }

    #[test]
    fn stores_a_hash_and_not_the_password() {
        let op = rec("hash");
        assert!(!op.phc.contains("correct horse battery"));
        assert!(op.phc.starts_with("$argon2id$"));
    }

    #[test]
    fn accepts_the_right_pair_and_rejects_the_rest() {
        let op = rec("verify");
        assert!(verify(&op, "ops", "correct horse battery"));
        assert!(!verify(&op, "ops", "wrong password here"));
        assert!(!verify(&op, "someone", "correct horse battery"));
    }

    #[test]
    fn refuses_a_short_password() {
        let dir = std::env::temp_dir().join(format!("gwauth-short{}", std::process::id()));
        let p = dir.join("operator.json");
        assert!(set_operator(&p, "ops", "short").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_session_resolves_once_created_and_not_after_revoke() {
        let s = Sessions::default();
        let id = s.create("ops", Role::Admin, Method::Local);
        let info = s.touch(&id).expect("live");
        assert_eq!(info.user, "ops");
        assert_eq!(info.role, Role::Admin);
        assert_eq!(info.method, Method::Local);
        s.revoke(&id);
        assert!(s.touch(&id).is_none());
    }

    #[test]
    fn session_carries_the_role_and_method_it_was_created_with() {
        let s = Sessions::default();
        let id = s.create("alice", Role::Viewer, Method::Oidc);
        let info = s.touch(&id).expect("live");
        assert_eq!(info.role, Role::Viewer);
        assert_eq!(info.method, Method::Oidc);
    }

    #[test]
    fn session_ids_are_unpredictable_and_distinct() {
        let s = Sessions::default();
        let a = s.create("ops", Role::Admin, Method::Local);
        let b = s.create("ops", Role::Admin, Method::Local);
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
    }
}
