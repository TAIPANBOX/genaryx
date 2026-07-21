//! Drills-panel Tauri managed state: a resolved `MockryxClient` (or an
//! honest record that no drills plane was found), plus the gateway/api-key/
//! scenario-dir a run needs.
//!
//! Simpler than every panel with a network target: like Crypto's qryx, there
//! is no serve process and no healthz step - mockryx is invoked fresh for
//! every run - so there is no `Unreachable` variant here at all, mirroring
//! `crypto::state`'s identical rationale exactly (a spawn/exit failure only
//! ever happens WHEN a drill actually runs, surfaced as a normal command
//! error - see `commands.rs` - not a bootstrap-time distinction). Still keeps
//! the same non-blocking `pending()`-then-spawn-`bootstrap()` wiring as every
//! other panel for consistency, even though today's `bootstrap` body never
//! actually awaits anything either (mirrors `crypto::state::bootstrap`'s
//! identical note).

use super::env::{self, EnvSource};
use genaryx_connectors::MockryxClient;
use std::path::PathBuf;
use tokio::sync::Mutex;

/// A resolved mockryx binary plus everything a run needs. Cheap to clone:
/// `MockryxClient` itself wraps only a `PathBuf` (stateless, like
/// `QryxClient` - a fresh process per run, never a held connection).
#[derive(Clone)]
pub struct DrillsClient {
    pub client: MockryxClient,
    pub source: EnvSource,
    pub mockryx_bin: PathBuf,
    pub gateway_url: String,
    pub api_key: Option<String>,
    /// A best-effort starting point for the operator's editable field, not
    /// an authority - see `env.rs`'s module doc.
    pub scenario_dir: Option<PathBuf>,
}

/// The Drills panel's whole state machine - see this module's doc comment
/// for why there is no `Unreachable` shape here.
pub enum DrillsInner {
    /// The initial state from [`DrillsState::pending`], until the
    /// background [`bootstrap`] task resolves.
    Bootstrapping,
    /// No usable `mockryx` binary and/or `services.gateway` descriptor entry
    /// found - the common case until an operator builds/installs mockryx and
    /// brings up an environment. A normal, renderable "no drills plane"
    /// state, never an error.
    NoEnvironment,
    Ready(DrillsClient),
}

/// Tauri-managed state wrapping [`DrillsInner`] in an async mutex, mirroring
/// `CryptoState`'s identical shape.
pub struct DrillsState {
    pub inner: Mutex<DrillsInner>,
}

impl DrillsState {
    /// The synchronous, immediately-manageable starting state - `setup`
    /// calls this directly, then spawns [`bootstrap`] in the background.
    #[must_use]
    pub fn pending() -> Self {
        Self {
            inner: Mutex::new(DrillsInner::Bootstrapping),
        }
    }
}

/// Resolve the mockryx binary + gateway + scenario dir - see this module's
/// doc comment for why there is no reachability probe to await here. Never
/// panics.
pub async fn bootstrap() -> DrillsInner {
    match env::discover() {
        Some(resolved) => DrillsInner::Ready(DrillsClient {
            // Point the drill at the environment's bus when there is one, so
            // a rehearsal leaves a trail. mockryx keeps no history of its own
            // (fresh run_id per run, `--save` overwrites, no `list`
            // subcommand), so the append-only event log is the only place a
            // finished drill survives. Mirrors `crates/ffi/src/drills`.
            client: match genaryx_core::bus::discover().and_then(|bus| bus.writer_path("mockryx")) {
                Some(path) => {
                    MockryxClient::new(resolved.mockryx_bin.clone()).with_events_path(path)
                }
                None => MockryxClient::new(resolved.mockryx_bin.clone()),
            },
            source: resolved.source,
            mockryx_bin: resolved.mockryx_bin,
            gateway_url: resolved.gateway_url,
            api_key: resolved.api_key,
            scenario_dir: resolved.scenario_dir,
        }),
        None => DrillsInner::NoEnvironment,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_starts_in_the_bootstrapping_state() {
        let state = DrillsState::pending();
        let guard = state
            .inner
            .try_lock()
            .expect("uncontended right after construction");
        assert!(matches!(&*guard, DrillsInner::Bootstrapping));
    }

    #[tokio::test]
    async fn bootstrap_never_panics_with_no_environment_available() {
        let inner = bootstrap().await;
        match inner {
            DrillsInner::Bootstrapping => {
                panic!("bootstrap must resolve past its own pending state")
            }
            DrillsInner::NoEnvironment | DrillsInner::Ready(_) => {}
        }
    }
}
