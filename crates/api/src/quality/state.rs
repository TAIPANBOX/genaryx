//! Quality-panel Tauri managed state: a resolved `verdryx.db` path (or an
//! honest record of why there isn't one), confirmed openable once at
//! bootstrap.
//!
//! Mirrors `crate::identity::state` in shape (`Bootstrapping` ->
//! background-resolve -> `Ready`/failure-variant, non-blocking `setup`-calls-
//! pending-then-spawns-bootstrap contract) but simpler in the way Verdryx
//! itself is: there is no live connection to hold at all. A
//! `genaryx_connectors::VerdryxClient` wraps a `rusqlite::Connection`, which
//! is `!Sync` and explicitly documented as "not Send-shared across threads
//! by design... the shells open one per read context" (its own module doc) -
//! so unlike `IdentityClient` (an `Arc<IdryxClient>` reused across every
//! read), [`QualityClient`] holds only the resolved `db_path`. Every read in
//! `super::commands` opens its own short-lived `VerdryxClient::open(&db_path)`
//! inside a `spawn_blocking` (rusqlite is synchronous, blocking IO - the same
//! reason `identity::commands::identity_rescan` wraps its own blocking
//! `IdryxClient::rescan` call), never a connection parked in this state
//! across calls.
//!
//! [`bootstrap`] itself performs exactly one such open-and-drop, purely to
//! confirm the resolved path is a genuine, readable SQLite store before
//! calling the plane `Ready` - a resolved path that fails to open (a corrupt
//! file, an uncheckpointed WAL from a crashed `eval`, wrong permissions)
//! becomes [`QualityInner::Unreachable`], distinct from
//! [`QualityInner::NoEnvironment`] ("never found a candidate path at all",
//! `super::env::discover`'s job).

use super::env::{self, EnvSource, ResolvedEnv};
use genaryx_connectors::VerdryxClient;
use std::path::PathBuf;
use tokio::sync::Mutex;

/// A resolved place to read Verdryx's quality plane from. Cheap to clone (a
/// tagged source plus one `PathBuf`) - mirrors `IdentityClient`'s rationale,
/// minus the live client itself (see this module's doc comment for why).
#[derive(Clone)]
pub struct QualityClient {
    pub source: EnvSource,
    pub db_path: PathBuf,
}

/// The Quality panel's whole state machine - mirrors `IdentityInner`'s four
/// shapes, substituting "a readable verdryx.db" for "a live idryx healthz".
pub enum QualityInner {
    /// The initial state from [`QualityState::pending`], until the
    /// background [`bootstrap`] task resolves.
    Bootstrapping,
    /// [`env::discover`] found no candidate path at all (no descriptor
    /// entry, no well-known file) - the common case until an operator points
    /// the console at a real `verdryx.db`. A normal, renderable "no quality
    /// plane" state, never an error.
    NoEnvironment,
    /// A candidate path resolved, but `VerdryxClient::open` failed on it.
    Unreachable {
        source: EnvSource,
        db_path: PathBuf,
        reason: String,
    },
    Ready(QualityClient),
}

/// Tauri-managed state wrapping [`QualityInner`] in an async mutex,
/// mirroring `IdentityState`'s identical shape.
pub struct QualityState {
    pub inner: Mutex<QualityInner>,
}

impl QualityState {
    /// The synchronous, immediately-manageable starting state - `setup`
    /// calls this directly, then spawns [`bootstrap`] in the background.
    #[must_use]
    pub fn pending() -> Self {
        Self {
            inner: Mutex::new(QualityInner::Bootstrapping),
        }
    }
}

/// Resolve an environment and confirm `verdryx.db` genuinely opens
/// read-only, off the async executor thread (rusqlite is blocking IO - see
/// this module's doc comment). Never panics, never returns anything other
/// than a [`QualityInner`] the UI can render.
pub async fn bootstrap() -> QualityInner {
    let Some(resolved) = env::discover() else {
        return QualityInner::NoEnvironment;
    };
    confirm_openable(resolved).await
}

/// Open-and-drop `resolved.db_path` once, off the async executor thread, to
/// turn a resolved candidate into `Ready` or an honest `Unreachable` with
/// `VerdryxError`'s own message as the reason.
async fn confirm_openable(resolved: ResolvedEnv) -> QualityInner {
    let path = resolved.db_path.clone();
    let opened = tokio::task::spawn_blocking(move || VerdryxClient::open(&path).map(|_| ())).await;
    match opened {
        Ok(Ok(())) => QualityInner::Ready(QualityClient {
            source: resolved.source,
            db_path: resolved.db_path,
        }),
        Ok(Err(e)) => QualityInner::Unreachable {
            source: resolved.source,
            db_path: resolved.db_path,
            reason: e.to_string(),
        },
        Err(join_err) => QualityInner::Unreachable {
            source: resolved.source,
            db_path: resolved.db_path,
            reason: format!("bootstrap task failed to run: {join_err}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_starts_in_the_bootstrapping_state() {
        let state = QualityState::pending();
        let guard = state
            .inner
            .try_lock()
            .expect("uncontended right after construction");
        assert!(matches!(&*guard, QualityInner::Bootstrapping));
    }

    #[tokio::test]
    async fn bootstrap_never_panics_with_no_environment_available() {
        // Same rationale as identity::state's identical test: this only
        // proves `bootstrap` resolves to a `QualityInner` rather than
        // panicking or hanging, regardless of whether this box happens to
        // have a real verdryx.db.
        let inner = bootstrap().await;
        match inner {
            QualityInner::Bootstrapping => {
                panic!("bootstrap must resolve past its own pending state")
            }
            QualityInner::NoEnvironment
            | QualityInner::Unreachable { .. }
            | QualityInner::Ready(_) => {}
        }
    }

    #[tokio::test]
    async fn confirm_openable_reports_unreachable_for_a_nonexistent_path() {
        let path = std::env::temp_dir().join("genaryx-quality-state-test-does-not-exist.db");
        let _ = std::fs::remove_file(&path);
        let resolved = ResolvedEnv {
            source: EnvSource::WellKnown,
            db_path: path,
        };
        match confirm_openable(resolved).await {
            QualityInner::Unreachable { reason, .. } => assert!(!reason.is_empty()),
            QualityInner::Ready(_) => {
                panic!("expected Unreachable for a nonexistent db, got Ready")
            }
            QualityInner::NoEnvironment => {
                panic!("expected Unreachable for a nonexistent db, got NoEnvironment")
            }
            QualityInner::Bootstrapping => {
                panic!("expected Unreachable for a nonexistent db, got Bootstrapping")
            }
        }
    }

    #[tokio::test]
    async fn confirm_openable_reports_ready_for_a_genuine_sqlite_file() {
        // A real, empty SQLite file (rusqlite::Connection::open creates one)
        // - enough to prove the read-only re-open in confirm_openable
        // succeeds against a genuine (if schema-less) database, mirroring
        // `VerdryxClient`'s own `open_missing_db_is_fail_closed` test's
        // counterpart-in-the-positive-direction.
        let path = std::env::temp_dir().join(format!(
            "genaryx-quality-state-test-{}-{}.db",
            std::process::id(),
            nanos()
        ));
        let _ = std::fs::remove_file(&path);
        {
            let _ = rusqlite::Connection::open(&path).expect("create a real sqlite file");
        }

        let resolved = ResolvedEnv {
            source: EnvSource::WellKnown,
            db_path: path.clone(),
        };
        match confirm_openable(resolved).await {
            QualityInner::Ready(client) => assert_eq!(client.db_path, path),
            QualityInner::Unreachable { reason, .. } => {
                panic!("expected Ready for a genuine sqlite file, got Unreachable: {reason}")
            }
            QualityInner::NoEnvironment => {
                panic!("expected Ready for a genuine sqlite file, got NoEnvironment")
            }
            QualityInner::Bootstrapping => {
                panic!("expected Ready for a genuine sqlite file, got Bootstrapping")
            }
        }

        let _ = std::fs::remove_file(&path);
    }

    fn nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }
}
