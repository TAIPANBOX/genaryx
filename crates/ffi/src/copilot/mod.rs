//! `CopilotHandle`: the UniFFI Object wrapping `genaryx_copilot::CopilotService`
//! for the SwiftUI Copilot surface (docs/PHASE6.md C0-W2, "Track B (SwiftUI,
//! `crates/ffi` + `apps/macos`): a `CopilotHandle`... exposing
//! `descriptor()` + `ask()`"), at parity with the Tauri shell's own
//! `#[tauri::command]` pair (`copilot_descriptor`/`copilot_ask`, the sibling
//! Track A).
//!
//! ## Simpler than every other handle: no environment of ITS OWN to discover
//!
//! Every other Object in this crate resolves an external plane first
//! (`taipan up`, an env var, a well-known binary path) because it talks to a
//! service that lives somewhere else. Felyx has no such plane: the service
//! itself is built once, locally, from `CopilotConfig::default()`
//! (`provider = "none"`) - only the tools it may call, wired in
//! [`build_clients`] (see "C1: real `Clients`, still no environment of
//! Felyx's own" below), reach out to other planes. So
//! [`CopilotHandle::create`] is a single, always-succeeding constructor (bar
//! a local runtime-allocation failure) - there is no `discover()`/`connect()`
//! split, no `EnvSource` enum, and no `NoEnvironment` variant anywhere in
//! this module: an unconfigured copilot is not an absent plane, it is a
//! normal, fully-constructed, honestly *disabled* [`CopilotService`] (see
//! that type's own doc comment), which [`CopilotHandle::status`] reports
//! directly rather than refusing to build at all.
//!
//! ## Owns a runtime for the same reason Wardryx/Idryx/Cloud do
//!
//! `CopilotService::ask`/`explain_incident` are `async` (each may run a
//! provider HTTP round trip plus any tool calls), but every UniFFI-exported
//! method on this crate's Objects is synchronous. So, exactly like
//! [`crate::wardryx::WardryxHandle`] and [`crate::idryx::IdryxHandle`], this
//! handle owns one `tokio::runtime::Runtime` (built once, in
//! [`CopilotHandle::create`]) and bridges with `self.runtime.block_on(...)`
//! per call - see those modules' own doc comments for the fuller rationale
//! (identical shape, not repeated here).
//!
//! ## C1: real `Clients`, still no environment of Felyx's own
//!
//! [`CopilotHandle::create`] now builds [`build_clients`] instead of
//! `Clients::default()` (docs/PHASE6-C1.md C1-W2: "Wire real `Clients` at
//! copilot bootstrap by REUSING each shell's existing env discovery"), so
//! once an operator DOES configure a provider, Felyx's read tools have real
//! data to call. "No environment of Felyx's own" (the section above) still
//! holds: [`build_clients`] invents nothing new - it calls the exact same
//! [`crate::cloud::env`]/[`crate::idryx::env`]/[`crate::wardryx::env`]/
//! [`crate::crypto::env`]/[`crate::quality::env`] discovery the
//! Money/Identity/Policy/Crypto/Quality panels already resolve their own
//! handles from, so Felyx's tools see exactly the stack the operator is
//! already looking at elsewhere in this console, never a second,
//! independently-resolved environment. Every plane is independently
//! best-effort (`cloud`/`idryx`/`wardryx` are the explain backbone;
//! `qryx_bin`/`verdryx_db` are cheap-and-nice): one plane failing to
//! resolve, or its connector failing to construct, only removes THAT
//! plane's tools (`ToolRegistry::new`'s own "only tools whose backing client
//! is present" contract) - it never fails this constructor, which still
//! only fails on a genuine local runtime-allocation problem, exactly as
//! before C1. `engram` is deliberately left `None` here - see
//! [`build_clients`]'s own doc for why.
//!
//! Fail-closed at the boundary (06 §0.5): nothing here panics across FFI;
//! [`CopilotHandle::ask`] against a disabled service returns the honest
//! [`CopilotFfiError::NoProvider`], never a fabricated answer.

pub mod dto;

pub use dto::{CopilotAnswerDto, CopilotFfiError, CopilotStatusDto, CopilotToolDto};

use genaryx_copilot::{Clients, CopilotConfig, CopilotService};

// C1 (docs/PHASE6-C1.md C1-W2): the connector clients `build_clients` wires,
// plus the SAME per-plane environment discovery every sibling handle in this
// crate already uses for that plane - see `build_clients`'s own doc.
use genaryx_connectors::{CloudClient, IdryxClient, WardryxClient};

use crate::cloud::env as cloud_env;
use crate::crypto::env as crypto_env;
use crate::idryx::env as idryx_env;
use crate::quality::env as quality_env;
use crate::wardryx::env as wardryx_env;

/// The Copilot UniFFI Object: an owned async runtime plus a ready
/// [`CopilotService`] (enabled or honestly disabled - see the module doc).
#[derive(uniffi::Object)]
pub struct CopilotHandle {
    runtime: tokio::runtime::Runtime,
    service: CopilotService,
}

#[uniffi::export]
impl CopilotHandle {
    /// Build the copilot: [`config_from_env`] (the operator's
    /// `GENARYX_COPILOT_*` config, or `provider = "none"` when unset) plus
    /// [`build_clients`] (C1: the real connector clients this box's
    /// environment resolves - see the module doc's "C1: real `Clients`"
    /// section). Fails only on a local runtime-allocation problem
    /// ([`CopilotFfiError::Failed`]) or a bad provider config
    /// ([`CopilotFfiError::Config`], e.g. a non-local endpoint without the
    /// remote opt-in) - never on "no provider set", which is
    /// the normal disabled state [`Self::status`] reports instead of a
    /// construction failure, and never on a plane this box simply does not
    /// have running (that plane's tools are just not advertised).
    #[uniffi::constructor]
    pub fn create() -> Result<Self, CopilotFfiError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| CopilotFfiError::Failed {
                reason: format!("could not start async runtime: {e}"),
            })?;

        let service = CopilotService::from_config_and_clients(&config_from_env(), build_clients())?;

        Ok(Self { runtime, service })
    }

    /// The residency banner's data: enabled + descriptor when a provider is
    /// configured, or the honest disabled reason when it is not (docs/PHASE6.md
    /// C0-W2: "the residency banner"). Never blocks - `CopilotService::descriptor`/
    /// `is_enabled`/`disabled_reason` are plain in-memory reads, no I/O - so
    /// unlike every other method on this handle this does not run through
    /// [`Self::runtime`].
    pub fn status(&self) -> CopilotStatusDto {
        CopilotStatusDto::from(&self.service)
    }

    /// Answer one question. Blocks on the owned runtime (`ask` is async - see
    /// the module doc); returns [`CopilotFfiError::NoProvider`] when
    /// disabled, exactly the same "no provider configured" outcome
    /// [`Self::status`] already names, surfaced here as an honest error
    /// rather than a fabricated answer.
    pub fn ask(&self, question: String) -> Result<CopilotAnswerDto, CopilotFfiError> {
        let answer = self.runtime.block_on(self.service.ask(&question))?;
        Ok(CopilotAnswerDto::from(answer))
    }

    /// The C1 "Explain with Felyx" affordance (docs/PHASE6-C1.md): a focused
    /// `ask` that seeds the loop with `incident_id` and asks for a
    /// cross-plane root-cause chain (the run's spend trajectory, the agent's
    /// identity posture, any governing policy, and prior memory rulings),
    /// citing the specific row ids relied on - see
    /// `genaryx_copilot::CopilotService::explain_incident`'s own doc for the
    /// exact seeded prompt. Blocks on the owned runtime exactly like
    /// [`Self::ask`] (which this simply wraps with a different prompt) and
    /// fails the same way - [`CopilotFfiError::NoProvider`] when disabled,
    /// never a fabricated chain.
    pub fn explain(&self, incident_id: String) -> Result<CopilotAnswerDto, CopilotFfiError> {
        let answer = self
            .runtime
            .block_on(self.service.explain_incident(&incident_id))?;
        Ok(CopilotAnswerDto::from(answer))
    }
}

// ---- private helpers (not exported over FFI) -------------------------------

/// Build the real connector [`Clients`] for [`CopilotHandle::create`] (C1,
/// docs/PHASE6-C1.md C1-W2) - see the module doc's "C1: real `Clients`"
/// section for the high-level contract. Each plane is resolved and built
/// independently; a `None` from that plane's own `env::discover()`, or a
/// `Result::Err` from its connector's own `::new`, both simply leave that
/// field `None` (logged to stderr so a genuinely surprising failure is at
/// least visible, never silently swallowed) - this function itself is
/// infallible, matching [`CopilotHandle::create`]'s own "never fails over an
/// absent plane" contract.
///
/// `cloud`/`idryx`/`wardryx` are the explain backbone (money spend, identity
/// posture, governing policy - the three planes
/// `CopilotService::explain_incident`'s own seeded prompt names by tool).
/// `qryx_bin`/`verdryx_db` are cheap-and-nice: both are plain path
/// resolution (`crypto::env`/`quality::env` never touch the network or spawn
/// anything - see each module's own doc), so there is no reason not to wire
/// them too.
///
/// `engram` stays `None`. `crates/copilot`'s `Clients` shape carries a slot
/// for it (C1-W1's sync-tool bridge already handles a configured one), but
/// wiring it here is a fundamentally heavier step than the other five
/// fields: [`genaryx_connectors::EngramClient`] has no unconnected
/// "just build a client" constructor the way `CloudClient`/`IdryxClient`/
/// `WardryxClient` do - the only way to get one is
/// `genaryx_connectors::EngramClient::spawn`, a REAL `engram-mcp` child
/// process plus an MCP handshake (see [`crate::memory::MemoryHandle`]'s own
/// module doc: "spawn happens IN THE CONSTRUCTOR"). `CopilotHandle::create`
/// runs eagerly at app launch (`GenaryxApp`'s `@State private var
/// copilotModel = CopilotModel()`, constructed alongside every other model),
/// and `CopilotConfig::default()` (`provider = "none"`) is the honest
/// out-of-the-box state on a box with no local model configured yet - in
/// which `CopilotService::from_config_and_clients`'s disabled arm never even
/// builds a `ToolRegistry` (it never touches `clients` at all, so whatever
/// this function built is simply dropped). Spawning a real subprocess on
/// every single launch, in the common case only to drop (and kill) it
/// unused a moment later, is not the "trivial" wiring this wave asks for -
/// see docs/PHASE6-C1.md C1-W2: "`engram` is OPTIONAL (spawns a child) -
/// wire only if `crates/ffi/src/memory` env makes it trivial, else `None`".
/// A follow-up wave that makes this construction lazier (only spawn once a
/// provider is actually configured), or that shares `MemoryModel`'s own
/// already-spawned handle instead of a redundant second child, can revisit
/// this.
/// The operator's copilot config, from the `GENARYX_COPILOT_*` environment
/// (the same names the relay's `copilot_config_from_env` and the
/// `live_felyx_demo` runner use), falling back to `CopilotConfig::default()`
/// (`provider = "none"`, the honest disabled state) when no provider is set.
///
/// This is the wiring the module doc's "once a future wave lets an operator
/// configure a real provider here" anticipated. The residency gate still runs
/// at construction in `CopilotService::from_config_and_clients`, so a non-local
/// endpoint without `GENARYX_COPILOT_ALLOW_REMOTE=1` fails closed there, not
/// here. Local providers (Ollama / LM Studio) keep inference on the machine and
/// need no opt-in; a BYO-cloud provider needs the explicit remote opt-in.
fn config_from_env() -> CopilotConfig {
    use genaryx_copilot::ProviderKind;
    let provider = match std::env::var("GENARYX_COPILOT_PROVIDER")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "ollama" => ProviderKind::Ollama,
        "lmstudio" => ProviderKind::LmStudio,
        "openai_compat" => ProviderKind::OpenAiCompat,
        "anthropic" => ProviderKind::Anthropic,
        "openrouter" => ProviderKind::OpenRouter,
        // Unset or unrecognized: the honest disabled default (provider = none).
        _ => return CopilotConfig::default(),
    };
    CopilotConfig {
        provider,
        base_url: std::env::var("GENARYX_COPILOT_BASE_URL").ok(),
        model: std::env::var("GENARYX_COPILOT_MODEL").ok(),
        api_key_ref: std::env::var("GENARYX_COPILOT_API_KEY_REF").ok(),
        allow_non_local_endpoints: std::env::var("GENARYX_COPILOT_ALLOW_REMOTE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
        // Enough room to gather across planes then answer without truncating the
        // final turn (a `max_tokens`-cut last turn reads as a blank reply).
        max_iterations: 8,
        max_tokens: 2048,
        ..CopilotConfig::default()
    }
}

fn build_clients() -> Clients {
    Clients {
        cloud: build_cloud_client(),
        idryx: build_idryx_client(),
        wardryx: build_wardryx_client(),
        engram: None,
        qryx_bin: crypto_env::discover().map(|resolved| resolved.qryx_bin),
        verdryx_db: quality_env::discover().map(|resolved| resolved.db_path),
    }
}

/// Resolve + build the money plane's [`CloudClient`], exactly the same
/// [`crate::cloud::env::discover`] the Money/Overview panel's own
/// `CloudHandle::discover` calls. UNLIKE `CloudHandle::build`, this never
/// pairs a device (`CloudClient::new` alone never touches the network - its
/// own doc comment): every Cloud tool Felyx has is a read
/// (`crates/copilot/src/tools/cloud.rs`), so there is nothing here that
/// needs a signer.
fn build_cloud_client() -> Option<CloudClient> {
    let resolved = cloud_env::discover()?;
    match CloudClient::new(resolved.cloud_url, resolved.admin_bearer) {
        Ok(client) => Some(client),
        Err(e) => {
            eprintln!(
                "genaryx-ffi copilot: cloud plane resolved but CloudClient::new failed, \
                 leaving its tools unavailable: {e}"
            );
            None
        }
    }
}

/// Resolve + build the identity plane's [`IdryxClient`], exactly the same
/// [`crate::idryx::env::discover`] the Identity panel's own
/// `IdryxHandle::discover` calls.
fn build_idryx_client() -> Option<IdryxClient> {
    let resolved = idryx_env::discover()?;
    match IdryxClient::new(resolved.idryx_url) {
        Ok(client) => Some(client),
        Err(e) => {
            eprintln!(
                "genaryx-ffi copilot: idryx plane resolved but IdryxClient::new failed, \
                 leaving its tools unavailable: {e}"
            );
            None
        }
    }
}

/// Resolve + build the policy plane's [`WardryxClient`], exactly the same
/// [`crate::wardryx::env::discover`] the Policy panel's own
/// `WardryxHandle::discover` calls.
fn build_wardryx_client() -> Option<WardryxClient> {
    let resolved = wardryx_env::discover()?;
    match WardryxClient::new(resolved.wardryx_url, resolved.admin_bearer) {
        Ok(client) => Some(client),
        Err(e) => {
            eprintln!(
                "genaryx-ffi copilot: wardryx plane resolved but WardryxClient::new failed, \
                 leaving its tools unavailable: {e}"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The C0 default config (`provider = "none"`) must build a working,
    /// honestly-disabled handle - never fail construction and never panic.
    /// Mirrors every other handle's own "construction never mishandles an
    /// absent plane" tests, adapted to Copilot's own "absent plane" being a
    /// disabled service rather than a `NoEnvironment` error (see the module
    /// doc).
    #[test]
    fn create_with_the_default_config_builds_a_disabled_handle() {
        let handle = CopilotHandle::create().expect("create() must succeed with provider=none");
        let status = handle.status();
        assert!(!status.enabled);
        assert!(status.provider.is_none());
        assert!(status.model.is_none());
        assert!(status.endpoint.is_none());
        assert!(status.local.is_none());
        assert!(
            status.disabled_reason.is_some(),
            "a disabled service must always explain why"
        );
    }

    /// `ask()` against the disabled C0 default must fail closed with
    /// `NoProvider`, never panic and never fabricate an answer.
    #[test]
    fn ask_against_a_disabled_handle_is_no_provider_not_a_panic() {
        let handle = CopilotHandle::create().expect("create() must succeed");
        match handle.ask("how are we doing?".to_string()) {
            Err(CopilotFfiError::NoProvider) => {}
            other => panic!("expected CopilotFfiError::NoProvider, got {other:?}"),
        }
    }

    /// `explain()` against the disabled default must also fail closed with
    /// `NoProvider`, never panic and never fabricate a root-cause chain -
    /// `explain_incident` is just a focused `ask`
    /// (`genaryx_copilot::CopilotService::explain_incident`'s own doc), so it
    /// inherits `ask`'s identical disabled behavior, mirrored here at the FFI
    /// boundary exactly like the test above.
    #[test]
    fn explain_against_a_disabled_handle_is_no_provider_not_a_panic() {
        let handle = CopilotHandle::create().expect("create() must succeed");
        match handle.explain("budget_exhausted:reconciliation-batch".to_string()) {
            Err(CopilotFfiError::NoProvider) => {}
            other => panic!("expected CopilotFfiError::NoProvider, got {other:?}"),
        }
    }

    /// `build_clients` must never panic regardless of this box's actual
    /// environment (a real `~/.taipan` descriptor, none at all, a qryx/
    /// verdryx path that does or does not exist, ...) - proven directly here
    /// rather than only indirectly through `create()`, since a future change
    /// to this function is far more likely to be exercised by a test that
    /// names it than one that only exercises `CopilotHandle::create` as a
    /// whole. Every field is independently `Option`-shaped by construction
    /// (see the type's own fields), so there is nothing to assert about
    /// WHICH planes resolved on whatever box happens to run this - only that
    /// building the value at all completes.
    #[test]
    fn build_clients_never_panics_regardless_of_this_boxs_environment() {
        let _ = build_clients();
    }

    /// `status()` is callable repeatedly and agrees with itself - a cheap
    /// proof it really is the plain in-memory read the module doc claims
    /// (no hidden mutation, no toggling between calls).
    #[test]
    fn status_is_stable_across_repeated_calls() {
        let handle = CopilotHandle::create().expect("create() must succeed");
        let first = handle.status();
        let second = handle.status();
        assert_eq!(first.enabled, second.enabled);
        assert_eq!(first.disabled_reason, second.disabled_reason);
    }
}
