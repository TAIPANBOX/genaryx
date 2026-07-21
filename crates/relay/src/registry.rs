//! Paired-device registry + pairing-window state (docs/PHASE5.md "registry"
//! module; itrat-console/13 D12.3).
//!
//! ## One operator, two devices
//!
//! D12.3 originally specified a single device row ("relay-enforced, one row;
//! `POST /relay/v1/pair` returns 409 `device_exists` while a row is active").
//! That is now relaxed to exactly one PHONE and one WATCH, because the wrist
//! is a second pager surface for the same operator, not a second tenant: both
//! read the same relay-computed exception slice, and each signs its own kills
//! with its own key. Everything else about the original stance is preserved:
//!
//! * At most one device PER KIND, and that is enforced by the schema itself
//!   (`UNIQUE INDEX device_kind_unique`), not only by the application checks
//!   that guard the races. The old code enforced its singleton in Rust alone.
//! * A pairing window cannot be armed for a kind whose slot is already full
//!   ("the desktop shows Disconnect instead").
//! * The public pairing route stays dark: no-window, expired-window and
//!   wrong-code remain one indistinguishable [`RegistryError::WindowNotOpen`].
//!
//! ## The kind is bound to the CODE, never claimed by the device
//!
//! [`Registry::arm_pairing_window`] takes the [`DeviceKind`] the desktop minted
//! that code for, and [`Registry::check_pairing_code`] returns the kind the
//! matched code was armed for. The redeeming device's self-reported `platform`
//! string is therefore never consulted when choosing a slot: a device cannot
//! take the watch slot by calling itself a watch. The desktop is the authority
//! that admits devices (D12.2), and this is what makes that true in code.
//!
//! One `rusqlite` connection behind a `Mutex` (this crate serves concurrent
//! axum requests, unlike `genaryx_core::Store`'s single-threaded ingest
//! caller, so the connection needs its own synchronization here).

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
    /// A device of this kind is already paired -- `POST /relay/v1/pair` and
    /// `POST /admin/pairing-window` both fail closed with this (mapped to
    /// HTTP 409 by their handlers). Note this is now per-kind: pairing a
    /// watch while a phone is paired is the normal path, not a conflict.
    #[error("a device of this kind is already paired")]
    DeviceExists,
    /// No pairing window is open for any kind, every open window expired, or
    /// the presented code matches none of them. Deliberately one variant for
    /// all three (see [`Registry::check_pairing_code`]'s doc): the public
    /// pairing route stays "dark" outside a window (D12.3), so a caller must
    /// not be able to distinguish "wrong code" from "no window" from the
    /// response shape.
    #[error("no matching open pairing window")]
    WindowNotOpen,
    /// A `kind` string read back from the database is not one this build
    /// knows. Only reachable if a newer relay wrote a kind this binary does
    /// not have, or the file was edited by hand; fail closed rather than
    /// guess which slot the row occupies.
    #[error("unknown device kind in registry: {0}")]
    UnknownKind(String),
    /// The registry file was written by a newer relay. Refuse to touch it
    /// rather than operate on a schema this build does not understand: a
    /// rollback must fail loudly, not half-work.
    #[error(
        "registry schema version {found} is newer than this build supports ({supported}); \
         this file was written by a newer relay"
    )]
    SchemaTooNew { found: i64, supported: i64 },
    /// The same pairing code hash was offered for a second slot. Refused at
    /// arm time so a slot is never chosen by whichever row a query happened
    /// to return first.
    #[error("that pairing code is already armed for the other device slot")]
    DuplicateCode,
}

/// Which pager surface a paired device is. Closed set: the schema carries a
/// `CHECK (kind IN ('phone','watch'))` so an unknown value cannot be stored,
/// and [`DeviceKind::parse`] is the only way one is read back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    /// The iPhone running TokenFuse Pocket: scans the desktop QR, and is the
    /// device that hands the watch its own code over WatchConnectivity.
    Phone,
    /// The Apple Watch: same exception slice, its own signing key, its own
    /// token, revocable on its own.
    Watch,
}

impl DeviceKind {
    /// The wire and storage spelling. Used in the QR's query parameters, the
    /// admin JSON and the `device`/`pairing_window` tables alike, so there is
    /// exactly one spelling of these values in the system.
    pub fn as_str(self) -> &'static str {
        match self {
            DeviceKind::Phone => "phone",
            DeviceKind::Watch => "watch",
        }
    }

    /// Parse the storage/wire spelling. Returns `None` for anything else:
    /// callers turn that into a 400 (request) or [`RegistryError::UnknownKind`]
    /// (stored row), never into a default kind.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "phone" => Some(DeviceKind::Phone),
            "watch" => Some(DeviceKind::Watch),
            _ => None,
        }
    }

    /// Every kind, for callers that need to report on all slots (the admin
    /// device view) without hardcoding the list a second time.
    pub const ALL: [DeviceKind; 2] = [DeviceKind::Phone, DeviceKind::Watch];
}

/// A paired device row, as stored (includes the Cloud-issued `device_token`
/// used to authenticate that device's calls to the relay's own endpoints).
#[derive(Debug, Clone)]
pub struct DeviceRow {
    pub device_id: String,
    pub kind: DeviceKind,
    pub name: String,
    pub platform: String,
    pub org: String,
    pub role: String,
    pub device_token: String,
    pub paired_at_unix: i64,
    pub last_seen_unix: i64,
    pub apns_token: Option<String>,
}

/// One open pairing window, as an operator sees it. Carries no secret: the
/// code hash stays inside the registry.
#[derive(Debug, Clone)]
pub struct PairingWindowState {
    pub kind: DeviceKind,
    pub expires_unix: i64,
    /// How many wrong codes have been presented since this window was armed.
    /// Purely observational (see [`MIGRATION_V3`]'s doc): nothing in the relay
    /// acts on it, because an unauthenticated caller can inflate it at will.
    /// A nonzero value on a route that is otherwise silent means someone is
    /// probing, and the operator decides what to do about that.
    pub failed_attempts: i64,
}

/// Fields needed to insert a freshly paired device (mirrors the Cloud's own
/// `PairResponse`, `cloud_rest.rs`, plus the platform/name the device sent).
/// Deliberately carries no `kind`: the slot comes from the redeemed code, and
/// [`Registry::insert_paired_device`] takes it as a separate argument so a
/// caller cannot accidentally pass the device's self-reported platform here.
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

/// The original single-device shape. Still applied first so an existing file
/// upgrades along the same path a fresh one is built on, then V2 reshapes it.
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

/// Phone + watch. Rebuilds both tables rather than `ALTER`ing, because the
/// point of the change is the new CHECK/UNIQUE constraints, and SQLite cannot
/// add either to an existing table.
///
/// Any device already paired under V1 was necessarily an iPhone (the watch had
/// no relay client at all), so it migrates into the phone slot. Any pairing
/// window in flight is dropped: it belongs to no kind, re-arming is one click
/// on the desktop, and inventing a kind for it would be exactly the guess this
/// module refuses to make elsewhere.
const MIGRATION_V2: &str = "
CREATE TABLE device_v2 (
    device_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('phone','watch')),
    name TEXT NOT NULL,
    platform TEXT NOT NULL,
    org TEXT NOT NULL,
    role TEXT NOT NULL,
    device_token TEXT NOT NULL,
    paired_at_unix INTEGER NOT NULL,
    last_seen_unix INTEGER NOT NULL,
    apns_token TEXT
);

INSERT INTO device_v2 (device_id, kind, name, platform, org, role, device_token,
                       paired_at_unix, last_seen_unix, apns_token)
SELECT device_id, 'phone', name, platform, org, role, device_token,
       paired_at_unix, last_seen_unix, apns_token
FROM device;

DROP TABLE device;
ALTER TABLE device_v2 RENAME TO device;
CREATE UNIQUE INDEX device_kind_unique ON device(kind);

DROP TABLE pairing_window;
CREATE TABLE pairing_window (
    kind TEXT PRIMARY KEY CHECK (kind IN ('phone','watch')),
    code_sha256 TEXT NOT NULL,
    expires_unix INTEGER NOT NULL
);
";

/// Count failed redemptions against each open window, so an operator can SEE
/// someone probing a route that is otherwise completely silent.
///
/// ## Why this counter drives nothing automatically
///
/// The obvious next step, closing the window after N failures, was considered
/// and deliberately rejected. `POST /relay/v1/pair` is pre-auth by design, so
/// anyone who can merely reach the listener can advance this counter for free.
/// Any state an unauthenticated caller can advance AND which automatically
/// reduces availability is a denial-of-service primitive, whatever threshold
/// you pick. It would be worse here than in general: the phone's window is
/// live for the seconds between arming and scanning, but the watch's window
/// waits minutes on a WatchConnectivity handoff, so a single host inside the
/// existing per-IP allowance could kill every watch window ever armed,
/// indefinitely, without even learning whether one existed.
///
/// And the guessing attack it would defend against does not reach: the code is
/// 8 chars over a 32-symbol alphabet (2^40), the route is rate limited and the
/// window is capped at 900s. Partial-knowledge guessing is the one case where
/// burning would genuinely help, and there is no channel for it: a QR is
/// Reed-Solomon coded so it decodes fully or not at all, the WatchConnectivity
/// payload carries the whole code or nothing, the relay stores only the hash,
/// and the desktop renders the link as a QR and never as selectable text.
///
/// So the counter exists purely to be LOOKED AT. Inflating it advertises the
/// attacker on a dead-silent endpoint; closing the window stays a human
/// decision taken through the authenticated loopback admin API. That also
/// matches the product's own grammar, where the pager reports and the human
/// acts.
const MIGRATION_V3: &str = "
DROP TABLE pairing_window;
CREATE TABLE pairing_window (
    kind TEXT PRIMARY KEY CHECK (kind IN ('phone','watch')),
    code_sha256 TEXT NOT NULL,
    expires_unix INTEGER NOT NULL,
    failed_attempts INTEGER NOT NULL DEFAULT 0
);
";

/// Schema version this build writes. Bumped with every migration added above.
const SCHEMA_VERSION: i64 = 3;

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
        Self::migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Apply every migration the file has not seen yet, tracked by SQLite's
    /// own `user_version`. A V1 file predates the counter and reads back 0,
    /// which is exactly right: it needs V2 and nothing else, and V1's
    /// `IF NOT EXISTS` statements are harmless to re-run on it.
    ///
    /// ## The version bump is INSIDE the transaction, and must stay there
    ///
    /// SQLite writes `user_version` transactionally, so stamping it in the same
    /// batch as the schema change makes "migrated" and "recorded as migrated"
    /// one atomic fact. An earlier revision bumped it in a separate statement
    /// after the `COMMIT`, which is a crash window with a nasty shape: the file
    /// is V2 but still reads version 0, so the next open re-runs V2 against an
    /// already-V2 schema. With both slots filled that re-run copies BOTH rows
    /// as `'phone'` and dies on `UNIQUE constraint failed: device.kind`, so
    /// `Registry::open` returns `Err` and the relay refuses to start on every
    /// subsequent boot until someone hand-edits the pragma. With only a watch
    /// paired it is worse than an error: the re-run SUCCEEDS and silently
    /// relabels that watch as the phone, which is exactly the guess this
    /// module's doc says it never makes. Reproduced against real files before
    /// the fix; covered by `a_crash_before_the_version_bump_does_not_wedge_the_relay`.
    fn migrate(conn: &Connection) -> Result<(), RegistryError> {
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        // Refuse a file written by a NEWER build rather than operating on a
        // schema this binary does not understand. Rolling a relay back must
        // fail loudly, not half-work.
        if version > SCHEMA_VERSION {
            return Err(RegistryError::SchemaTooNew {
                found: version,
                supported: SCHEMA_VERSION,
            });
        }
        if version < 1 {
            conn.execute_batch(&format!(
                "BEGIN; {MIGRATION_V1} PRAGMA user_version = 1; COMMIT;"
            ))?;
        }
        if version < 2 {
            // A file can be V2-SHAPED while still reading version 0, if it was
            // migrated by the earlier build that stamped the version outside
            // the transaction and was interrupted in between. Rebuilding the
            // table again is what wedges it (see the doc above), so recognise
            // the shape and just stamp it.
            if Self::has_column(conn, "device", "kind")? {
                eprintln!(
                    "genaryx-relay: registry: schema is already V2-shaped but unstamped \
                     (interrupted migration); recording the version without rebuilding"
                );
                conn.pragma_update(None, "user_version", 2)?;
            } else {
                conn.execute_batch(&format!(
                    "BEGIN; {MIGRATION_V2} PRAGMA user_version = 2; COMMIT;"
                ))?;
            }
        }
        if version < 3 {
            // `pairing_window` holds only ephemeral state, so recreating it is
            // cheap and honest: any in-flight window is dropped and re-arming
            // is one click, the same trade V2 already made.
            conn.execute_batch(&format!(
                "BEGIN; {MIGRATION_V3} PRAGMA user_version = 3; COMMIT;"
            ))?;
        }
        Ok(())
    }

    /// Does `table` already have `column`? Used to tell a genuinely
    /// pre-migration file from one that was migrated but never stamped.
    fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, RegistryError> {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == column {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Every paired device, in a stable order (phone before watch) so the
    /// admin view and the startup log do not reshuffle between reads.
    pub fn devices(&self) -> Result<Vec<DeviceRow>, RegistryError> {
        let conn = self.conn.lock().expect("registry mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT device_id, kind, name, platform, org, role, device_token, \
                    paired_at_unix, last_seen_unix, apns_token \
             FROM device ORDER BY kind",
        )?;
        let rows = stmt.query_map([], row_to_device)?;
        let mut out = Vec::new();
        for row in rows {
            // Skip and warn on a row this build cannot understand, rather than
            // failing the whole enumeration. `verify_bearer` runs through here
            // on every authenticated request, so one unreadable row must not
            // 500 both devices out of the pager. Slot SELECTION still fails
            // closed: `device_of_kind` asks for one kind and propagates the
            // error, so nothing lands in a slot by guess.
            match row? {
                Ok(device) => out.push(device),
                Err(e) => eprintln!("genaryx-relay: registry: skipping unreadable device row: {e}"),
            }
        }
        Ok(out)
    }

    /// The device occupying `kind`'s slot, if any.
    pub fn device_of_kind(&self, kind: DeviceKind) -> Result<Option<DeviceRow>, RegistryError> {
        let conn = self.conn.lock().expect("registry mutex poisoned");
        conn.query_row(
            "SELECT device_id, kind, name, platform, org, role, device_token, \
                    paired_at_unix, last_seen_unix, apns_token \
             FROM device WHERE kind = ?1",
            params![kind.as_str()],
            row_to_device,
        )
        .optional()?
        .transpose()
    }

    pub fn has_device_of_kind(&self, kind: DeviceKind) -> Result<bool, RegistryError> {
        Ok(self.device_of_kind(kind)?.is_some())
    }

    /// Is anything at all paired? Used for the startup log and for the
    /// desktop's "is this relay claimed" glance, never as a pairing gate:
    /// gating is per-kind now.
    pub fn has_any_device(&self) -> Result<bool, RegistryError> {
        let conn = self.conn.lock().expect("registry mutex poisoned");
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM device", [], |r| r.get(0))?;
        Ok(count > 0)
    }

    /// Arm the pairing window for one kind: refuses with
    /// [`RegistryError::DeviceExists`] while a device of THAT kind is paired
    /// (D12.3: "the pairing window cannot even be armed while paired -- the
    /// desktop shows Disconnect instead"), else (re)writes that kind's row.
    ///
    /// Arming phone and watch is two calls, and the desktop makes both when it
    /// builds a two-code QR. They are independent: a phone already paired does
    /// not stop a watch window from opening, which is the whole point of the
    /// two-slot registry.
    pub fn arm_pairing_window(
        &self,
        kind: DeviceKind,
        code_sha256: &str,
        expires_unix: i64,
    ) -> Result<(), RegistryError> {
        let mut conn = self.conn.lock().expect("registry mutex poisoned");
        let tx = conn.transaction()?;
        let paired: i64 = tx.query_row(
            "SELECT COUNT(*) FROM device WHERE kind = ?1",
            params![kind.as_str()],
            |r| r.get(0),
        )?;
        if paired > 0 {
            return Err(RegistryError::DeviceExists);
        }
        // Refuse the same code hash on two slots. A desktop that minted one
        // code and armed both windows with it would make the slot a coin
        // flip, and the caller would have no way to know which device it just
        // admitted. Cheap to check here, impossible to untangle later.
        let clash: i64 = tx.query_row(
            "SELECT COUNT(*) FROM pairing_window WHERE code_sha256 = ?1 AND kind <> ?2",
            params![code_sha256, kind.as_str()],
            |r| r.get(0),
        )?;
        if clash > 0 {
            return Err(RegistryError::DuplicateCode);
        }
        tx.execute(
            "INSERT INTO pairing_window (kind, code_sha256, expires_unix) VALUES (?1, ?2, ?3) \
             ON CONFLICT(kind) DO UPDATE SET code_sha256 = excluded.code_sha256, \
                                             expires_unix = excluded.expires_unix",
            params![kind.as_str(), code_sha256, expires_unix],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Peek-only check across every open window: does `code` match one, and if
    /// so which kind was it armed for? Does not consume the window -- a
    /// Cloud-side rejection of the same code must leave it intact for a retry
    /// within its TTL (D12.2 step 8 closes the window only on success).
    ///
    /// Every open row is compared, with no early exit on the first match or
    /// mismatch, so the work done is a function of how many windows are open
    /// and not of which one (if any) matched. Expiry is folded into the same
    /// accumulator for the same reason.
    pub fn check_pairing_code(
        &self,
        code: &str,
        now_unix: i64,
    ) -> Result<DeviceKind, RegistryError> {
        let conn = self.conn.lock().expect("registry mutex poisoned");
        // ORDER BY so that if two windows ever carried the SAME hash, the slot
        // chosen is deterministic rather than whatever SQLite returned first.
        // Arming already refuses a duplicate hash (see `arm_pairing_window`),
        // so this is the second lock on the same door, not the only one.
        let mut stmt = conn
            .prepare("SELECT kind, code_sha256, expires_unix FROM pairing_window ORDER BY kind")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;

        let presented = sha256_hex(code.as_bytes());
        let mut matched: Option<DeviceKind> = None;
        for row in rows {
            let (raw_kind, code_sha256, expires_unix) = row?;
            let hit = constant_time_eq(presented.as_bytes(), code_sha256.as_bytes());
            let live = now_unix < expires_unix;
            if hit && live {
                // Not a short circuit: the loop still visits the remaining
                // rows, it just records the first live hit it saw.
                let kind = DeviceKind::parse(&raw_kind)
                    .ok_or_else(|| RegistryError::UnknownKind(raw_kind.clone()))?;
                matched.get_or_insert(kind);
            }
        }
        drop(stmt);

        if matched.is_none() {
            // A wrong code names no slot, so every LIVE window counts it. See
            // MIGRATION_V3's doc for why this counter never closes anything by
            // itself. Done under the same lock, after the constant-work loop,
            // so it adds no oracle: whether the code hit or missed is already
            // in the HTTP response either way.
            conn.execute(
                "UPDATE pairing_window SET failed_attempts = failed_attempts + 1 \
                 WHERE expires_unix > ?1",
                params![now_unix],
            )?;
        }
        matched.ok_or(RegistryError::WindowNotOpen)
    }

    /// Every open window's observable state, for the desktop's Pocket panel.
    /// Deliberately never includes `code_sha256`: the panel needs to show that
    /// a window is armed, when it closes and whether anyone is probing it, and
    /// nothing about the secret itself.
    pub fn pairing_window_states(&self) -> Result<Vec<PairingWindowState>, RegistryError> {
        let conn = self.conn.lock().expect("registry mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT kind, expires_unix, failed_attempts FROM pairing_window ORDER BY kind",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (raw_kind, expires_unix, failed_attempts) = row?;
            let Some(kind) = DeviceKind::parse(&raw_kind) else {
                eprintln!("genaryx-relay: registry: skipping window of unknown kind {raw_kind}");
                continue;
            };
            out.push(PairingWindowState {
                kind,
                expires_unix,
                failed_attempts,
            });
        }
        Ok(out)
    }

    /// Insert the newly paired device into `kind`'s slot and close that kind's
    /// pairing window, in one transaction (re-checks the slot is still empty,
    /// closing a race between two devices redeeming the same window). Called
    /// only AFTER the code has been redeemed at the Cloud (`pairing.rs`), so a
    /// [`RegistryError::DeviceExists`] here means a concurrent pairing won the
    /// race: the caller already has a Cloud-registered device+token for a slot
    /// it will not get, which `pairing.rs` logs (Cloud-side revocation of that
    /// orphan is a later PR, same as Disconnect's).
    ///
    /// `kind` must be the value [`Registry::check_pairing_code`] returned for
    /// the code being redeemed, never anything derived from `new.platform`.
    pub fn insert_paired_device(
        &self,
        kind: DeviceKind,
        new: NewDevice,
    ) -> Result<(), RegistryError> {
        let mut conn = self.conn.lock().expect("registry mutex poisoned");
        let tx = conn.transaction()?;
        let paired: i64 = tx.query_row(
            "SELECT COUNT(*) FROM device WHERE kind = ?1",
            params![kind.as_str()],
            |r| r.get(0),
        )?;
        if paired > 0 {
            return Err(RegistryError::DeviceExists);
        }
        tx.execute(
            "INSERT INTO device (device_id, kind, name, platform, org, role, device_token, \
                                  paired_at_unix, last_seen_unix, apns_token) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, NULL)",
            params![
                new.device_id,
                kind.as_str(),
                new.name,
                new.platform,
                new.org,
                new.role,
                new.device_token,
                new.paired_at_unix,
            ],
        )?;
        tx.execute(
            "DELETE FROM pairing_window WHERE kind = ?1",
            params![kind.as_str()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Delete paired devices (+ their APNs tokens, same rows) and the matching
    /// pairing windows. `Some(kind)` revokes just that surface, which is why
    /// the two devices have separate tokens in the first place: losing a watch
    /// must not force the phone to re-pair. `None` revokes everything.
    /// Returns how many device rows were removed.
    ///
    /// Upstream Cloud-side revocation of the device token is a later PR
    /// (itrat-console/13 D12.3 R5: no `DELETE /v1/devices/{id}` yet) --
    /// noted here, not silently implied.
    pub fn disconnect(&self, kind: Option<DeviceKind>) -> Result<usize, RegistryError> {
        let conn = self.conn.lock().expect("registry mutex poisoned");
        let deleted = match kind {
            Some(k) => {
                let n = conn.execute("DELETE FROM device WHERE kind = ?1", params![k.as_str()])?;
                conn.execute(
                    "DELETE FROM pairing_window WHERE kind = ?1",
                    params![k.as_str()],
                )?;
                n
            }
            None => {
                let n = conn.execute("DELETE FROM device", [])?;
                conn.execute("DELETE FROM pairing_window", [])?;
                n
            }
        };
        Ok(deleted)
    }

    /// Constant-time bearer check across every paired device (D12.2c step 3:
    /// "token matches (constant-time)"). `None` if nothing is paired or the
    /// token matches no device -- callers must treat both identically (401),
    /// never distinguish them in the response.
    ///
    /// Every row is compared with no early exit, so which device a token
    /// belongs to (or that it belongs to none) is not observable from how long
    /// the check took.
    pub fn verify_bearer(&self, token: &str) -> Result<Option<DeviceRow>, RegistryError> {
        let mut matched: Option<DeviceRow> = None;
        for device in self.devices()? {
            if constant_time_eq(device.device_token.as_bytes(), token.as_bytes()) {
                matched.get_or_insert(device);
            }
        }
        Ok(matched)
    }

    /// Update `last_seen_unix` for one device (best-effort freshness for the
    /// desktop's Pocket panel device view).
    pub fn touch_last_seen(&self, device_id: &str, now_unix: i64) -> Result<(), RegistryError> {
        let conn = self.conn.lock().expect("registry mutex poisoned");
        conn.execute(
            "UPDATE device SET last_seen_unix = ?1 WHERE device_id = ?2",
            params![now_unix, device_id],
        )?;
        Ok(())
    }
}

/// Maps a row to a [`DeviceRow`]. The stored `kind` is parsed rather than
/// trusted, so a value this build does not know fails closed as
/// [`RegistryError::UnknownKind`] instead of silently landing in a slot. The
/// nested `Result` is unwrapped by the callers (`??` / `.transpose()`) because
/// `rusqlite`'s row mapper can only return `rusqlite::Error`.
#[allow(clippy::type_complexity)]
fn row_to_device(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<DeviceRow, RegistryError>> {
    let raw_kind: String = r.get(1)?;
    let Some(kind) = DeviceKind::parse(&raw_kind) else {
        return Ok(Err(RegistryError::UnknownKind(raw_kind)));
    };
    Ok(Ok(DeviceRow {
        device_id: r.get(0)?,
        kind,
        name: r.get(2)?,
        platform: r.get(3)?,
        org: r.get(4)?,
        role: r.get(5)?,
        device_token: r.get(6)?,
        paired_at_unix: r.get(7)?,
        last_seen_unix: r.get(8)?,
        apns_token: r.get(9)?,
    }))
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
        assert!(!reg.has_any_device().unwrap());
        assert!(reg.devices().unwrap().is_empty());
        for kind in DeviceKind::ALL {
            assert!(reg.device_of_kind(kind).unwrap().is_none());
        }
    }

    #[test]
    fn arm_check_and_insert_happy_path() {
        let reg = Registry::open_in_memory().unwrap();
        let code_sha256 = sha256_hex(b"ABCD1234");
        reg.arm_pairing_window(DeviceKind::Phone, &code_sha256, 2_000)
            .unwrap();

        let kind = reg
            .check_pairing_code("ABCD1234", 1_500)
            .expect("code matches, window open");
        assert_eq!(kind, DeviceKind::Phone);

        reg.insert_paired_device(kind, new_device("dev-1")).unwrap();
        let dev = reg
            .device_of_kind(DeviceKind::Phone)
            .unwrap()
            .expect("phone now paired");
        assert_eq!(dev.device_id, "dev-1");
        assert_eq!(dev.device_token, "token-dev-1");
        assert_eq!(dev.kind, DeviceKind::Phone);
        assert!(dev.apns_token.is_none());

        // Window closed by a successful insert.
        assert!(matches!(
            reg.check_pairing_code("ABCD1234", 1_600),
            Err(RegistryError::WindowNotOpen)
        ));
    }

    #[test]
    fn phone_and_watch_pair_independently() {
        let reg = Registry::open_in_memory().unwrap();
        reg.arm_pairing_window(DeviceKind::Phone, &sha256_hex(b"PHONECODE"), 2_000)
            .unwrap();
        reg.arm_pairing_window(DeviceKind::Watch, &sha256_hex(b"WATCHCODE"), 2_000)
            .unwrap();

        // Each code resolves to its own slot, and only its own slot.
        assert_eq!(
            reg.check_pairing_code("PHONECODE", 1_500).unwrap(),
            DeviceKind::Phone
        );
        assert_eq!(
            reg.check_pairing_code("WATCHCODE", 1_500).unwrap(),
            DeviceKind::Watch
        );

        reg.insert_paired_device(DeviceKind::Phone, new_device("phone-1"))
            .unwrap();
        // A paired phone must not block the watch: this is the whole change.
        reg.insert_paired_device(DeviceKind::Watch, new_device("watch-1"))
            .unwrap();

        let devices = reg.devices().unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].kind, DeviceKind::Phone); // ORDER BY kind: phone, watch
        assert_eq!(devices[1].kind, DeviceKind::Watch);
    }

    #[test]
    fn a_second_device_of_the_same_kind_is_refused() {
        let reg = Registry::open_in_memory().unwrap();
        reg.insert_paired_device(DeviceKind::Phone, new_device("phone-1"))
            .unwrap();

        let err = reg
            .arm_pairing_window(DeviceKind::Phone, &sha256_hex(b"WHATEVER"), 9_999)
            .unwrap_err();
        assert!(matches!(err, RegistryError::DeviceExists));

        let err = reg
            .insert_paired_device(DeviceKind::Phone, new_device("phone-2"))
            .unwrap_err();
        assert!(matches!(err, RegistryError::DeviceExists));
        // Still the FIRST phone, not overwritten.
        assert_eq!(
            reg.device_of_kind(DeviceKind::Phone)
                .unwrap()
                .unwrap()
                .device_id,
            "phone-1"
        );

        // ...while the watch slot is untouched and still armable.
        reg.arm_pairing_window(DeviceKind::Watch, &sha256_hex(b"WATCHCODE"), 9_999)
            .unwrap();
    }

    #[test]
    fn the_unique_index_backstops_the_application_check() {
        // Even if the COUNT guard above were bypassed, the schema itself
        // refuses a second row of the same kind.
        let reg = Registry::open_in_memory().unwrap();
        reg.insert_paired_device(DeviceKind::Watch, new_device("watch-1"))
            .unwrap();
        let conn = reg.conn.lock().unwrap();
        let raw = conn.execute(
            "INSERT INTO device (device_id, kind, name, platform, org, role, device_token, \
                                  paired_at_unix, last_seen_unix, apns_token) \
             VALUES ('watch-2', 'watch', 'w', 'watchos', 'acme', 'admin', 't', 1, 1, NULL)",
            [],
        );
        assert!(raw.is_err(), "UNIQUE INDEX device_kind_unique must refuse");
    }

    #[test]
    fn an_unknown_kind_cannot_be_stored() {
        let reg = Registry::open_in_memory().unwrap();
        let conn = reg.conn.lock().unwrap();
        let raw = conn.execute(
            "INSERT INTO device (device_id, kind, name, platform, org, role, device_token, \
                                  paired_at_unix, last_seen_unix, apns_token) \
             VALUES ('x', 'laptop', 'l', 'macos', 'acme', 'admin', 't', 1, 1, NULL)",
            [],
        );
        assert!(
            raw.is_err(),
            "CHECK (kind IN ('phone','watch')) must refuse"
        );
    }

    #[test]
    fn wrong_code_and_expired_window_both_fail_the_same_way() {
        let reg = Registry::open_in_memory().unwrap();
        reg.arm_pairing_window(DeviceKind::Phone, &sha256_hex(b"RIGHTCODE"), 2_000)
            .unwrap();

        let wrong = reg.check_pairing_code("WRONGCODE", 1_500).unwrap_err();
        assert!(matches!(wrong, RegistryError::WindowNotOpen));

        let expired = reg.check_pairing_code("RIGHTCODE", 2_500).unwrap_err();
        assert!(matches!(expired, RegistryError::WindowNotOpen));
    }

    #[test]
    fn an_expired_window_of_one_kind_does_not_shadow_a_live_one() {
        let reg = Registry::open_in_memory().unwrap();
        reg.arm_pairing_window(DeviceKind::Phone, &sha256_hex(b"OLDCODE"), 1_000)
            .unwrap();
        reg.arm_pairing_window(DeviceKind::Watch, &sha256_hex(b"NEWCODE"), 9_999)
            .unwrap();

        assert!(matches!(
            reg.check_pairing_code("OLDCODE", 5_000),
            Err(RegistryError::WindowNotOpen)
        ));
        assert_eq!(
            reg.check_pairing_code("NEWCODE", 5_000).unwrap(),
            DeviceKind::Watch
        );
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
    fn verify_bearer_matches_only_the_right_token_across_both_devices() {
        let reg = Registry::open_in_memory().unwrap();
        reg.insert_paired_device(DeviceKind::Phone, new_device("phone-1"))
            .unwrap();
        reg.insert_paired_device(DeviceKind::Watch, new_device("watch-1"))
            .unwrap();

        let phone = reg.verify_bearer("token-phone-1").unwrap().expect("phone");
        assert_eq!(phone.kind, DeviceKind::Phone);
        let watch = reg.verify_bearer("token-watch-1").unwrap().expect("watch");
        assert_eq!(watch.kind, DeviceKind::Watch);

        assert!(reg.verify_bearer("token-phone-1x").unwrap().is_none());
        assert!(reg.verify_bearer("").unwrap().is_none());
    }

    #[test]
    fn disconnecting_one_kind_leaves_the_other_paired() {
        let reg = Registry::open_in_memory().unwrap();
        reg.insert_paired_device(DeviceKind::Phone, new_device("phone-1"))
            .unwrap();
        reg.insert_paired_device(DeviceKind::Watch, new_device("watch-1"))
            .unwrap();

        assert_eq!(reg.disconnect(Some(DeviceKind::Watch)).unwrap(), 1);
        assert!(reg.device_of_kind(DeviceKind::Watch).unwrap().is_none());
        assert!(
            reg.device_of_kind(DeviceKind::Phone).unwrap().is_some(),
            "losing the watch must not force the phone to re-pair"
        );
        // The freed slot can be armed and refilled on its own.
        reg.arm_pairing_window(DeviceKind::Watch, &sha256_hex(b"AGAIN"), 9_999)
            .unwrap();
        let kind = reg.check_pairing_code("AGAIN", 1_000).unwrap();
        reg.insert_paired_device(kind, new_device("watch-2"))
            .unwrap();
        assert_eq!(
            reg.device_of_kind(DeviceKind::Watch)
                .unwrap()
                .unwrap()
                .device_id,
            "watch-2"
        );
    }

    #[test]
    fn disconnect_all_clears_both_devices_and_every_window() {
        let reg = Registry::open_in_memory().unwrap();
        reg.insert_paired_device(DeviceKind::Phone, new_device("phone-1"))
            .unwrap();
        reg.insert_paired_device(DeviceKind::Watch, new_device("watch-1"))
            .unwrap();

        assert_eq!(reg.disconnect(None).unwrap(), 2);
        assert!(!reg.has_any_device().unwrap());
        assert_eq!(reg.disconnect(None).unwrap(), 0, "nothing left");
    }

    #[test]
    fn touch_last_seen_updates_only_that_device() {
        let reg = Registry::open_in_memory().unwrap();
        reg.insert_paired_device(DeviceKind::Phone, new_device("phone-1"))
            .unwrap();
        reg.insert_paired_device(DeviceKind::Watch, new_device("watch-1"))
            .unwrap();

        reg.touch_last_seen("watch-1", 5_000).unwrap();
        assert_eq!(
            reg.device_of_kind(DeviceKind::Watch)
                .unwrap()
                .unwrap()
                .last_seen_unix,
            5_000
        );
        assert_eq!(
            reg.device_of_kind(DeviceKind::Phone)
                .unwrap()
                .unwrap()
                .last_seen_unix,
            1_000
        );
    }

    #[test]
    fn a_v1_file_upgrades_its_lone_device_into_the_phone_slot() {
        // Build a V1-shaped database by hand, with a device row and an
        // in-flight pairing window, then open it through Registry::migrate.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATION_V1).unwrap();
        conn.execute(
            "INSERT INTO device (device_id, name, platform, org, role, device_token, \
                                  paired_at_unix, last_seen_unix, apns_token) \
             VALUES ('legacy-1', 'iPhone', 'ios', 'acme', 'admin', 'legacy-token', 7, 8, 'apns')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO pairing_window (id, code_sha256, expires_unix) VALUES (1, 'abc', 9999)",
            [],
        )
        .unwrap();

        let reg = Registry::from_connection(conn).expect("V1 file migrates");

        let phone = reg
            .device_of_kind(DeviceKind::Phone)
            .unwrap()
            .expect("the V1 device lands in the phone slot");
        assert_eq!(phone.device_id, "legacy-1");
        assert_eq!(phone.device_token, "legacy-token");
        assert_eq!(phone.paired_at_unix, 7);
        assert_eq!(phone.last_seen_unix, 8);
        assert_eq!(phone.apns_token.as_deref(), Some("apns"));
        assert!(reg.device_of_kind(DeviceKind::Watch).unwrap().is_none());

        // The kindless in-flight window was dropped, not guessed at.
        assert!(matches!(
            reg.check_pairing_code("anything", 1),
            Err(RegistryError::WindowNotOpen)
        ));
        // ...and the freed watch slot is immediately usable.
        reg.arm_pairing_window(DeviceKind::Watch, &sha256_hex(b"W"), 9_999)
            .unwrap();
    }

    #[test]
    fn migration_is_idempotent_across_reopens() {
        let dir =
            std::env::temp_dir().join(format!("genaryx-registry-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("registry.sqlite3");
        let _ = std::fs::remove_file(&path);

        {
            let reg = Registry::open(&path).unwrap();
            reg.insert_paired_device(DeviceKind::Phone, new_device("phone-1"))
                .unwrap();
        }
        {
            let reg = Registry::open(&path).expect("reopening must not re-run V2");
            assert_eq!(
                reg.device_of_kind(DeviceKind::Phone)
                    .unwrap()
                    .unwrap()
                    .device_id,
                "phone-1",
                "a second open must not wipe the paired device"
            );
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn a_crash_before_the_version_bump_does_not_wedge_the_relay() {
        // Reproduces the exact state an interrupted migration leaves behind:
        // the schema is already V2 (both slots filled), but `user_version` was
        // never stamped. Before the fix, reopening re-ran MIGRATION_V2, which
        // copied BOTH rows as 'phone' and died on the UNIQUE index, so the
        // relay refused to start on every boot from then on.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATION_V1).unwrap();
        conn.execute_batch(MIGRATION_V2).unwrap();
        conn.execute(
            "INSERT INTO device (device_id, kind, name, platform, org, role, device_token, \
                                  paired_at_unix, last_seen_unix, apns_token) \
             VALUES ('p1','phone','iPhone','ios','acme','admin','tok-p',1,1,NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO device (device_id, kind, name, platform, org, role, device_token, \
                                  paired_at_unix, last_seen_unix, apns_token) \
             VALUES ('w1','watch','Watch','watchos','acme','admin','tok-w',1,1,NULL)",
            [],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 0).unwrap();

        let reg = Registry::from_connection(conn).expect("an unstamped V2 file must still open");

        // Both devices survived, and the watch was NOT relabelled as a phone.
        assert_eq!(
            reg.device_of_kind(DeviceKind::Phone)
                .unwrap()
                .unwrap()
                .device_id,
            "p1"
        );
        let watch = reg.device_of_kind(DeviceKind::Watch).unwrap().unwrap();
        assert_eq!(watch.device_id, "w1");
        assert_eq!(watch.platform, "watchos");
    }

    #[test]
    fn a_lone_watch_is_never_relabelled_as_the_phone() {
        // The nastier half of the same bug: with only a watch paired, the
        // re-run SUCCEEDED and silently moved it into the phone slot, which
        // inverts per-slot revocation.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATION_V1).unwrap();
        conn.execute_batch(MIGRATION_V2).unwrap();
        conn.execute(
            "INSERT INTO device (device_id, kind, name, platform, org, role, device_token, \
                                  paired_at_unix, last_seen_unix, apns_token) \
             VALUES ('w1','watch','Watch','watchos','acme','admin','tok-w',1,1,NULL)",
            [],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 0).unwrap();

        let reg = Registry::from_connection(conn).expect("opens");
        assert!(
            reg.device_of_kind(DeviceKind::Phone).unwrap().is_none(),
            "the phone slot must stay empty"
        );
        assert_eq!(
            reg.device_of_kind(DeviceKind::Watch)
                .unwrap()
                .unwrap()
                .device_id,
            "w1"
        );
    }

    #[test]
    fn a_file_from_a_newer_build_is_refused_not_half_understood() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATION_V1).unwrap();
        conn.execute_batch(MIGRATION_V2).unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        match Registry::from_connection(conn) {
            Err(RegistryError::SchemaTooNew { found, supported }) => {
                assert_eq!(found, SCHEMA_VERSION + 1);
                assert_eq!(supported, SCHEMA_VERSION);
            }
            Err(other) => panic!("expected SchemaTooNew, got {other:?}"),
            Ok(_) => panic!("expected SchemaTooNew, but the file opened"),
        }
    }

    #[test]
    fn the_same_code_cannot_be_armed_for_both_slots() {
        let reg = Registry::open_in_memory().unwrap();
        let shared = sha256_hex(b"ONECODE");
        reg.arm_pairing_window(DeviceKind::Phone, &shared, 9_999)
            .unwrap();
        let err = reg
            .arm_pairing_window(DeviceKind::Watch, &shared, 9_999)
            .unwrap_err();
        assert!(matches!(err, RegistryError::DuplicateCode));

        // A different code for the other slot is of course still fine.
        reg.arm_pairing_window(DeviceKind::Watch, &sha256_hex(b"OTHERCODE"), 9_999)
            .unwrap();
        assert_eq!(
            reg.check_pairing_code("ONECODE", 1_000).unwrap(),
            DeviceKind::Phone
        );
        assert_eq!(
            reg.check_pairing_code("OTHERCODE", 1_000).unwrap(),
            DeviceKind::Watch
        );
    }

    #[test]
    fn one_unreadable_row_does_not_take_down_the_whole_registry() {
        // verify_bearer runs through devices() on every authenticated request,
        // so a row this build cannot parse must not 500 both devices out.
        let reg = Registry::open_in_memory().unwrap();
        {
            // Stand in for a future build that knows a kind this one does not:
            // rebuild the table without the CHECK, then write both rows.
            let conn = reg.conn.lock().unwrap();
            conn.execute_batch(
                "DROP TABLE device;
                 CREATE TABLE device (
                     device_id TEXT PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL,
                     platform TEXT NOT NULL, org TEXT NOT NULL, role TEXT NOT NULL,
                     device_token TEXT NOT NULL, paired_at_unix INTEGER NOT NULL,
                     last_seen_unix INTEGER NOT NULL, apns_token TEXT);
                 INSERT INTO device VALUES
                   ('phone-1','phone','iPhone','ios','acme','admin','token-phone-1',1,1,NULL),
                   ('t1','tablet','iPad','ipados','acme','admin','tok-t',1,1,NULL);",
            )
            .unwrap();
        }
        let devices = reg
            .devices()
            .expect("enumeration survives the unreadable row");
        assert_eq!(devices.len(), 1, "the tablet row is skipped, not fatal");
        assert!(reg.verify_bearer("token-phone-1").unwrap().is_some());
    }

    #[test]
    fn wrong_codes_are_counted_against_every_live_window() {
        let reg = Registry::open_in_memory().unwrap();
        reg.arm_pairing_window(DeviceKind::Phone, &sha256_hex(b"PHONECODE"), 9_999)
            .unwrap();
        reg.arm_pairing_window(DeviceKind::Watch, &sha256_hex(b"WATCHCODE"), 9_999)
            .unwrap();

        for _ in 0..3 {
            assert!(reg.check_pairing_code("NOPE", 1_000).is_err());
        }
        // A wrong code names no slot, so both live windows count it.
        for w in reg.pairing_window_states().unwrap() {
            assert_eq!(w.failed_attempts, 3, "{:?}", w.kind);
        }
    }

    #[test]
    fn the_failure_counter_never_closes_a_window() {
        // This is the decision, encoded. `POST /relay/v1/pair` is pre-auth, so
        // anyone who can reach the listener can run this counter up for free.
        // If it ever closed the window, that would be an unauthenticated denial
        // of pairing, and the watch's long-lived window would be the easiest
        // thing in the system to keep permanently shut.
        let reg = Registry::open_in_memory().unwrap();
        reg.arm_pairing_window(DeviceKind::Watch, &sha256_hex(b"WATCHCODE"), 9_999)
            .unwrap();

        for _ in 0..500 {
            assert!(reg.check_pairing_code("WRONG", 1_000).is_err());
        }

        // Five hundred wrong guesses later, the real code still works.
        assert_eq!(
            reg.check_pairing_code("WATCHCODE", 1_000).unwrap(),
            DeviceKind::Watch
        );
        let states = reg.pairing_window_states().unwrap();
        assert_eq!(states.len(), 1, "the window is still armed");
        assert_eq!(states[0].failed_attempts, 500, "and the probing is visible");
    }

    #[test]
    fn an_expired_window_stops_accruing_failures() {
        let reg = Registry::open_in_memory().unwrap();
        reg.arm_pairing_window(DeviceKind::Phone, &sha256_hex(b"OLD"), 1_000)
            .unwrap();
        // now_unix is past expiry: the window is dead, so counting against it
        // would only produce noise about a window nobody can use.
        assert!(reg.check_pairing_code("NOPE", 5_000).is_err());
        assert_eq!(
            reg.pairing_window_states().unwrap()[0].failed_attempts,
            0,
            "a closed window must not accrue"
        );
    }

    #[test]
    fn a_correct_code_does_not_count_as_a_failure() {
        let reg = Registry::open_in_memory().unwrap();
        reg.arm_pairing_window(DeviceKind::Phone, &sha256_hex(b"RIGHT"), 9_999)
            .unwrap();
        reg.check_pairing_code("RIGHT", 1_000).unwrap();
        assert_eq!(reg.pairing_window_states().unwrap()[0].failed_attempts, 0);
    }

    #[test]
    fn window_states_carry_no_secret_and_vanish_once_redeemed() {
        let reg = Registry::open_in_memory().unwrap();
        reg.arm_pairing_window(DeviceKind::Phone, &sha256_hex(b"RIGHT"), 9_999)
            .unwrap();
        let states = reg.pairing_window_states().unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].kind, DeviceKind::Phone);
        assert_eq!(states[0].expires_unix, 9_999);
        // The struct has no field that could carry the hash: if someone adds
        // one, this stops compiling rather than quietly leaking it.
        let PairingWindowState {
            kind: _,
            expires_unix: _,
            failed_attempts: _,
        } = states[0].clone();

        let kind = reg.check_pairing_code("RIGHT", 1_000).unwrap();
        reg.insert_paired_device(kind, new_device("phone-1"))
            .unwrap();
        assert!(
            reg.pairing_window_states().unwrap().is_empty(),
            "a redeemed window is closed, so there is nothing left to report"
        );
    }

    #[test]
    fn constant_time_eq_behaves_like_eq_for_correctness() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn device_kind_round_trips_and_rejects_anything_else() {
        for kind in DeviceKind::ALL {
            assert_eq!(DeviceKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(DeviceKind::parse("laptop"), None);
        assert_eq!(DeviceKind::parse("Phone"), None, "spelling is exact");
        assert_eq!(DeviceKind::parse(""), None);
    }
}
