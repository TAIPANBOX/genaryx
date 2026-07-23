//! Admission-plane managed state: a [`GatewayClient`] (or an honest record of
//! why there isn't one yet). Mirrors `crate::credentials::state` byte for
//! byte (same `Bootstrapping -> background-resolve ->
//! Unreachable`/`Ready` shape, same non-blocking `setup`-calls-
//! [`AdmissionState::pending`]-then-spawns-[`bootstrap`] contract, the SAME
//! `GET /v1/keys` read as the reachability probe - see that module's doc
//! comment for why there is no dedicated healthz route to use instead).
//!
//! Deliberately holds ONLY the gateway leg: the verdryx binary and
//! `verdryx.db` legs are independent, re-checked-per-call facts
//! (`super::env::resolve_verdryx_bin`/`resolve_verdryx_db`), never cached in
//! this state machine - see `env.rs`'s module doc, "Honest per-piece
//! resolution states", for why they are not folded in here the way
//! `crate::drills::state` folds mockryx+gateway together into one `Ready`.

use super::env::{self, EnvSource, ResolvedEnv};
use genaryx_connectors::GatewayClient;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// How long [`bootstrap`] waits for discovery+the reachability probe before
/// giving up and falling back to [`AdmissionInner::Unreachable`] - same value
/// and rationale as `credentials::state::CONNECT_TIMEOUT`.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// A ready-to-use gateway connection. Cheap to clone (an `Arc`ed client plus
/// a couple of small strings), mirroring `CredentialsClient`'s identical
/// rationale.
#[derive(Clone)]
pub struct AdmissionClient {
    pub client: Arc<GatewayClient>,
    pub source: EnvSource,
    pub gateway_url: String,
}

/// The Admission plane's gateway-leg state machine - mirrors
/// `CredentialsInner`'s same four shapes exactly.
pub enum AdmissionInner {
    /// The initial state from [`AdmissionState::pending`], until the
    /// background [`bootstrap`] task resolves.
    Bootstrapping,
    /// [`env::discover_gateway`] found nothing usable: no `taipan up`
    /// descriptor with a `gateway` service. A normal, renderable "no
    /// admission plane" state, never an error.
    NoEnvironment,
    /// An environment resolved (a URL we could build a client for), but the
    /// reachability probe (`GET /v1/keys`) failed, timed out, or answered a
    /// non-2xx status.
    Unreachable {
        source: EnvSource,
        gateway_url: String,
        reason: String,
    },
    Ready(AdmissionClient),
}

/// Managed state wrapping [`AdmissionInner`] in an async mutex, mirroring
/// `CredentialsState`'s identical shape.
pub struct AdmissionState {
    pub inner: Mutex<AdmissionInner>,
}

impl AdmissionState {
    /// The synchronous, immediately-manageable starting state - `setup`/
    /// `Ctx::bootstrap` calls this directly, then spawns [`bootstrap`] in the
    /// background (see this module's doc comment).
    #[must_use]
    pub fn pending() -> Self {
        Self {
            inner: Mutex::new(AdmissionInner::Bootstrapping),
        }
    }
}

/// Resolve the gateway leg and confirm it is actually live - the
/// [`AdmissionInner`] the caller should swap into managed state. Never
/// panics and never returns anything other than an [`AdmissionInner`] the UI
/// can render. The verdryx binary/db legs are NOT resolved here - see this
/// module's doc comment.
pub async fn bootstrap() -> AdmissionInner {
    let Some(resolved) = env::discover_gateway() else {
        return AdmissionInner::NoEnvironment;
    };

    match tokio::time::timeout(CONNECT_TIMEOUT, connect(&resolved)).await {
        Ok(Ok(client)) => AdmissionInner::Ready(AdmissionClient {
            client: Arc::new(client),
            source: resolved.source,
            gateway_url: resolved.gateway_url,
        }),
        Ok(Err(reason)) => AdmissionInner::Unreachable {
            source: resolved.source,
            gateway_url: resolved.gateway_url,
            reason,
        },
        Err(_elapsed) => AdmissionInner::Unreachable {
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
/// discarded here - only reachability matters at bootstrap time;
/// `super::commands::admission_check` always re-fetches fresh.
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
        let state = AdmissionState::pending();
        let guard = state
            .inner
            .try_lock()
            .expect("uncontended right after construction");
        assert!(matches!(&*guard, AdmissionInner::Bootstrapping));
    }

    #[tokio::test]
    async fn bootstrap_never_panics_with_no_environment_available() {
        // Same rationale as credentials::state's identical test: this only
        // proves `bootstrap` resolves to an `AdmissionInner` rather than
        // panicking or hanging, regardless of whether this box happens to
        // have a real `taipan up` environment.
        let inner = bootstrap().await;
        match inner {
            AdmissionInner::Bootstrapping => {
                panic!("bootstrap must resolve past its own pending state")
            }
            AdmissionInner::NoEnvironment
            | AdmissionInner::Unreachable { .. }
            | AdmissionInner::Ready(_) => {}
        }
    }
}
