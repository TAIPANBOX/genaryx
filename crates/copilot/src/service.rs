//! `CopilotService`: the one assembly entry point the hosts call, so the shells
//! (Tauri commands, the FFI handle) stay thin (06 §0.9). It turns a
//! [`CopilotConfig`] plus the connector [`Clients`] into a ready [`Felyx`], or a
//! disabled service when `provider = "none"` (the honest default on a box with
//! no local model configured).
//!
//! The residency gate still runs inside [`build_provider`], so a misconfigured
//! non-local endpoint fails HERE, at construction, not at first use.

use crate::agent::{Answer, CopilotError, Felyx};
use crate::config::{ConfigError, CopilotConfig};
use crate::provider::{ProviderDescriptor, build_provider};
use crate::tools::{Clients, ToolRegistry};

pub struct CopilotService {
    felyx: Option<Felyx>,
    /// Retained so the shell can render a "no provider configured" banner that
    /// still names the residency posture even when disabled.
    disabled_reason: Option<String>,
}

impl CopilotService {
    /// Assemble the service. `Ok` with a disabled service when the provider is
    /// `none`; `Err` only when a configured provider is invalid (e.g. a
    /// non-local endpoint without opt-in, or a missing key/model).
    pub fn from_config_and_clients(
        config: &CopilotConfig,
        clients: Clients,
    ) -> Result<Self, ConfigError> {
        match build_provider(config)? {
            Some(provider) => {
                let registry = ToolRegistry::new(clients);
                let felyx =
                    Felyx::new(provider, registry, config.max_iterations, config.max_tokens);
                Ok(Self {
                    felyx: Some(felyx),
                    disabled_reason: None,
                })
            }
            None => Ok(Self {
                felyx: None,
                disabled_reason: Some(
                    "No copilot provider is configured. Set a local provider (Ollama / LM Studio) \
                     to keep inference on this machine, or a BYO-key cloud provider."
                        .to_string(),
                ),
            }),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.felyx.is_some()
    }

    /// The residency descriptor for the banner, or `None` when disabled.
    pub fn descriptor(&self) -> Option<ProviderDescriptor> {
        self.felyx.as_ref().map(Felyx::descriptor)
    }

    pub fn disabled_reason(&self) -> Option<&str> {
        self.disabled_reason.as_deref()
    }

    /// Answer one question, or [`CopilotError::NoProvider`] when disabled.
    pub async fn ask(&self, question: &str) -> Result<Answer, CopilotError> {
        match &self.felyx {
            Some(felyx) => felyx.answer(question).await,
            None => Err(CopilotError::NoProvider),
        }
    }

    /// The C1 cross-plane "explain" flow (docs/PHASE6-C1.md, itrat-console/13
    /// D13.7 C1 / D13.4's example chain): seed the loop with an incident-focused
    /// instruction so it gathers the money, policy, and identity evidence plus
    /// any prior ruling, then gives a root-cause chain with cited row ids. It is
    /// just a focused [`Self::ask`]: the whole capability is the tools plus this
    /// prompt, not new machinery. The tools it names are only run if their plane
    /// is configured; whatever is missing, the model works with what it has.
    pub async fn explain_incident(&self, incident_id: &str) -> Result<Answer, CopilotError> {
        let prompt = format!(
            "Explain incident `{incident_id}` as a cross-plane root-cause chain. Work through \
             the tools you have: use `incidents` to find it; `alerts` and `list_runs` for the \
             affected run's spend trajectory; `identity_alerts` for the agent's identity posture; \
             `policies` for any governing policy; and `memory_recall` to check whether this \
             pattern was ruled on before (e.g. a past false alarm). Then give a SHORT root-cause \
             chain (cause -> effect -> effect) and one recommended action, citing the specific \
             run / incident / policy ids you relied on. You can recommend but not act; a human \
             must approve and sign any change."
        );
        self.ask(&prompt).await
    }

    /// C3 push annotation (docs/PHASE6-C3.md): a fast, tool-free one-line summary
    /// of a pager event for the relay's triage stage, or `None` when the copilot
    /// is disabled. The relay wraps this in its own latency budget and never lets
    /// it block or suppress a HARD push (the deterministic floor dispatches first).
    pub async fn annotate(
        &self,
        event: &str,
    ) -> Result<Option<crate::action::CopilotAnnotation>, CopilotError> {
        match &self.felyx {
            Some(felyx) => felyx.annotate(event).await.map(Some),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProviderKind;

    #[test]
    fn provider_none_yields_a_disabled_service() {
        let cfg = CopilotConfig::default(); // provider = none
        let svc = CopilotService::from_config_and_clients(&cfg, Clients::default()).unwrap();
        assert!(!svc.is_enabled());
        assert!(svc.descriptor().is_none());
        assert!(svc.disabled_reason().is_some());
    }

    #[tokio::test]
    async fn disabled_service_ask_is_no_provider() {
        let svc =
            CopilotService::from_config_and_clients(&CopilotConfig::default(), Clients::default())
                .unwrap();
        assert!(matches!(svc.ask("hi").await, Err(CopilotError::NoProvider)));
    }

    #[tokio::test]
    async fn explain_incident_on_a_disabled_service_is_no_provider() {
        // The C1 explain flow is a focused `ask`, so it inherits the same
        // disabled behavior: no provider -> NoProvider, never a fabricated chain.
        let svc =
            CopilotService::from_config_and_clients(&CopilotConfig::default(), Clients::default())
                .unwrap();
        assert!(matches!(
            svc.explain_incident("budget_exhausted:reconciliation-batch")
                .await,
            Err(CopilotError::NoProvider)
        ));
    }

    #[tokio::test]
    async fn annotate_on_a_disabled_service_is_none() {
        // C3: a disabled copilot yields no annotation (the relay then pushes the
        // HARD event plain - the deterministic floor never depends on the AI).
        let svc =
            CopilotService::from_config_and_clients(&CopilotConfig::default(), Clients::default())
                .unwrap();
        assert!(svc.annotate("run r-1 over cap").await.unwrap().is_none());
    }

    #[test]
    fn a_non_local_provider_without_opt_in_fails_at_construction() {
        let cfg = CopilotConfig {
            provider: ProviderKind::Anthropic,
            model: Some("claude-sonnet-5".into()),
            api_key_ref: Some("env:GENARYX_COPILOT_NONEXISTENT".into()),
            allow_non_local_endpoints: false, // the gate
            ..Default::default()
        };
        // The missing key OR the residency gate must make this fail; either way
        // it never yields a usable non-local service by default.
        assert!(CopilotService::from_config_and_clients(&cfg, Clients::default()).is_err());
    }
}
