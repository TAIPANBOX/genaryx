//! Tauri commands for the Copilot view (Phase 6, C0 - docs/PHASE6.md,
//! itrat-console/13): [`copilot_status`] (a flat DTO for the residency
//! banner) and [`copilot_ask`] (one question/answer round trip through
//! Felyx, the hand-rolled agent loop `genaryx-copilot` owns).
//!
//! Unlike every other panel's status command, [`copilot_status`] returns a
//! flat struct rather than a tagged `#[serde(tag = "state", ...)]` enum:
//! there is no environment to discover and no reachability to probe here
//! (see `state.rs`'s module doc), so the only thing worth telling the
//! frontend is "is the copilot enabled, and if so with what residency" -
//! `CopilotService::is_enabled`/`descriptor`/`disabled_reason` map onto this
//! DTO's fields directly.
//!
//! [`copilot_ask`] returns `Result<Answer, String>`, not a structured error
//! DTO like `IdentityError`/`PolicyError`: `genaryx_copilot::Answer` already
//! derives `Serialize` (mirrors `identity::commands`'s reuse of
//! `genaryx_connectors::Idryx*` DTOs directly - no UI-facing mirror struct
//! needed), and `CopilotError` (`thiserror`-derived `Display`) has exactly
//! one message worth showing an operator, so its `.to_string()` - e.g.
//! `CopilotError::NoProvider`'s "no copilot provider is configured; set
//! [copilot].provider (local by default)" - IS the error, not a code the
//! frontend has to translate.

use std::sync::Arc;

use genaryx_copilot::{Answer, CopilotService};
use serde::Serialize;

use super::state::{CopilotInner, CopilotState};

// ============================================================================
// DTOs
// ============================================================================

/// The residency banner's whole data model - mirrors
/// `identity::commands::IdentityStatusDto` in spirit (never inferred from a
/// read command's error shape) but flat, not tagged (see this module's doc
/// comment). `provider`/`model`/`endpoint`/`local` are `Some` together
/// exactly when a [`genaryx_copilot::ProviderDescriptor`] exists to show;
/// `disabled_reason` is `Some` exactly when the service is disabled.
#[derive(Debug, Clone, Serialize)]
pub struct CopilotStatusDto {
    pub enabled: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub local: Option<bool>,
    pub disabled_reason: Option<String>,
}

impl CopilotStatusDto {
    /// The disabled shape, used for every non-`Ready` [`CopilotInner`]
    /// (`Bootstrapping`, `Failed`) - `reason` is always operator-readable
    /// text, never a bare error code.
    fn disabled(reason: impl Into<String>) -> Self {
        Self {
            enabled: false,
            provider: None,
            model: None,
            endpoint: None,
            local: None,
            disabled_reason: Some(reason.into()),
        }
    }

    /// Read the live DTO off a resolved [`CopilotService`]: the residency
    /// descriptor plus fields when a provider is configured, or
    /// [`Self::disabled`] with the service's own explanation when not.
    /// `enabled` always comes straight from `is_enabled()` rather than being
    /// inferred from `descriptor().is_some()`, so this stays correct even if
    /// that equivalence ever stops holding on the crate side.
    fn from_service(service: &CopilotService) -> Self {
        let enabled = service.is_enabled();
        match service.descriptor() {
            Some(d) => Self {
                enabled,
                provider: Some(d.provider),
                model: Some(d.model),
                endpoint: Some(d.endpoint),
                local: Some(d.local),
                disabled_reason: service.disabled_reason().map(str::to_string),
            },
            None => Self::disabled(
                service
                    .disabled_reason()
                    .unwrap_or("No copilot provider is configured.")
                    .to_string(),
            ),
        }
    }
}

// ============================================================================
// helpers
// ============================================================================

/// Resolve the current `Arc<CopilotService>` out of managed state, or a
/// plain operator-readable message when the panel is not ready yet - mirrors
/// `identity::commands::ready_client`'s "clone the cheap handle out of the
/// lock" shape, but returns `String` (not a structured error type) since
/// that is [`copilot_ask`]'s own error type (see this module's doc comment).
async fn ready_service(
    state: &tauri::State<'_, CopilotState>,
) -> Result<Arc<CopilotService>, String> {
    let guard = state.inner.lock().await;
    match &*guard {
        CopilotInner::Ready(service) => Ok(Arc::clone(service)),
        CopilotInner::Bootstrapping => {
            Err("Copilot is still starting up; try again in a moment.".to_string())
        }
        CopilotInner::Failed(reason) => Err(reason.clone()),
    }
}

// ============================================================================
// commands
// ============================================================================

/// Whole-panel status for the residency banner. Never fails: every
/// [`CopilotInner`] shape (including `Bootstrapping`/`Failed`) maps onto a
/// renderable [`CopilotStatusDto`].
#[tauri::command]
pub async fn copilot_status(
    state: tauri::State<'_, CopilotState>,
) -> Result<CopilotStatusDto, ()> {
    let guard = state.inner.lock().await;
    Ok(match &*guard {
        CopilotInner::Bootstrapping => CopilotStatusDto::disabled("Copilot is still starting up."),
        CopilotInner::Ready(service) => CopilotStatusDto::from_service(service),
        CopilotInner::Failed(reason) => CopilotStatusDto::disabled(reason.clone()),
    })
}

/// Ask Felyx one question. Runs the bounded agent loop
/// (`genaryx_copilot::Felyx::answer`, via `CopilotService::ask`) over
/// whatever tools are registered for today's `Clients::default()` (none yet
/// in C0 - see `state.rs`'s module doc), so C0 answers come from the model's
/// own text only; `tool_trace` is still always present on the returned
/// [`Answer`] (empty today) so the frontend's evidence rendering needs no
/// change once a later cut wires real clients through.
///
/// `Err` is always operator-readable text, not a code: with today's default
/// config this is virtually always `CopilotError::NoProvider`'s message (no
/// LLM configured on this box) - the frontend renders it as an assistant
/// note rather than treating it as a crash (see `CopilotView.tsx`).
#[tauri::command]
pub async fn copilot_ask(
    state: tauri::State<'_, CopilotState>,
    question: String,
) -> Result<Answer, String> {
    let service = ready_service(&state).await?;
    service.ask(&question).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use genaryx_copilot::{Clients, CopilotConfig};

    #[test]
    fn status_dto_disabled_carries_the_reason_and_no_descriptor_fields() {
        let dto = CopilotStatusDto::disabled("no provider configured");
        assert!(!dto.enabled);
        assert_eq!(
            dto.disabled_reason.as_deref(),
            Some("no provider configured")
        );
        assert!(dto.provider.is_none());
        assert!(dto.model.is_none());
        assert!(dto.endpoint.is_none());
        assert!(dto.local.is_none());
    }

    #[test]
    fn status_dto_from_a_disabled_default_service_is_honestly_disabled() {
        let service =
            CopilotService::from_config_and_clients(&CopilotConfig::default(), Clients::default())
                .expect("default config must construct");
        let dto = CopilotStatusDto::from_service(&service);
        assert!(!dto.enabled);
        assert!(dto.disabled_reason.is_some());
        assert!(dto.provider.is_none());
        assert!(dto.model.is_none());
        assert!(dto.endpoint.is_none());
        assert!(dto.local.is_none());
    }
}
