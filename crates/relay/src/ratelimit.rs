//! A tiny in-process rate limiter for the public listener's authenticated
//! and pre-auth routes (docs/PHASE5.md "proxy" module: "Rate-limit; never
//! alter a forwarded request." / itrat-console/13 D12.3 R2 mitigations).
//!
//! Fixed-window-per-key, good enough for a single relay process guarding a
//! handful of routes against a misbehaving or malicious caller; not meant to
//! be a general-purpose limiter. Keys are the caller's device id for the
//! mutation pass-through (single device, so this is really "how fast can
//! kill/budget/ack fire") and the caller's IP for the pre-auth pairing route.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct RateLimiter {
    max_per_window: u32,
    window: Duration,
    hits: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    pub fn new(max_per_window: u32, window: Duration) -> Self {
        Self {
            max_per_window,
            window,
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// Record one attempt for `key` and report whether it is within the
    /// limit. `true` = allowed (and counted); `false` = rejected (also not
    /// counted again -- a caller that's already over the limit doesn't get
    /// to extend its own window by hammering harder).
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut hits = self.hits.lock().expect("rate limiter mutex poisoned");
        let entry = hits.entry(key.to_string()).or_default();
        while let Some(&front) = entry.front() {
            if now.duration_since(front) > self.window {
                entry.pop_front();
            } else {
                break;
            }
        }
        if entry.len() as u32 >= self.max_per_window {
            return false;
        }
        entry.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_the_limit_then_rejects() {
        let rl = RateLimiter::new(3, Duration::from_secs(60));
        assert!(rl.check("k"));
        assert!(rl.check("k"));
        assert!(rl.check("k"));
        assert!(!rl.check("k"), "fourth attempt in the window is rejected");
    }

    #[test]
    fn keys_are_independent() {
        let rl = RateLimiter::new(1, Duration::from_secs(60));
        assert!(rl.check("a"));
        assert!(rl.check("b"), "a different key has its own budget");
        assert!(!rl.check("a"));
    }

    #[test]
    fn window_expiry_frees_up_budget() {
        let rl = RateLimiter::new(1, Duration::from_millis(20));
        assert!(rl.check("k"));
        assert!(!rl.check("k"));
        std::thread::sleep(Duration::from_millis(30));
        assert!(rl.check("k"), "window elapsed, budget renewed");
    }
}
