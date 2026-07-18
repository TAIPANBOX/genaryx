//! Copilot-panel Tauri managed state (Phase 6, C0 - docs/PHASE6.md,
//! itrat-console/13): the `genaryx-copilot` `CopilotService`, behind the
//! same non-blocking `pending()`-then-spawn-`bootstrap()` shape every other
//! panel's state module uses (mirrors `identity::state` most closely).
//!
//! Unlike every panel before it, there is no environment to DISCOVER here at
//! all: C0's config is always `CopilotConfig::default()` (provider "none",
//! the honest "no LLM configured on this box" state) over `Clients::default()`
//! (no connector clients wired yet - the disabled service never calls a
//! tool, so there is nothing to wire until a later cut threads the existing
//! Money/Policy/Identity clients through here). Building that default
//! service is synchronous and provably infallible (see [`bootstrap`]'s doc
//! comment), but this module still goes through the same
//! `Bootstrapping -> Ready` background-task shape as every other panel, so a
//! later config source (env var, an on-disk `[copilot]` TOML block) can
//! become genuinely async without another shape change here.
//!
//! `CopilotService` is `Send + Sync` but NOT `Clone` (it owns a
//! `Box<dyn LlmProvider>` internally), so unlike `IdentityClient`/
//! `PolicyClient` (which derive `Clone` by wrapping an already-`Arc`ed
//! connector client), this module wraps the WHOLE service in an `Arc`
//! itself: `CopilotInner::Ready(Arc<CopilotService>)`. That lets
//! `super::commands`'s helper clone the cheap `Arc` handle out of the state
//! lock exactly the way every other panel clones its own `XxxClient` out -
//! no extra inner mutex is needed beyond that, since every method this panel
//! calls (`is_enabled`/`descriptor`/`disabled_reason`/`ask`) takes `&self`,
//! so concurrent callers just share the one `Arc<CopilotService>` directly.

use genaryx_copilot::{Clients, CopilotConfig, CopilotService};
use std::sync::Arc;
use tokio::sync::Mutex;

/// The Copilot panel's whole state machine. Only two shapes are reachable in
/// practice with today's fixed C0 config - see [`bootstrap`]'s doc comment
/// for why [`CopilotInner::Failed`] is defensive rather than something this
/// build can actually hit.
pub enum CopilotInner {
    /// The initial state from [`CopilotState::pending`], until the
    /// background [`bootstrap`] task resolves - see this module's doc
    /// comment for why that resolution is near-instant today.
    Bootstrapping,
    Ready(Arc<CopilotService>),
    /// `CopilotService::from_config_and_clients` returned `Err`. Provably
    /// unreachable for today's always-`CopilotConfig::default()` input (the
    /// crate's own `provider_none_yields_a_disabled_service` unit test
    /// asserts `.unwrap()` on the identical call: `ProviderKind::None`
    /// short-circuits `build_provider` to `Ok(None)` before any fallible
    /// validation ever runs), but handled here rather than `.expect()`-ed so
    /// this module never panics even if that invariant ever changes -
    /// mirrors every other panel's "bootstrap never panics" discipline.
    Failed(String),
}

/// Tauri-managed state wrapping [`CopilotInner`] in an async mutex,
/// mirroring every other panel's identical shape.
pub struct CopilotState {
    pub inner: Mutex<CopilotInner>,
}

impl CopilotState {
    /// The synchronous, immediately-manageable starting state - `setup`
    /// calls this directly, then spawns [`bootstrap`] in the background.
    #[must_use]
    pub fn pending() -> Self {
        Self {
            inner: Mutex::new(CopilotInner::Bootstrapping),
        }
    }
}

/// Build the C0 copilot service: [`CopilotConfig::default`] (provider
/// "none") over [`Clients::default`] (no connector clients wired yet - C0
/// ships the read path with zero planes threaded through the Tauri shell,
/// see this module's doc comment). Synchronous under the hood and provably
/// infallible for this input (see [`CopilotInner::Failed`]'s doc comment:
/// `provider = none` short-circuits before any I/O or fallible validation
/// runs), but still an `async fn` run through `tauri::async_runtime::spawn`
/// from `lib.rs`'s `setup`, matching every other panel's non-blocking shape
/// - worth keeping even though nothing here awaits anything today, so a
/// later async config source is a body change, not a shape change. Never
/// panics.
pub async fn bootstrap() -> CopilotInner {
    let config = CopilotConfig::default();
    let clients = Clients::default();
    match CopilotService::from_config_and_clients(&config, clients) {
        Ok(service) => CopilotInner::Ready(Arc::new(service)),
        Err(e) => CopilotInner::Failed(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_starts_in_the_bootstrapping_state() {
        let state = CopilotState::pending();
        let guard = state
            .inner
            .try_lock()
            .expect("uncontended right after construction");
        assert!(matches!(&*guard, CopilotInner::Bootstrapping));
    }

    #[tokio::test]
    async fn bootstrap_resolves_to_a_ready_disabled_service_by_default() {
        // The C0 default config has no LLM configured on this box
        // (provider = "none"), so bootstrap must resolve to `Ready` with a
        // disabled `CopilotService` - never `Failed` (see this module's doc
        // comment for why that is provably infallible here) and never stuck
        // in `Bootstrapping`.
        match bootstrap().await {
            CopilotInner::Ready(service) => {
                assert!(!service.is_enabled());
                assert!(service.descriptor().is_none());
                assert!(service.disabled_reason().is_some());
            }
            CopilotInner::Bootstrapping => {
                panic!("bootstrap must resolve past its own pending state")
            }
            CopilotInner::Failed(reason) => {
                panic!("expected the infallible default config to succeed, got: {reason}")
            }
        }
    }
}
