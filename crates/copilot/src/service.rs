//! `CopilotService`: the one assembly entry point the hosts call, so every
//! shell stays thin (06 §0.9) - today genaryx-api's console commands; before
//! the desktop shells were removed, Tauri commands and the FFI handle. It
//! turns a [`CopilotConfig`] plus the connector [`Clients`] into a ready
//! [`Felyx`], or a disabled service when `provider = "none"` (the honest
//! default on a box with no local model configured).
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

    /// A fast, tool-free one-line summary of one event, or `None` when the
    /// copilot is disabled. A caller is expected to wrap this in its own
    /// latency budget and to never let it block or suppress the alert itself:
    /// the deterministic path dispatches first, this only enriches it.
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
        // A disabled copilot yields no annotation (the caller then sends the
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

    /// LIVE demo-runner (docs/PHASE6-C*): drives Felyx against a REAL provider
    /// and REAL planes, printing the transcript. Ignored by default; run with a
    /// configured provider + reachable planes, e.g.:
    ///   GENARYX_COPILOT_PROVIDER=anthropic GENARYX_COPILOT_MODEL=claude-sonnet-5 \
    ///   GENARYX_COPILOT_API_KEY_REF=file:/path/key GENARYX_COPILOT_ALLOW_REMOTE=1 \
    ///   GENARYX_DEMO_CLOUD_URL=http://127.0.0.1:8080 GENARYX_DEMO_CLOUD_KEY=devkey \
    ///   cargo test -p genaryx-copilot live_felyx_demo -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "live: needs a real provider (GENARYX_COPILOT_*) + reachable planes"]
    async fn live_felyx_demo() {
        use genaryx_connectors::{CloudClient, IdryxClient, WardryxClient};

        let provider = match std::env::var("GENARYX_COPILOT_PROVIDER")
            .unwrap_or_default()
            .as_str()
        {
            "anthropic" => ProviderKind::Anthropic,
            "ollama" => ProviderKind::Ollama,
            "openrouter" => ProviderKind::OpenRouter,
            "openai_compat" => ProviderKind::OpenAiCompat,
            "lmstudio" => ProviderKind::LmStudio,
            _ => {
                eprintln!("SKIP live_felyx_demo: set GENARYX_COPILOT_PROVIDER");
                return;
            }
        };
        let cfg = CopilotConfig {
            provider,
            base_url: std::env::var("GENARYX_COPILOT_BASE_URL").ok(),
            model: std::env::var("GENARYX_COPILOT_MODEL").ok(),
            api_key_ref: std::env::var("GENARYX_COPILOT_API_KEY_REF").ok(),
            allow_non_local_endpoints: std::env::var("GENARYX_COPILOT_ALLOW_REMOTE")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            // A real fleet needs more room than the conservative defaults: enough
            // iterations to gather across several planes THEN propose, and enough
            // output that the final synthesis turn is never cut mid-answer (a
            // `stop_reason=max_tokens` on the LAST turn is what yields a blank
            // reply, so give the answer generous headroom).
            max_iterations: 8,
            max_tokens: 4096,
            ..Default::default()
        };
        let cloud = std::env::var("GENARYX_DEMO_CLOUD_URL")
            .ok()
            .zip(std::env::var("GENARYX_DEMO_CLOUD_KEY").ok())
            .and_then(|(u, k)| CloudClient::new(u, k).ok());
        let idryx = std::env::var("GENARYX_DEMO_IDRYX_URL")
            .ok()
            .and_then(|u| IdryxClient::new(u).ok());
        let wardryx = std::env::var("GENARYX_DEMO_WARDRYX_URL")
            .ok()
            .zip(std::env::var("GENARYX_DEMO_WARDRYX_KEY").ok())
            .and_then(|(u, k)| WardryxClient::new(u, k).ok());
        let clients = Clients {
            cloud,
            idryx,
            wardryx,
            ..Default::default()
        };
        let svc = CopilotService::from_config_and_clients(&cfg, clients)
            .expect("service must build for the live demo");
        assert!(
            svc.is_enabled(),
            "provider must be enabled for the live demo"
        );
        eprintln!("=== Felyx live: {:?}\n", svc.descriptor());

        // A small printer so each cut (C0 Q&A, C1 explain, C2 propose) reports
        // the answer, the REAL tools the model called, any proposals, and usage.
        let show = |label: &str, r: Result<Answer, CopilotError>| match r {
            Ok(a) => {
                eprintln!("--- [{label}] answer:\n{}", a.text);
                let tools: Vec<&str> = a.tool_trace.iter().map(|t| t.name.as_str()).collect();
                eprintln!("--- tools called: {tools:?}");
                for p in &a.proposals {
                    eprintln!(
                        "--- PROPOSAL: {:?} target={} confidence={:.2}\n    rationale: {}\n    evidence: {:?}",
                        p.kind, p.target, p.confidence, p.rationale, p.evidence_refs
                    );
                }
                eprintln!(
                    "--- usage: {}+{} tokens\n",
                    a.usage.prompt_tokens, a.usage.completion_tokens
                );
            }
            Err(e) => eprintln!("!!! [{label}] error: {e}\n"),
        };

        // C0 - money Q&A: the over-budget / runaway signal for a batch-ingested
        // fleet lives in `incidents` (live burn-rate `alerts` may be empty), so
        // the prompt lets Felyx consult both and cite ids from the real data.
        let q0 = "Which agents or runs have blown their budget or look runaway (stuck loops, \
                  fan-out, repeated budget breaks)? Check `alerts` and `incidents`, and use \
                  `list_runs` for the biggest spenders. Be brief and cite specific ids.";
        eprintln!(">>> [C0 money Q&A] {q0}");
        show("C0", svc.ask(q0).await);

        // C1 - cross-plane explain on a REAL incident id from this dataset.
        let incident = "budget_exhausted:kyc-intake-agent-loop-00";
        eprintln!(">>> [C1 explain_incident] {incident}");
        show("C1", svc.explain_incident(incident).await);

        // C2 - propose (recommend, never act): expect a ProposedAction in
        // `proposals`, which the shell would render as an approve/sign card.
        let q2 = "Given the runaway spend you found, which single run or agent would you \
                  recommend killing to stop the bleed, and why? Propose it with evidence.";
        eprintln!(">>> [C2 propose] {q2}");
        show("C2", svc.ask(q2).await);
    }
}
