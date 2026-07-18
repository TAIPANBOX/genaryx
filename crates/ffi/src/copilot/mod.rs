//! `CopilotHandle`: the UniFFI Object wrapping `genaryx_copilot::CopilotService`
//! for the SwiftUI Copilot surface (docs/PHASE6.md C0-W2, "Track B (SwiftUI,
//! `crates/ffi` + `apps/macos`): a `CopilotHandle`... exposing
//! `descriptor()` + `ask()`"), at parity with the Tauri shell's own
//! `#[tauri::command]` pair (`copilot_descriptor`/`copilot_ask`, the sibling
//! Track A).
//!
//! ## Simpler than every other handle: no environment to discover
//!
//! Every other Object in this crate resolves an external plane first
//! (`taipan up`, an env var, a well-known binary path) because it talks to a
//! service that lives somewhere else. Felyx has no such plane in C0: the
//! service is built once, locally, from `CopilotConfig::default()`
//! (`provider = "none"`) and `Clients::default()` (see "No connectors wired
//! in C0" below). So [`CopilotHandle::create`] is a single, always-succeeding
//! constructor (bar a local runtime-allocation failure) - there is no
//! `discover()`/`connect()` split, no `EnvSource` enum, and no
//! `NoEnvironment` variant anywhere in this module: an unconfigured copilot
//! is not an absent plane, it is a normal, fully-constructed, honestly
//! *disabled* [`CopilotService`] (see that type's own doc comment), which
//! [`CopilotHandle::status`] reports directly rather than refusing to build
//! at all.
//!
//! ## Owns a runtime for the same reason Wardryx/Idryx/Cloud do
//!
//! `CopilotService::ask` is `async` (it may run a provider HTTP round trip
//! plus any tool calls), but every UniFFI-exported method on this crate's
//! Objects is synchronous. So, exactly like [`crate::wardryx::WardryxHandle`]
//! and [`crate::idryx::IdryxHandle`], this handle owns one
//! `tokio::runtime::Runtime` (built once, in [`CopilotHandle::create`]) and
//! bridges with `self.runtime.block_on(...)` per call - see those modules'
//! own doc comments for the fuller rationale (identical shape, not repeated
//! here).
//!
//! ## No connectors wired in C0
//!
//! [`CopilotHandle::create`] passes `Clients::default()` (every connector
//! client field `None`), so [`CopilotService::ask`] has no read tool to call
//! even once a provider IS configured - matching this crate's own C0 scope
//! (docs/PHASE6.md: "Desktop-only, read-only, no proposals, no relay").
//! Wiring the existing `CloudHandle`/`IdryxHandle`/`WardryxHandle` connector
//! clients into `genaryx_copilot::Clients` (so Felyx's read tools have real
//! data to call) is later-wave work, not this one's.
//!
//! Fail-closed at the boundary (06 §0.5): nothing here panics across FFI;
//! [`CopilotHandle::ask`] against a disabled service returns the honest
//! [`CopilotFfiError::NoProvider`], never a fabricated answer.

pub mod dto;

pub use dto::{CopilotAnswerDto, CopilotFfiError, CopilotStatusDto, CopilotToolDto};

use genaryx_copilot::{Clients, CopilotConfig, CopilotService};

/// The Copilot UniFFI Object: an owned async runtime plus a ready
/// [`CopilotService`] (enabled or honestly disabled - see the module doc).
#[derive(uniffi::Object)]
pub struct CopilotHandle {
    runtime: tokio::runtime::Runtime,
    service: CopilotService,
}

#[uniffi::export]
impl CopilotHandle {
    /// Build the C0 copilot: `CopilotConfig::default()` (`provider = "none"`)
    /// plus `Clients::default()` (no connector clients - see the module
    /// doc's "No connectors wired in C0"). Fails only on a local
    /// runtime-allocation problem ([`CopilotFfiError::Failed`]) or, once a
    /// future wave lets an operator configure a real provider here, a bad
    /// config ([`CopilotFfiError::Config`]) - never on "no provider set",
    /// which is the normal disabled state [`Self::status`] reports instead
    /// of a construction failure.
    #[uniffi::constructor]
    pub fn create() -> Result<Self, CopilotFfiError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| CopilotFfiError::Failed {
                reason: format!("could not start async runtime: {e}"),
            })?;

        let service =
            CopilotService::from_config_and_clients(&CopilotConfig::default(), Clients::default())?;

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
