//! A bounded set of slots for the proofs that run a child process.
//!
//! A proof runs a CLI for up to thirty seconds, and any viewer may ask for one.
//! Without a bound, a viewer can start as many children as the box will allow.
//! With one, a busy slot is an answer the caller can read ([`Busy`]), not a
//! queue that grows until something gives. The slots are counted in the console
//! process, shared by every kind of proof (slice 3's policy replay, slice 4's
//! budget backtest), so the bound is on proofs and not on one tool.
//!
//! The slot is a guard: it is released when it is dropped, on every path out of
//! the proof, a timeout and a panic included.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// How many proofs the console process runs at once.
pub const MAX_CONCURRENT_PROOFS: usize = 2;

/// Every slot is taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy {
    pub limit: usize,
}

/// A set of slots. Cheap to clone; clones share the count.
#[derive(Debug, Clone)]
pub struct Slots {
    limit: usize,
    in_use: Arc<AtomicUsize>,
}

/// One slot taken. Dropping it gives the slot back.
#[derive(Debug)]
pub struct Slot {
    in_use: Arc<AtomicUsize>,
}

impl Slots {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            in_use: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    /// How many slots are taken now.
    pub fn in_use(&self) -> usize {
        self.in_use.load(Ordering::Acquire)
    }

    /// Take a slot, or say that none is free. Never waits.
    pub fn try_acquire(&self) -> Result<Slot, Busy> {
        self.in_use.fetch_add(1, Ordering::AcqRel);
        Ok(Slot {
            in_use: Arc::clone(&self.in_use),
        })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.in_use.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_hand_out_exactly_their_limit() {
        let slots = Slots::new(2);
        let a = slots.try_acquire().expect("a first slot");
        let b = slots.try_acquire().expect("a second slot");
        assert_eq!(slots.in_use(), 2);
        assert_eq!(slots.try_acquire().err(), Some(Busy { limit: 2 }));
        drop((a, b));
        assert_eq!(slots.in_use(), 0);
    }

    #[test]
    fn a_released_slot_is_taken_again() {
        let slots = Slots::new(1);
        let first = slots.try_acquire().expect("a slot");
        assert!(slots.try_acquire().is_err());
        drop(first);
        let second = slots.try_acquire().expect("the slot came back");
        assert_eq!(slots.in_use(), 1);
        drop(second);
    }

    #[test]
    fn a_limit_of_zero_refuses_everything() {
        let slots = Slots::new(0);
        assert_eq!(slots.try_acquire().err(), Some(Busy { limit: 0 }));
        assert_eq!(slots.in_use(), 0);
    }

    /// Many threads taking and giving slots never hold more than the limit at
    /// once, and the count ends where it began.
    #[test]
    fn two_hundred_threads_never_hold_more_than_the_limit() {
        let slots = Slots::new(3);
        let held = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..200)
            .map(|_| {
                let (slots, held, most) = (slots.clone(), Arc::clone(&held), Arc::clone(&most));
                std::thread::spawn(move || {
                    if let Ok(slot) = slots.try_acquire() {
                        let now = held.fetch_add(1, Ordering::AcqRel) + 1;
                        most.fetch_max(now, Ordering::AcqRel);
                        std::thread::yield_now();
                        held.fetch_sub(1, Ordering::AcqRel);
                        drop(slot);
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("a thread");
        }
        assert!(most.load(Ordering::Acquire) <= 3, "the limit was exceeded");
        assert_eq!(slots.in_use(), 0, "a slot leaked");
    }
}
