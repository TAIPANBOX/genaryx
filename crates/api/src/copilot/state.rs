//! Copilot-panel Tauri managed state (Phase 6 - docs/PHASE6.md,
//! itrat-console/13): the `genaryx-copilot` `CopilotService`, behind the
//! same non-blocking `pending()`-then-spawn-`bootstrap()` shape every other
//! panel's state module uses (mirrors `identity::state` most closely).
//!
//! C0 built `CopilotConfig::default()` (provider "none", the honest "no LLM
//! configured on this box" state) over `Clients::default()` (no connector
//! clients at all - the disabled service never calls a tool, so there was
//! nothing to wire yet). C1 (docs/PHASE6-C1.md C1-W2) still builds
//! `CopilotConfig::default()` - this shell has no `[copilot]` config source
//! wired in yet, so the service itself stays disabled by default exactly
//! like C0 - but [`bootstrap`] now resolves a REAL [`Clients`] by reusing
//! each existing panel's own `env::discover()` (`crate::money`/`identity`/
//! `policy`/`crypto`/`quality`/`memory`) instead of `Clients::default()`, so
//! the day a provider IS configured, Felyx's tools already have real
//! connectors to read through rather than needing yet another wiring pass.
//! See [`resolve_clients`] for exactly how each plane is wired, or honestly
//! left `None`. Building the (still-disabled-by-default) service stays
//! provably infallible either way (see [`bootstrap`]'s doc comment), but
//! this module still goes through the same `Bootstrapping -> Ready`
//! background-task shape as every other panel, matching the non-blocking
//! contract every plane's own discovery already keeps.
//!
//! `CopilotService` is `Send + Sync` but NOT `Clone` (it owns a
//! `Box<dyn LlmProvider>` internally), so unlike `IdentityClient`/
//! `PolicyClient` (which derive `Clone` by wrapping an already-`Arc`ed
//! connector client), this module wraps the WHOLE service in an `Arc`
//! itself: `CopilotInner::Ready(Arc<CopilotService>)`. That lets
//! `super::commands`'s helper clone the cheap `Arc` handle out of the state
//! lock exactly the way every other panel clones its own `XxxClient` out -
//! no extra inner mutex is needed beyond that, since every method this panel
//! calls (`is_enabled`/`descriptor`/`disabled_reason`/`ask`/
//! `explain_incident`) takes `&self`, so concurrent callers just share the
//! one `Arc<CopilotService>` directly.

use genaryx_connectors::{CloudClient, EngramClient, IdryxClient, WardryxClient};
use genaryx_copilot::{Clients, CopilotConfig, CopilotService, TokenfuseTraces};
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

/// Build the copilot service: [`CopilotConfig::default`] (provider "none" -
/// this shell has no `[copilot]` config source yet, so the service stays
/// disabled by default exactly like C0) over a REAL [`Clients`] resolved by
/// [`resolve_clients`] (docs/PHASE6-C1.md C1-W2) rather than C0's
/// `Clients::default()`. Construction itself stays provably infallible for
/// this `config` regardless of what `clients` holds (see
/// [`CopilotInner::Failed`]'s doc comment: `provider = none` short-circuits
/// `build_provider` to `Ok(None)` before `clients` is ever inspected), but
/// still an `async fn` run through `tauri::async_runtime::spawn` from
/// `lib.rs`'s `setup`, matching every other panel's non-blocking shape - see
/// [`resolve_clients`]'s doc comment for why resolving `clients` itself
/// stays cheap (no network round trip) for every plane except Engram. Never
/// panics.
pub async fn bootstrap() -> CopilotInner {
    let config = CopilotConfig::default();
    let clients = resolve_clients().await;
    match CopilotService::from_config_and_clients(&config, clients) {
        Ok(service) => CopilotInner::Ready(Arc::new(service)),
        Err(e) => CopilotInner::Failed(e.to_string()),
    }
}

/// Build the [`Clients`] Felyx's tools read through, reusing the SAME
/// environment discovery each existing panel's own `env::discover` already
/// performs (docs/PHASE6-C1.md C1-W2) rather than inventing a new resolution
/// path here. `cloud`/`idryx`/`wardryx` are the explain flow's backbone
/// (`CopilotService::explain_incident`'s prompt names `alerts`/`list_runs`/
/// `identity_alerts`/`policies` by tool - crate-side tool names over these
/// three clients); `qryx_bin`/`verdryx_db` are cheap, well-known paths wired
/// the same way the Crypto/Quality panels resolve them; `tokenfuse` (I10
/// "Felyx optimization recommendations" - `savings_breakdown`/
/// `cost_per_action`) is likewise a cheap, well-known bin+dir pair, reusing
/// the EVIDENCE Center's own resolution rather than a fresh one - see
/// [`resolve_tokenfuse`]; `engram` is the one heavier, genuinely optional
/// plane (it spawns and handshakes a real child process - see
/// [`resolve_engram`]). Every step here is independent and
/// best-effort: a plane that does not resolve is simply left `None` in the
/// returned [`Clients`], so `genaryx_copilot::ToolRegistry::new` just does
/// not advertise that plane's tools - never a bootstrap failure
/// ([`CopilotInner::Failed`] stays reserved for a genuine
/// `CopilotService::from_config_and_clients` construction error, unrelated
/// to which planes happened to resolve).
///
/// Deliberately does NOT reuse the live `MoneyState`/`IdentityState`/
/// `PolicyState`/`CryptoState`/`QualityState`/`MemoryState` `tauri::State`
/// handles those panels' own `bootstrap`s already build: this task runs
/// concurrently with (no ordering guarantee relative to) every other panel's
/// own background bootstrap (see `lib.rs`'s `setup`), so depending on
/// another panel's ALREADY-RESOLVED state here would be a race. Calling each
/// plane's `env::discover()` fresh is cheap (local filesystem/JSON only,
/// mirrors every `env.rs` module's own "never touches the network"
/// contract), and building a client from the result is likewise free of any
/// network round trip (`CloudClient::new`/`IdryxClient::new`/
/// `WardryxClient::new` only build an HTTP client - no pairing, no healthz
/// probe here). A plane that resolves here but turns out to be genuinely
/// unreachable simply fails at first TOOL CALL, surfaced to the model as
/// normal tool-error data - the same fail-closed-as-data contract every read
/// tool already has (`genaryx_copilot::tools::ToolError::Connector`).
async fn resolve_clients() -> Clients {
    let cloud = crate::money::env::discover()
        .and_then(|env| CloudClient::new(env.cloud_url, env.admin_bearer).ok());

    let idryx =
        crate::identity::env::discover().and_then(|env| IdryxClient::new(env.idryx_url).ok());

    let wardryx = crate::policy::env::discover()
        .and_then(|env| WardryxClient::new(env.wardryx_url, env.admin_bearer).ok());

    let qryx_bin = crate::crypto::env::discover().map(|env| env.qryx_bin);
    let verdryx_db = crate::quality::env::discover().map(|env| env.db_path);
    let engram = resolve_engram().await;
    let tokenfuse = resolve_tokenfuse();

    Clients {
        cloud,
        idryx,
        wardryx,
        engram,
        qryx_bin,
        verdryx_db,
        tokenfuse,
    }
}

/// Best-effort tokenfuse-traces wiring (I10 "Felyx optimization
/// recommendations"): reuses the Evidence Center's OWN
/// `evidence::env::discover_tokenfuse` - the SAME `~/.taipan/bin/
/// tokenfuse-gateway` binary resolution plus the SAME `<name>.traces/gateway`
/// default-traces-dir convention Evidence already ground-truthed against a
/// live `taipan up` box (see that module's doc comment for the full
/// derivation; not re-derived here, since it already exists and this is
/// exactly the "reuse each existing panel's own env discovery" pattern
/// [`resolve_clients`] uses for every other plane). Evidence's own
/// `default_traces_dir` is only a STARTING POINT for an operator-editable UI
/// field there (that panel lets the operator override it), but Felyx's tools
/// have no such field to fall back on, so this resolves to `Some` only when
/// a concrete traces dir was actually found on disk - `None` (never a
/// fabricated path) otherwise, leaving `savings_breakdown`/`cost_per_action`
/// simply unadvertised, exactly like every other plane here.
fn resolve_tokenfuse() -> Option<TokenfuseTraces> {
    let resolved = crate::evidence::env::discover_tokenfuse()?;
    let traces_dir = resolved.default_traces_dir?;
    Some(TokenfuseTraces {
        bin: resolved.tokenfuse_bin,
        traces_dir,
    })
}

/// Best-effort Engram wiring: reuses `memory::env::discover` for the SAME
/// binary+store pair the Memory panel itself resolves, then spawns and
/// handshakes a SECOND, independent `engram-mcp` child the same way
/// `memory::state::spawn_client` does - never the Memory panel's own
/// long-lived process (see [`resolve_clients`]'s doc comment for why
/// borrowing another panel's live state is not an option here). A second
/// process is deliberately accepted rather than plumbed through: the
/// alternative (sharing one process across two independent Tauri managed
/// states with no ordering guarantee between their bootstraps) is real
/// cross-panel coupling this codebase otherwise avoids everywhere, while a
/// second `engram-mcp` is cheap to spawn - the real cost
/// (`crates/connectors/src/engram.rs`'s module doc: engram's first `recall`
/// call lazily loads its embedding model) only lands on whichever process
/// actually serves the first `memory_recall`/`memory_why` tool call, not at
/// this spawn+handshake step. Every failure (no binary/store resolved at
/// all, spawn failed, the MCP `initialize` handshake timed out - bounded by
/// `McpStdioClient`'s own fail-closed deadline, never a hang) yields `None`:
/// Engram is explicitly the one OPTIONAL plane for C1 (docs/PHASE6-C1.md),
/// so `memory_recall`/`memory_why` are then just not advertised, never a
/// bootstrap failure.
async fn resolve_engram() -> Option<Arc<std::sync::Mutex<EngramClient>>> {
    let resolved = crate::memory::env::discover()?;
    let spawned = tokio::task::spawn_blocking(move || {
        let db = resolved.db_path.to_string_lossy().into_owned();
        EngramClient::spawn(&resolved.engram_mcp_bin, &db, None)
    })
    .await;
    match spawned {
        Ok(Ok(client)) => Some(Arc::new(std::sync::Mutex::new(client))),
        Ok(Err(e)) => {
            eprintln!(
                "genaryx: copilot's own engram-mcp wiring failed, memory_* tools will not be \
                 advertised (the Memory panel's own connection is unaffected): {e}"
            );
            None
        }
        Err(join_err) => {
            eprintln!("genaryx: copilot's engram bootstrap task failed to run: {join_err}");
            None
        }
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

    #[tokio::test]
    async fn resolve_clients_never_panics() {
        // Best-effort, like every other `~/.taipan`-dependent resolution in
        // this codebase (see e.g.
        // `identity::state::resolve_idryx_bin_never_panics`): only proves
        // this resolves to a `Clients` value without panicking or hanging,
        // regardless of this box's actual local environment - whether that
        // yields every plane, none of them, or anything in between is a
        // property of this machine's `~/.taipan` state, not this test.
        let _ = resolve_clients().await;
    }

    #[test]
    fn resolve_tokenfuse_never_panics() {
        // Same best-effort discipline as `resolve_clients_never_panics`: this
        // box may or may not have a `~/.taipan/bin/tokenfuse-gateway` plus a
        // resolvable traces dir; either way `resolve_tokenfuse` must return a
        // clean `Option`, never panic.
        let resolved = resolve_tokenfuse();
        // If it DID resolve on this dev box, sanity-check the shape rather
        // than asserting nothing at all - both fields should be non-empty
        // paths, not a fabricated placeholder.
        if let Some(tf) = resolved {
            assert!(!tf.bin.as_os_str().is_empty());
            assert!(!tf.traces_dir.as_os_str().is_empty());
        }
    }
}
