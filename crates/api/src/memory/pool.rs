//! Scoped Engram servers for the Memory panel (F-007).
//!
//! Engram's `engram-mcp` takes its scope from its own identity: started with
//! `--agent-id`, it serves that agent; started without one, it serves the whole
//! file. A read cannot name another agent to it. So a per-agent view needs a
//! server started for that agent, and the whole-file view keeps the one server
//! the panel starts at bootstrap.
//!
//! [`ScopePool`] holds those per-agent servers. It starts one the first time its
//! agent is read, reuses it after that, never holds more than `max` of them, and
//! closes one that has not been read for `idle`. Closing is dropping the last
//! handle: the client's own `Drop` kills and reaps the child process.
//!
//! [`valid_agent_scope`] decides which names may become a server's `--agent-id`,
//! so a name from the UI never reaches the command line as a flag or as text
//! with spaces in it.

use std::sync::Arc;
use std::time::{Duration, Instant};

/// Per-agent servers held at once, beside the whole-file one.
pub const MAX_SCOPED_SERVERS: usize = 4;

/// How long a per-agent server may go unread before it is closed.
pub const IDLE_AFTER: Duration = Duration::from_secs(10 * 60);

/// Whether `agent_id` may become a server's `--agent-id`: one printable word,
/// at most 256 bytes, and not a flag. Estate agent ids (`agent://…`) pass; a
/// name with a space, a control character, a leading dash or non-ASCII text
/// does not.
pub fn valid_agent_scope(agent_id: &str) -> bool {
    !agent_id.is_empty()
        && agent_id.len() <= 256
        && !agent_id.starts_with('-')
        && agent_id.chars().all(|c| c.is_ascii_graphic())
}

/// One held server: the scope it serves, the server, and when and in what order
/// it was last read.
struct Entry<C> {
    scope: String,
    client: Arc<C>,
    last_used: Instant,
    seq: u64,
}

/// The per-agent servers the Memory panel holds: at most `max` at once, each
/// closed after `idle` without a read. A read reuses the server its agent
/// already has. A start that fails is not held.
pub struct ScopePool<C> {
    entries: Vec<Entry<C>>,
    max: usize,
    idle: Duration,
    next_seq: u64,
}

impl<C> ScopePool<C> {
    /// A pool that holds at least one server, whatever `max` says.
    pub fn new(max: usize, idle: Duration) -> Self {
        Self {
            entries: Vec::new(),
            max: max.max(1),
            idle,
            next_seq: 0,
        }
    }

    /// The server for `scope`, reused when the pool holds one and started with
    /// `spawn` otherwise. Idle servers are closed first; then, when the pool is
    /// full, the least recently read one is closed to make room.
    pub fn client_for<E>(
        &mut self,
        scope: &str,
        now: Instant,
        spawn: impl FnOnce() -> Result<C, E>,
    ) -> Result<Arc<C>, E> {
        self.close_idle(now);
        self.next_seq += 1;
        let seq = self.next_seq;
        if let Some(entry) = self.entries.iter_mut().find(|e| e.scope == scope) {
            entry.last_used = now;
            entry.seq = seq;
            return Ok(Arc::clone(&entry.client));
        }
        let client = Arc::new(spawn()?);
        while self.entries.len() >= self.max {
            self.close_least_recent();
        }
        self.entries.push(Entry {
            scope: scope.to_string(),
            client: Arc::clone(&client),
            last_used: now,
            seq,
        });
        Ok(client)
    }

    /// Close every server not read for longer than `idle`.
    fn close_idle(&mut self, now: Instant) {
        let idle = self.idle;
        self.entries
            .retain(|e| now.saturating_duration_since(e.last_used) <= idle);
    }

    /// Close the least recently read server.
    fn close_least_recent(&mut self) {
        let oldest = self
            .entries
            .iter()
            .enumerate()
            .min_by_key(|(_, e)| e.seq)
            .map(|(i, _)| i);
        if let Some(i) = oldest {
            self.entries.swap_remove(i);
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const A: &str = "agent://acme.example/support/bot";
    const B: &str = "agent://acme.example/support/other";
    const C: &str = "agent://acme.example/finance/bot";

    #[test]
    fn a_scope_is_spawned_once_and_then_reused() {
        let spawns = Cell::new(0u32);
        let mut pool = ScopePool::<u32>::new(2, IDLE_AFTER);
        let t = Instant::now();
        let first = pool
            .client_for(A, t, || {
                spawns.set(spawns.get() + 1);
                Ok::<u32, String>(1)
            })
            .unwrap();
        let again = pool
            .client_for(A, t, || {
                spawns.set(spawns.get() + 1);
                Ok::<u32, String>(2)
            })
            .unwrap();
        assert_eq!(
            spawns.get(),
            1,
            "the second read reuses the server it started"
        );
        assert!(Arc::ptr_eq(&first, &again));
    }

    #[test]
    fn the_pool_never_holds_more_than_its_bound() {
        let mut pool = ScopePool::<u32>::new(2, IDLE_AFTER);
        let t = Instant::now();
        for scope in [A, B, C] {
            pool.client_for(scope, t, || Ok::<u32, String>(0)).unwrap();
        }
        assert_eq!(pool.len(), 2, "a third scope evicts one of the first two");
    }

    #[test]
    fn the_least_recently_used_scope_is_the_one_evicted() {
        let spawns = Cell::new(0u32);
        let spawn = || {
            spawns.set(spawns.get() + 1);
            Ok::<u32, String>(0)
        };
        let mut pool = ScopePool::<u32>::new(2, IDLE_AFTER);
        let t = Instant::now();
        pool.client_for(A, t, spawn).unwrap();
        pool.client_for(B, t, spawn).unwrap();
        pool.client_for(A, t, spawn).unwrap(); // A is now the more recent
        pool.client_for(C, t, spawn).unwrap(); // evicts B, not A
        assert_eq!(
            spawns.get(),
            3,
            "A, B and C each started once; A was reused"
        );
        pool.client_for(A, t, spawn).unwrap();
        assert_eq!(spawns.get(), 3, "A is still held");
        pool.client_for(B, t, spawn).unwrap();
        assert_eq!(spawns.get(), 4, "B was evicted and starts again");
    }

    #[test]
    fn an_idle_scope_is_closed_on_the_next_use() {
        let spawns = Cell::new(0u32);
        let spawn = || {
            spawns.set(spawns.get() + 1);
            Ok::<u32, String>(0)
        };
        let mut pool = ScopePool::<u32>::new(4, Duration::from_secs(600));
        let t0 = Instant::now();
        pool.client_for(A, t0, spawn).unwrap();
        pool.client_for(B, t0 + Duration::from_secs(601), spawn)
            .unwrap();
        assert_eq!(pool.len(), 1, "A was idle past the limit, so it is closed");
        pool.client_for(A, t0 + Duration::from_secs(602), spawn)
            .unwrap();
        assert_eq!(spawns.get(), 3, "A starts again, because it was closed");
    }

    #[test]
    fn a_recently_used_scope_is_kept_when_another_goes_idle() {
        let mut pool = ScopePool::<u32>::new(4, Duration::from_secs(600));
        let t0 = Instant::now();
        pool.client_for(A, t0, || Ok::<u32, String>(0)).unwrap();
        pool.client_for(B, t0 + Duration::from_secs(500), || Ok::<u32, String>(0))
            .unwrap();
        pool.client_for(C, t0 + Duration::from_secs(601), || Ok::<u32, String>(0))
            .unwrap();
        assert_eq!(
            pool.len(),
            2,
            "A went idle and closed; B was read 101 s ago and stays"
        );
    }

    #[test]
    fn a_failed_start_is_not_held_and_is_tried_again() {
        let mut pool = ScopePool::<u32>::new(2, IDLE_AFTER);
        let t = Instant::now();
        assert!(
            pool.client_for(A, t, || Err::<u32, String>("no binary".into()))
                .is_err()
        );
        assert_eq!(pool.len(), 0, "nothing is held for a start that failed");
        assert_eq!(
            pool.client_for(A, t, || Ok::<u32, String>(7)).map(|c| *c),
            Ok(7)
        );
    }

    #[test]
    fn a_scope_must_be_one_printable_word() {
        assert!(valid_agent_scope(A));
        assert!(valid_agent_scope("support-bot"));
        assert!(!valid_agent_scope(""));
        assert!(!valid_agent_scope("--db"));
        assert!(!valid_agent_scope("-x"));
        assert!(!valid_agent_scope("has space"));
        assert!(!valid_agent_scope("tab\tin"));
        assert!(!valid_agent_scope("new\nline"));
        assert!(!valid_agent_scope("é"));
        assert!(!valid_agent_scope(&"a".repeat(257)));
    }
}
