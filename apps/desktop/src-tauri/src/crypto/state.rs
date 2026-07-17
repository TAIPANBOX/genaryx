//! Crypto-panel Tauri managed state: a resolved `QryxClient` (or an honest
//! record that no qryx binary was found), plus the default on-demand scan
//! target.
//!
//! Simpler than every other panel's state shape: qryx has no service to
//! confirm reachable at bootstrap (no serve process, no healthz - it is
//! invoked fresh for every scan), so there is no `Unreachable` variant here
//! at all - either the binary resolved (`Ready`) or it did not
//! (`NoEnvironment`), mirroring exactly how
//! `identity::commands::IdentityStatusDto::Ready::rescan_available` is
//! itself just `idryx_bin.is_some()`, no liveness probe. Still keeps the
//! same non-blocking `pending()`-then-spawn-`bootstrap()` wiring as every
//! other panel for consistency, even though today's `bootstrap` body never
//! actually awaits anything.

use super::env;
use genaryx_connectors::QryxClient;
use std::path::PathBuf;
use tokio::sync::Mutex;

/// A resolved qryx binary plus a default scan target. Cheap to clone:
/// `QryxClient` itself wraps only a `PathBuf`.
#[derive(Clone)]
pub struct CryptoClient {
    pub client: QryxClient,
    pub qryx_bin: PathBuf,
    pub default_target: PathBuf,
}

/// The Crypto panel's whole state machine - see this module's doc comment
/// for why there is no `Unreachable` shape here.
pub enum CryptoInner {
    /// The initial state from [`CryptoState::pending`], until the background
    /// [`bootstrap`] task resolves.
    Bootstrapping,
    /// No `~/.taipan/bin/qryx` file found - the common case until an
    /// operator builds/installs one there. A normal, renderable "no crypto
    /// plane" state, never an error.
    NoEnvironment,
    Ready(CryptoClient),
}

/// Tauri-managed state wrapping [`CryptoInner`] in an async mutex, mirroring
/// `IdentityState`'s identical shape.
pub struct CryptoState {
    pub inner: Mutex<CryptoInner>,
}

impl CryptoState {
    /// The synchronous, immediately-manageable starting state - `setup`
    /// calls this directly, then spawns [`bootstrap`] in the background.
    #[must_use]
    pub fn pending() -> Self {
        Self {
            inner: Mutex::new(CryptoInner::Bootstrapping),
        }
    }
}

/// Resolve the qryx binary - see this module's doc comment for why there is
/// no reachability probe to await here. Never panics.
pub async fn bootstrap() -> CryptoInner {
    match env::discover() {
        Some(resolved) => CryptoInner::Ready(CryptoClient {
            client: QryxClient::new(resolved.qryx_bin.clone()),
            qryx_bin: resolved.qryx_bin,
            default_target: resolved.default_target,
        }),
        None => CryptoInner::NoEnvironment,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_starts_in_the_bootstrapping_state() {
        let state = CryptoState::pending();
        let guard = state
            .inner
            .try_lock()
            .expect("uncontended right after construction");
        assert!(matches!(&*guard, CryptoInner::Bootstrapping));
    }

    #[tokio::test]
    async fn bootstrap_never_panics_with_no_environment_available() {
        let inner = bootstrap().await;
        match inner {
            CryptoInner::Bootstrapping => {
                panic!("bootstrap must resolve past its own pending state")
            }
            CryptoInner::NoEnvironment | CryptoInner::Ready(_) => {}
        }
    }
}
