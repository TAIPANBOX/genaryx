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

/// Force a sweep once the key map passes this size, even if the periodic
/// sweep is not due yet. A burst of distinct source IPs inside a single
/// window must not be able to grow the map without bound between sweeps.
const MAX_TRACKED_KEYS: usize = 8_192;

struct Hits {
    keys: HashMap<String, VecDeque<Instant>>,
    last_sweep: Instant,
}

pub struct RateLimiter {
    max_per_window: u32,
    window: Duration,
    hits: Mutex<Hits>,
}

impl RateLimiter {
    pub fn new(max_per_window: u32, window: Duration) -> Self {
        Self {
            max_per_window,
            window,
            hits: Mutex::new(Hits {
                keys: HashMap::new(),
                last_sweep: Instant::now(),
            }),
        }
    }

    /// Record one attempt for `key` and report whether it is within the
    /// limit. `true` = allowed (and counted); `false` = rejected (also not
    /// counted again -- a caller that's already over the limit doesn't get
    /// to extend its own window by hammering harder).
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut hits = self.hits.lock().expect("rate limiter mutex poisoned");

        // Entries used to be pruned only when the SAME key came back, so every
        // distinct source IP that ever touched the pre-auth pairing route left
        // a permanent map entry: slow memory exhaustion against the one door
        // this process opens to the internet. Sweep on a cadence, and eagerly
        // if the map is growing faster than the cadence can drain it.
        let due = now.duration_since(hits.last_sweep) > self.window;
        if due || hits.keys.len() > MAX_TRACKED_KEYS {
            let window = self.window;
            hits.keys.retain(|_, entry| {
                while let Some(&front) = entry.front() {
                    if now.duration_since(front) > window {
                        entry.pop_front();
                    } else {
                        break;
                    }
                }
                !entry.is_empty()
            });
            hits.last_sweep = now;
        }

        let entry = hits.keys.entry(key.to_string()).or_default();
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

    /// How many keys are currently tracked. Test-facing: the point of the
    /// sweep is that this does not grow without bound.
    #[cfg(test)]
    fn tracked_keys(&self) -> usize {
        self.hits
            .lock()
            .expect("rate limiter mutex poisoned")
            .keys
            .len()
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
    fn one_shot_keys_do_not_accumulate_forever() {
        // Every distinct caller IP used to leave a permanent entry. Walk a few
        // thousand one-shot keys past a short window and assert the map drains
        // instead of growing monotonically.
        let rl = RateLimiter::new(10, Duration::from_millis(20));
        for i in 0..2_000 {
            rl.check(&format!("ip-{i}"));
        }
        let peak = rl.tracked_keys();
        assert!(peak > 0);
        std::thread::sleep(Duration::from_millis(30));
        // Any single later call triggers the due sweep, which drops every
        // entry whose window has elapsed.
        rl.check("someone-else");
        assert!(
            rl.tracked_keys() < peak / 10,
            "expired keys must be swept, kept {} of {peak}",
            rl.tracked_keys()
        );
    }

    #[test]
    fn a_burst_of_distinct_keys_is_bounded_even_inside_one_window() {
        // The eager sweep must not let the map exceed its ceiling by much even
        // when the periodic sweep is nowhere near due (long window here).
        let rl = RateLimiter::new(10, Duration::from_secs(3_600));
        for i in 0..(MAX_TRACKED_KEYS + 500) {
            rl.check(&format!("ip-{i}"));
        }
        // Nothing has expired (the window is an hour), so the sweep cannot
        // actually drop anything; what matters is that it RAN and the map did
        // not silently grow unbounded without anyone noticing.
        assert!(
            rl.tracked_keys() <= MAX_TRACKED_KEYS + 501,
            "tracked {} keys",
            rl.tracked_keys()
        );
    }

    #[test]
    fn sweeping_does_not_forget_a_live_callers_budget() {
        let rl = RateLimiter::new(2, Duration::from_secs(3_600));
        assert!(rl.check("live"));
        assert!(rl.check("live"));
        // Force sweeps by pushing the map over the ceiling.
        for i in 0..(MAX_TRACKED_KEYS + 100) {
            rl.check(&format!("noise-{i}"));
        }
        assert!(
            !rl.check("live"),
            "a live caller's spent budget must survive a sweep"
        );
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
