//! Sign-in throttle for the local operator password (F-017).
//!
//! The password is the break-glass factor, so it is the factor a guessing run
//! goes after. Every failure is counted against two keys, the account it named
//! and the client address it came from, inside a fixed window. Once a key has
//! failed its limit it is held back until the window passes.
//!
//! The table is bounded. A flood of distinct names or addresses cannot grow it
//! past `max_keys`: expired windows go first, then the oldest live one.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Failures allowed per account name inside one window (F-017).
pub const ACCOUNT_LIMIT: u32 = 10;
/// Failures allowed per client address inside one window. Higher than the
/// account limit, because one address can serve many people behind a proxy.
pub const ADDRESS_LIMIT: u32 = 30;
/// One window: a held key is admitted again once it has passed.
pub const WINDOW: Duration = Duration::from_secs(15 * 60);
/// Keys held at once, per table. This bounds the memory a flood can take.
pub const MAX_KEYS: usize = 4096;
/// Characters of a name kept in its key. A longer name shares a key with its
/// prefix, which only makes its limit stricter, and keeps every key one size.
const MAX_KEY_CHARS: usize = 128;

/// The failures of one key inside its current window.
struct Window {
    started: Instant,
    failures: u32,
}

/// Failed password sign-ins, counted per key inside a fixed window.
pub struct LoginThrottle {
    limit: u32,
    window: Duration,
    max_keys: usize,
    windows: Mutex<HashMap<String, Window>>,
}

impl LoginThrottle {
    pub fn new(limit: u32, window: Duration, max_keys: usize) -> Self {
        Self {
            limit,
            window,
            max_keys,
            windows: Mutex::new(HashMap::new()),
        }
    }

    fn table(&self) -> MutexGuard<'_, HashMap<String, Window>> {
        // A panic while the lock was held leaves the counts as they were.
        // Refusing every later sign-in over that would lock the operator out.
        self.windows.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn is_live(&self, w: &Window, now: Instant) -> bool {
        now.saturating_duration_since(w.started) < self.window
    }

    /// `Ok` while `key` may try again. `Err` carries the time left in its
    /// window once it has used up its failures.
    pub fn check(&self, key: &str, now: Instant) -> Result<(), Duration> {
        let table = self.table();
        match table.get(key) {
            Some(w) if w.failures >= self.limit && self.is_live(w, now) => {
                Err(self.window - now.saturating_duration_since(w.started))
            }
            _ => Ok(()),
        }
    }

    /// Count one failed sign-in against `key`. A key whose window has passed
    /// starts a fresh one.
    pub fn record_failure(&self, key: &str, now: Instant) {
        let mut table = self.table();
        if let Some(w) = table.get_mut(key) {
            if self.is_live(w, now) {
                w.failures = w.failures.saturating_add(1);
            } else {
                *w = Window {
                    started: now,
                    failures: 1,
                };
            }
            return;
        }
        if table.len() >= self.max_keys {
            self.make_room(&mut table, now);
        }
        table.insert(
            key.to_string(),
            Window {
                started: now,
                failures: 1,
            },
        );
    }

    /// Forget the windows that have passed. If the table is still full, forget
    /// the oldest live one, so a flood of names cannot grow it.
    fn make_room(&self, table: &mut HashMap<String, Window>, now: Instant) {
        table.retain(|_, w| self.is_live(w, now));
        if table.len() < self.max_keys {
            return;
        }
        let oldest = table
            .iter()
            .min_by_key(|(_, w)| w.started)
            .map(|(k, _)| k.clone());
        if let Some(key) = oldest {
            table.remove(&key);
        }
    }

    /// A sign-in under `key` succeeded: the key starts from nothing.
    pub fn record_success(&self, key: &str) {
        self.table().remove(key);
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.table().len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The key for one account name or one client address. The value is cut to
/// `MAX_KEY_CHARS`, so a hostile username cannot make a key of any size.
pub fn login_key(kind: &str, value: &str) -> String {
    let short: String = value.chars().take(MAX_KEY_CHARS).collect();
    format!("{kind}:{short}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const WINDOW: Duration = Duration::from_secs(60);

    #[test]
    fn a_key_is_refused_after_its_limit_and_admitted_after_the_window() {
        let t = LoginThrottle::new(3, WINDOW, 16);
        let start = Instant::now();
        for _ in 0..3 {
            assert!(t.check("k", start).is_ok());
            t.record_failure("k", start);
        }
        let wait = t.check("k", start + Duration::from_secs(10)).unwrap_err();
        assert_eq!(wait, Duration::from_secs(50), "the wait is the time left");
        assert!(
            t.check("k", start + WINDOW).is_ok(),
            "admitted at the window's end"
        );
    }

    #[test]
    fn a_success_clears_the_key() {
        let t = LoginThrottle::new(2, WINDOW, 16);
        let now = Instant::now();
        t.record_failure("k", now);
        t.record_success("k");
        t.record_failure("k", now);
        assert!(
            t.check("k", now).is_ok(),
            "one failure after a success is no lockout"
        );
    }

    #[test]
    fn one_key_does_not_throttle_another() {
        let t = LoginThrottle::new(1, WINDOW, 16);
        let now = Instant::now();
        t.record_failure("a", now);
        assert!(t.check("a", now).is_err());
        assert!(t.check("b", now).is_ok());
    }

    #[test]
    fn memory_stays_bounded_under_a_flood_of_distinct_keys() {
        let t = LoginThrottle::new(2, WINDOW, 8);
        let now = Instant::now();
        for i in 0..10_000 {
            t.record_failure(&format!("k{i}"), now);
        }
        assert!(!t.is_empty(), "a flood still leaves its newest keys held");
        assert!(t.len() <= 8, "held {} keys", t.len());
    }

    #[test]
    fn a_hostile_username_is_keyed_in_bounded_space() {
        let huge = "x".repeat(1 << 20);
        let key = login_key("user", &huge);
        assert!(key.len() <= 256, "key is {} bytes", key.len());
        assert_ne!(login_key("user", &huge), login_key("addr", &huge));
    }
}
