//! Evidence-panel console-managed state: the independently-resolved local-tool
//! sources (qryx / idryx / tokenfuse) a pack can draw from - see `env.rs`'s
//! module doc for how each is resolved. Cloud is deliberately absent from
//! this state entirely: the Evidence build command reuses the Money plane's
//! already-paired `CloudClient` directly out of `MoneyState` (never pairs a
//! second device) - see `commands.rs`'s module doc for the full rationale.
//!
//! Simpler than every panel with a single Ready/NoEnvironment/Unreachable
//! gate: since all three sources here are independently `Option` (an
//! operator can build a pack from just one, or none at all -
//! `crates/connectors/src/evidence.rs`'s "every source is optional and
//! independent" contract), there is no single readiness gate to model at
//! all. `EvidenceInner` only distinguishes `Bootstrapping` from
//! `Ready(EvidenceEnv)`; `EvidenceEnv`'s own fields carry the "unresolved"
//! case per source. Still kept as the same non-blocking
//! `pending()`-then-spawn-`bootstrap()` shape as every other panel for
//! consistency (mirrors `crypto::state`/`drills::state`'s identical note:
//! `bootstrap` itself does no I/O that could ever meaningfully block - three
//! cheap filesystem checks - but the shape stays uniform across panels).

use super::env;
use genaryx_connectors::{QryxClient, TokenfuseClient};
use std::path::PathBuf;
use tokio::sync::Mutex;

/// A resolved qryx source: the client, plus the bin/target the status DTO
/// reports back to the frontend for display.
#[derive(Clone)]
pub struct QryxSource {
    pub client: QryxClient,
    pub qryx_bin: PathBuf,
    pub default_target: PathBuf,
}

/// A resolved idryx source. No client object: `IdryxClient::agent_bom` is a
/// synchronous associated function over a binary path
/// (`crates/connectors/src/idryx.rs`), not an instance method - there is
/// nothing to hold beyond the path and the load specs themselves.
#[derive(Clone)]
pub struct IdryxSource {
    pub idryx_bin: PathBuf,
    pub loads: Vec<(String, PathBuf)>,
}

/// A resolved TokenFuse source.
#[derive(Clone)]
pub struct TokenfuseSource {
    pub client: TokenfuseClient,
    pub tokenfuse_bin: PathBuf,
    pub default_traces_dir: Option<PathBuf>,
}

/// Every local-tool source the Evidence Center can draw from, each resolved
/// fully independently - see this module's doc comment.
#[derive(Clone, Default)]
pub struct EvidenceEnv {
    pub qryx: Option<QryxSource>,
    pub idryx: Option<IdryxSource>,
    pub tokenfuse: Option<TokenfuseSource>,
}

/// The Evidence panel's whole state machine - see this module's doc comment
/// for why there is only ever `Bootstrapping` or `Ready` (never a
/// `NoEnvironment`: an all-`None` `EvidenceEnv` is still a valid `Ready`
/// state, just one where the frontend disables every local-tool checkbox).
pub enum EvidenceInner {
    /// The initial state from [`EvidenceState::pending`], until the
    /// background [`bootstrap`] task resolves.
    Bootstrapping,
    Ready(EvidenceEnv),
}

/// Console-managed state wrapping [`EvidenceInner`] in an async mutex,
/// mirroring every sibling panel's identical shape.
pub struct EvidenceState {
    pub inner: Mutex<EvidenceInner>,
}

impl EvidenceState {
    /// The synchronous, immediately-manageable starting state - `setup`
    /// calls this directly, then spawns [`bootstrap`] in the background.
    #[must_use]
    pub fn pending() -> Self {
        Self {
            inner: Mutex::new(EvidenceInner::Bootstrapping),
        }
    }
}

/// Resolve all three local-tool sources - see `env.rs`'s module doc. Never
/// panics and always resolves to `Ready` (there is no failure mode at this
/// layer: an unresolved source is simply `None` inside `EvidenceEnv`, not an
/// error - see this module's doc comment).
pub async fn bootstrap() -> EvidenceInner {
    let qryx = env::discover_qryx().map(|r| QryxSource {
        client: QryxClient::new(r.qryx_bin.clone()),
        qryx_bin: r.qryx_bin,
        default_target: r.default_target,
    });
    let idryx = env::discover_idryx().map(|r| IdryxSource {
        idryx_bin: r.idryx_bin,
        loads: r.loads,
    });
    let tokenfuse = env::discover_tokenfuse().map(|r| TokenfuseSource {
        client: TokenfuseClient::new(r.tokenfuse_bin.clone()),
        tokenfuse_bin: r.tokenfuse_bin,
        default_traces_dir: r.default_traces_dir,
    });
    EvidenceInner::Ready(EvidenceEnv {
        qryx,
        idryx,
        tokenfuse,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_starts_in_the_bootstrapping_state() {
        let state = EvidenceState::pending();
        let guard = state
            .inner
            .try_lock()
            .expect("uncontended right after construction");
        assert!(matches!(&*guard, EvidenceInner::Bootstrapping));
    }

    #[tokio::test]
    async fn bootstrap_never_panics_and_always_reaches_ready() {
        // Mirrors every sibling panel's identical rationale: this only
        // proves `bootstrap` resolves (never panics, never hangs) regardless
        // of whether this box happens to have any of the three tools
        // installed - an all-`None` `EvidenceEnv` is still `Ready`.
        let inner = bootstrap().await;
        assert!(matches!(inner, EvidenceInner::Ready(_)));
    }

    #[test]
    fn evidence_env_default_is_all_none() {
        let env = EvidenceEnv::default();
        assert!(env.qryx.is_none());
        assert!(env.idryx.is_none());
        assert!(env.tokenfuse.is_none());
    }
}
