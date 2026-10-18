//! Sign-in throttle for the local operator password (F-017).
//!
//! The password is the break-glass factor, so it is the factor a guessing run
//! goes after. Every failure is counted against two keys, the account it named
//! and the client address it came from, inside a fixed window. Once a key has
//! failed its limit it is held back until the window passes.
//!
//! The table is bounded. A flood of distinct names or addresses cannot grow it
//! past `max_keys`: expired windows go first, then the oldest live one.

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
