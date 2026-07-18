//! Single-device registry + pairing-window state (docs/PHASE5.md "registry"
//! module; itrat-console/13 D12.3: "Single-device: relay-enforced, one row;
//! `POST /relay/v1/pair` returns 409 `device_exists` while a row is active;
//! the pairing window cannot even be armed while paired").
//!
//! One `rusqlite` connection behind a `Mutex` (this crate serves concurrent
//! axum requests, unlike `genaryx_core::Store`'s single-threaded ingest
//! caller, so the connection needs its own synchronization here). Schema:
//! at most one `device` row and at most one `pairing_window` row (`id = 1`
//! singleton, enforced by a `CHECK` constraint).

use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// A device is already paired -- `POST /relay/v1/pair` and
    /// `POST /admin/pairing-window` both fail closed with this (mapped to
    /// HTTP 409 by their handlers).
    #[error("device already paired")]
    DeviceExists,
    /// No pairing window is open, it expired, or the presented code does not
    /// match the armed window's hash. Deliberately one variant for all three
    /// (see [`Registry::check_pairing_code`]'s doc): the public pairing route
    /// stays "dark" outside a window (D12.3), so a caller must not be able to
    /// distinguish "wrong code" from "no window" from the response shape.
    #[error("no matching open pairing window")]
    WindowNotOpen,
}

/// A paired device row, as stored (includes the Cloud-issued `device_token`
/// used to authenticate the phone's calls to the relay's own endpoints).
#[derive(Debug, Clone)]
pub struct DeviceRow {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub org: String,
    pub role: String,
    pub device_token: String,
    pub paired_at_unix: i64,
    pub last_seen_unix: i64,
    pub apns_token: Option<String>,
}

/// Fields needed to insert a freshly paired device (mirrors the Cloud's own
/// `PairResponse`, `cloud_rest.rs`, plus the platform/name the phone sent).
#[derive(Debug, Clone)]
pub struct NewDevice {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub org: String,
    pub role: String,
    pub device_token: String,
    pub paired_at_unix: i64,
}

pub struct Registry {
    conn: Mutex<Connection>,
}

const MIGRATION_V1: &str = "
CREATE TABLE IF NOT EXISTS device (
    device_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    platform TEXT NOT NULL,
    org TEXT NOT NULL,
    role TEXT NOT NULL,
    device_token TEXT NOT NULL,
    paired_at_unix INTEGER NOT NULL,
    last_seen_unix INTEGER NOT NULL,
    apns_token TEXT
);

CREATE TABLE IF NOT EXISTS pairing_window (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    code_sha256 TEXT NOT NULL,
    expires_unix INTEGER NOT NULL
);
";

impl Registry {
    /// Open (or create) the registry at `path`: WAL + fail-closed pragmas
    /// (mirrors `genaryx_core::store`'s own pragma set), then idempotent
    /// migrations.
    pub fn open(path: &Path) -> Result<Self, RegistryError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    /// In-memory registry: same schema, no file on disk. Used by tests.
    pub fn open_in_memory() -> Result<Self, RegistryError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self, RegistryError> {
        conn.pragma_update(None, "journal_mode", "WAL").ok(); // no-op / errs harmlessly for :memory:
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.execute_batch(MIGRATION_V1)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// The one paired device row, if any.
    pub fn current_device(&self) -> Result<Option<DeviceRow>, RegistryError> {
        let conn = self.conn.lock().expect("registry mutex poisoned");
        conn.query_row(
            "SELECT device_id, name, platform, org, role, device_token, \
                    paired_at_unix, last_seen_unix, apns_token \
             FROM device LIMIT 1",
            [],
            row_to_device,
        )
        .optional()
        .map_err(RegistryError::from)
    }

    pub fn has_device(&self) -> Result<bool, RegistryError> {
        Ok(self.current_device()?.is_some())
    }

    /// Arm the pairing window: refuses with [`RegistryError::DeviceExists`]
    /// while a device is paired (D12.3: "the pairing window cannot even be
    /// armed while paired -- the desktop shows Disconnect instead"), else
    /// (re)writes the singleton `pairing_window` row.
    pub fn arm_pairing_window(
        &self,
        code_sha256: &str,
        expires_unix: i64,
    ) -> Result<(), RegistryError> {
        let mut conn = self.conn.lock().expect("registry mutex poisoned");
        let tx = conn.transaction()?;
        let paired: i64 = tx.query_row("SELECT COUNT(*) FROM device", [], |r| r.get(0))?;
        if paired > 0 {
            return Err(RegistryError::DeviceExists);
        }
        tx.execute(
            "INSERT INTO pairing_window (id, code_sha256, expires_unix) VALUES (1, ?1, ?2) \
             ON CONFLICT(id) DO UPDATE SET code_sha256 = excluded.code_sha256, \
                                            expires_unix = excluded.expires_unix",
            params![code_sha256, expires_unix],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Peek-only check: is a pairing window open, unexpired, and does its
    /// hash match `code`? Does not consume the window -- a Cloud-side
    /// rejection of the same code must leave the window intact for a retry
    /// within its TTL (D12.2 step 8 closes the window only on success).
    pub fn check_pairing_code(&self, code: &str, now_unix: i64) -> Result<(), RegistryError> {
        let conn = self.conn.lock().expect("registry mutex poisoned");
        let row: Option<(String, i64)> = conn
            .query_row(
                "SELECT code_sha256, expires_unix FROM pairing_window WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((code_sha256, expires_unix)) = row else {
            return Err(RegistryError::WindowNotOpen);
        };
        if now_unix >= expires_unix {
            return Err(RegistryError::WindowNotOpen);
        }
        let presented = sha256_hex(code.as_bytes());
        if !constant_time_eq(presented.as_bytes(), code_sha256.as_bytes()) {
            return Err(RegistryError::WindowNotOpen);
        }
        Ok(())
    }

    /// Insert the newly paired device and close the pairing window, in one
    /// transaction (re-checks the single-device slot is still empty, closing
    /// a race between two phones redeeming the same window). Called only
    /// AFTER the code has been redeemed at the Cloud (`pairing.rs`), so a
    /// [`RegistryError::DeviceExists`] here means a concurrent pairing won
    /// the race -- the caller already has a Cloud-registered device+token
    /// for a slot it will not get, which `pairing.rs` logs (Cloud-side
    /// revocation of that orphan is a later PR, same as Disconnect's).
    pub fn insert_paired_device(&self, new: NewDevice) -> Result<(), RegistryError> {
        let mut conn = self.conn.lock().expect("registry mutex poisoned");
        let tx = conn.transaction()?;
        let paired: i64 = tx.query_row("SELECT COUNT(*) FROM device", [], |r| r.get(0))?;
        if paired > 0 {
            return Err(RegistryError::DeviceExists);
        }
        tx.execute(
            "INSERT INTO device (device_id, name, platform, org, role, device_token, \
                                  paired_at_unix, last_seen_unix, apns_token) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, NULL)",
            params![
                new.device_id,
                new.name,
                new.platform,
                new.org,
                new.role,
                new.device_token,
                new.paired_at_unix,
            ],
        )?;
        tx.execute("DELETE FROM pairing_window WHERE id = 1", [])?;
        tx.commit()?;
        Ok(())
    }

    /// Delete the paired device (+ its APNs token, same row) and any open
    /// pairing window. Returns whether a device was actually present.
    /// Upstream Cloud-side revocation of the device token is a later PR
    /// (itrat-console/13 D12.3 R5: no `DELETE /v1/devices/{id}` yet) --
    /// noted here, not silently implied.
    pub fn disconnect(&self) -> Result<bool, RegistryError> {
        let conn = self.conn.lock().expect("registry mutex poisoned");
        let deleted = conn.execute("DELETE FROM device", [])?;
        conn.execute("DELETE FROM pairing_window WHERE id = 1", [])?;
        Ok(deleted > 0)
    }

    /// Constant-time bearer check against the one paired device's token
    /// (D12.2c step 3: "token matches (constant-time)"). `None` if no device
    /// is paired or the token doesn't match -- callers must treat both
    /// identically (401), never distinguish them in the response.
    pub fn verify_bearer(&self, token: &str) -> Result<Option<DeviceRow>, RegistryError> {
        let device = self.current_device()?;
        Ok(device.filter(|d| constant_time_eq(d.device_token.as_bytes(), token.as_bytes())))
    }

    /// Update `last_seen_unix` for the paired device (best-effort freshness
    /// for the desktop's Pocket panel device view).
    pub fn touch_last_seen(&self, device_id: &str, now_unix: i64) -> Result<(), RegistryError> {
        let conn = self.conn.lock().expect("registry mutex poisoned");
        conn.execute(
            "UPDATE device SET last_seen_unix = ?1 WHERE device_id = ?2",
            params![now_unix, device_id],
        )?;
        Ok(())
    }
}

fn row_to_device(r: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceRow> {
    Ok(DeviceRow {
        device_id: r.get(0)?,
        name: r.get(1)?,
        platform: r.get(2)?,
        org: r.get(3)?,
        role: r.get(4)?,
        device_token: r.get(5)?,
        paired_at_unix: r.get(6)?,
        last_seen_unix: r.get(7)?,
        apns_token: r.get(8)?,
    })
}

/// Lowercase-hex SHA-256, reusing `genaryx_signing`'s existing helper
/// (`es256::body_sha256_hex`) rather than a second hand-rolled hasher --
/// this crate's one real (non-test) call into `genaryx-signing`.
fn sha256_hex(bytes: &[u8]) -> String {
    genaryx_signing::body_sha256_hex(bytes)
}

/// Byte-for-byte constant-time comparison (no early return on the first
/// differing byte), for the bearer-token and pairing-code-hash checks above.
/// Lengths differing is an immediate `false`: revealing "wrong length" is not
/// a meaningful timing side channel for a 64-hex-char token/hash pair, and
/// requiring equal length up front is the same trade-off `ring`'s own
/// `constant_time::verify_slices_are_equal` makes.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_device(id: &str) -> NewDevice {
        NewDevice {
            device_id: id.to_string(),
            name: "iPhone".to_string(),
            platform: "ios".to_string(),
            org: "acme".to_string(),
            role: "admin".to_string(),
            device_token: format!("token-{id}"),
            paired_at_unix: 1_000,
        }
    }

    #[test]
    fn no_device_paired_initially() {
        let reg = Registry::open_in_memory().unwrap();
        assert!(!reg.has_device().unwrap());
        assert!(reg.current_device().unwrap().is_none());
    }

    #[test]
    fn arm_check_and_insert_happy_path() {
        let reg = Registry::open_in_memory().unwrap();
        let code_sha256 = sha256_hex(b"ABCD1234");
        reg.arm_pairing_window(&code_sha256, 2_000).unwrap();

        reg.check_pairing_code("ABCD1234", 1_500)
            .expect("code matches, window open");

        reg.insert_paired_device(new_device("dev-1")).unwrap();
        let dev = reg.current_device().unwrap().expect("device now paired");
        assert_eq!(dev.device_id, "dev-1");
        assert_eq!(dev.device_token, "token-dev-1");
        assert!(dev.apns_token.is_none());

        // Window closed by a successful insert.
        assert!(matches!(
            reg.check_pairing_code("ABCD1234", 1_600),
            Err(RegistryError::WindowNotOpen)
        ));
    }

    #[test]
    fn second_pair_attempt_while_paired_is_device_exists() {
        let reg = Registry::open_in_memory().unwrap();
        reg.insert_paired_device(new_device("dev-1")).unwrap();

        let err = reg
            .arm_pairing_window(&sha256_hex(b"WHATEVER"), 9_999)
            .unwrap_err();
        assert!(matches!(err, RegistryError::DeviceExists));

        let err = reg.insert_paired_device(new_device("dev-2")).unwrap_err();
        assert!(matches!(err, RegistryError::DeviceExists));
        // Still the FIRST device, not overwritten.
        assert_eq!(reg.current_device().unwrap().unwrap().device_id, "dev-1");
    }

    #[test]
    fn wrong_code_and_expired_window_both_fail_the_same_way() {
        let reg = Registry::open_in_memory().unwrap();
        reg.arm_pairing_window(&sha256_hex(b"RIGHTCODE"), 2_000)
            .unwrap();

        let wrong = reg.check_pairing_code("WRONGCODE", 1_500).unwrap_err();
        assert!(matches!(wrong, RegistryError::WindowNotOpen));

        let expired = reg.check_pairing_code("RIGHTCODE", 2_500).unwrap_err();
        assert!(matches!(expired, RegistryError::WindowNotOpen));
    }

    #[test]
    fn no_window_open_is_window_not_open() {
        let reg = Registry::open_in_memory().unwrap();
        assert!(matches!(
            reg.check_pairing_code("ANYCODE", 1_000),
            Err(RegistryError::WindowNotOpen)
        ));
    }

    #[test]
    fn verify_bearer_matches_only_the_right_token() {
        let reg = Registry::open_in_memory().unwrap();
        reg.insert_paired_device(new_device("dev-1")).unwrap();

        assert!(reg.verify_bearer("token-dev-1").unwrap().is_some());
        assert!(reg.verify_bearer("token-dev-1x").unwrap().is_none());
        assert!(reg.verify_bearer("").unwrap().is_none());
    }

    #[test]
    fn disconnect_clears_device_and_window_and_frees_the_slot() {
        let reg = Registry::open_in_memory().unwrap();
        reg.insert_paired_device(new_device("dev-1")).unwrap();
        assert!(reg.disconnect().unwrap(), "a device was present");
        assert!(!reg.has_device().unwrap());
        assert!(!reg.disconnect().unwrap(), "nothing left to disconnect");

        // A fresh pairing window can now be armed and redeemed.
        reg.arm_pairing_window(&sha256_hex(b"NEWCODE"), 9_999)
            .unwrap();
        reg.check_pairing_code("NEWCODE", 1_000).unwrap();
        reg.insert_paired_device(new_device("dev-2")).unwrap();
        assert_eq!(reg.current_device().unwrap().unwrap().device_id, "dev-2");
    }

    #[test]
    fn touch_last_seen_updates_the_row() {
        let reg = Registry::open_in_memory().unwrap();
        reg.insert_paired_device(new_device("dev-1")).unwrap();
        reg.touch_last_seen("dev-1", 5_000).unwrap();
        assert_eq!(reg.current_device().unwrap().unwrap().last_seen_unix, 5_000);
    }

    #[test]
    fn constant_time_eq_behaves_like_eq_for_correctness() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(constant_time_eq(b"", b""));
    }
}
