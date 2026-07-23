//! Credentials-panel managed state: a [`GatewayClient`] (or an honest record
//! of why there isn't one yet). Mirrors `crate::identity::state` structurally
//! (same `Bootstrapping -> background-resolve -> Unreachable`/`Ready` shape,
//! same non-blocking `setup`-calls-[`CredentialsState::pending`]-then-spawns-
//! [`bootstrap`] contract) but simpler still: this plane keeps nothing
//! alongside the client beyond the gateway URL itself - no Rescan-equivalent,
//! no extra best-effort resolution, since its one command
//! (`super::commands::credentials_keys`) always re-fetches fresh.
//!
//! Unlike idryx, the gateway has no dedicated health-check route in the I15
//! contract (docs/22-key-lifecycle.md in tokenfuse names only
//! `GET /v1/keys`), so [`connect`] uses that SAME read as its own
//! reachability probe: successfully fetching the key-lifecycle report IS "the
//! gateway is up and this plane can do its one job", and any failure
//! (transport, non-2xx, an undeserializable body) is an honest
//! [`CredentialsInner::Unreachable`], never a fabricated `Ready`. The result
//! of that bootstrap-time fetch is discarded, not cached: `credentials_keys`
//! re-reads on every call (the report changes as calls come in), so nothing
//! here is ever treated as a stale snapshot.

use super::env::{self, EnvSource, ResolvedEnv};
use genaryx_connectors::GatewayClient;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// How long [`bootstrap`] waits for discovery+the reachability probe before
/// giving up and falling back to [`CredentialsInner::Unreachable`] - same
/// value and rationale as `identity::state::CONNECT_TIMEOUT`.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// A ready-to-use gateway connection. Cheap to clone (an `Arc`ed client plus
/// a couple of small strings), mirroring `IdentityClient`'s identical
/// rationale.
#[derive(Clone)]
pub struct CredentialsClient {
    pub client: Arc<GatewayClient>,
    pub source: EnvSource,
    pub gateway_url: String,
}

/// The Credentials panel's whole state machine - mirrors `IdentityInner`'s
/// same four shapes (no separate "pairing" concept: the gateway has no auth
/// at all to fail on here, so `Unreachable` covers a failed/timed-out probe
/// exactly like it does for Identity's unauthenticated idryx).
pub enum CredentialsInner {
    /// The initial state from [`CredentialsState::pending`], until the
    /// background [`bootstrap`] task resolves.
    Bootstrapping,
    /// [`env::discover`] found nothing usable: no `taipan up` descriptor with
    /// a `gateway` service. A normal, renderable "no credentials plane"
    /// state, never an error.
    NoEnvironment,
    /// An environment resolved (a URL we could build a client for), but the
    /// reachability probe (`GET /v1/keys`) failed, timed out, or answered a
    /// non-2xx status.
    Unreachable {
        source: EnvSource,
        gateway_url: String,
        reason: String,
    },
    Ready(CredentialsClient),
}

/// Managed state wrapping [`CredentialsInner`] in an async mutex, mirroring
/// `IdentityState`'s identical shape.
pub struct CredentialsState {
    pub inner: Mutex<CredentialsInner>,
}

impl CredentialsState {
    /// The synchronous, immediately-manageable starting state - `setup`/
    /// `Ctx::bootstrap` calls this directly, then spawns [`bootstrap`] in the
    /// background (see this module's doc comment).
    #[must_use]
    pub fn pending() -> Self {
        Self {
            inner: Mutex::new(CredentialsInner::Bootstrapping),
        }
    }
}

/// Resolve an environment and confirm it is actually live - the
/// [`CredentialsInner`] the caller should swap into managed state. Never
/// panics and never returns anything other than a [`CredentialsInner`] the UI
/// can render.
pub async fn bootstrap() -> CredentialsInner {
    let Some(resolved) = env::discover() else {
        return CredentialsInner::NoEnvironment;
    };

    match tokio::time::timeout(CONNECT_TIMEOUT, connect(&resolved)).await {
        Ok(Ok(client)) => CredentialsInner::Ready(CredentialsClient {
            client: Arc::new(client),
            source: resolved.source,
            gateway_url: resolved.gateway_url,
        }),
        Ok(Err(reason)) => CredentialsInner::Unreachable {
            source: resolved.source,
            gateway_url: resolved.gateway_url,
            reason,
        },
        Err(_elapsed) => CredentialsInner::Unreachable {
            source: resolved.source,
            gateway_url: resolved.gateway_url,
            reason: format!(
                "timed out after {:.0}s waiting for the gateway to respond",
                CONNECT_TIMEOUT.as_secs_f64()
            ),
        },
    }
}

/// Build a [`GatewayClient`] and confirm it is live by actually fetching the
/// key-lifecycle report once (see this module's doc comment for why there is
/// no separate `/healthz` to probe instead). The fetched report itself is
/// discarded here - only reachability matters at bootstrap time.
async fn connect(resolved: &ResolvedEnv) -> Result<GatewayClient, String> {
    let client = GatewayClient::new(resolved.gateway_url.clone()).map_err(|e| e.to_string())?;
    client.get_keys().await.map_err(|e| e.to_string())?;
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_starts_in_the_bootstrapping_state() {
        let state = CredentialsState::pending();
        let guard = state
            .inner
            .try_lock()
            .expect("uncontended right after construction");
        assert!(matches!(&*guard, CredentialsInner::Bootstrapping));
    }

    #[tokio::test]
    async fn bootstrap_never_panics_with_no_environment_available() {
        // Same rationale as identity::state's identical test: this only
        // proves `bootstrap` resolves to a `CredentialsInner` rather than
        // panicking or hanging, regardless of whether this box happens to
        // have a real `taipan up` environment.
        let inner = bootstrap().await;
        match inner {
            CredentialsInner::Bootstrapping => {
                panic!("bootstrap must resolve past its own pending state")
            }
            CredentialsInner::NoEnvironment
            | CredentialsInner::Unreachable { .. }
            | CredentialsInner::Ready(_) => {}
        }
    }
}
