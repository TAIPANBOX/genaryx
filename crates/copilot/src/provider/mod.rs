//! The LLM provider abstraction (docs/PHASE6.md, itrat-console/13 D13.2): one
//! trait, two real wire implementations. `OpenAiCompat` (base_url =
//! `http://127.0.0.1:11434/v1`) IS the Ollama / LM Studio / vLLM / OpenRouter /
//! OpenAI path - one wire format covers them all; `AnthropicMessages` is the
//! Anthropic Messages API. A third impl, `MockProvider`, lives in `mock` behind
//! `cfg(test)` for deterministic loop tests.
//!
//! Every real constructor runs the [`crate::residency`] gate, so a provider that
//! could leak to a public endpoint cannot even be built unless the operator
//! explicitly set `allow_non_local_endpoints = true`.

mod anthropic;
#[cfg(test)]
pub(crate) mod mock;
mod openai;

pub use anthropic::AnthropicMessages;
pub use openai::OpenAiCompat;

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::config::{ConfigError, CopilotConfig, ProviderKind};
use crate::residency::{HostResidency, classify_host};
use crate::resolver::{HostnameLookup, ResidencyDnsResolver, SystemLookup, resolve_all_local};

/// A provider-agnostic chat turn request. `tools` are advertised to the model;
/// the loop, not the provider, decides what to do with any returned tool calls.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: u32,
    pub temperature: f32,
}

/// One conversation message. Tool results ride back as [`Role::Tool`] messages
/// carrying the `tool_call_id` they answer; the system prompt declares all such
/// content as DATA, never instructions (the prompt-injection posture, D13.3).
#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// Set on assistant turns that requested tools (so the wire layer can
    /// reconstruct the provider-native `tool_calls`/`tool_use` blocks).
    pub tool_calls: Vec<ToolCall>,
    /// Set on [`Role::Tool`] messages: which call this result answers.
    pub tool_call_id: Option<String>,
    /// Set on [`Role::Tool`] messages: the tool's name (some wire formats want it).
    pub tool_name: Option<String>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
        }
    }

    pub fn assistant_tool_calls(content: Option<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.unwrap_or_default(),
            tool_calls,
            tool_call_id: None,
            tool_name: None,
        }
    }

    pub fn tool_result(
        call_id: impl Into<String>,
        name: impl Into<String>,
        result: &Value,
    ) -> Self {
        Self {
            role: Role::Tool,
            content: result.to_string(),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
            tool_name: Some(name.into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A tool advertised to the model: name, human description, and a JSON-Schema
/// object for its parameters (empty object for C0's parameterless read tools).
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub params_schema: Value,
}

/// One model-requested tool call. `arguments` is the parsed JSON object the
/// model supplied (`{}` for a parameterless tool).
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// One provider turn: free text and/or a set of tool calls, plus token usage.
#[derive(Debug, Clone, Default)]
pub struct ChatTurn {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

impl std::ops::AddAssign for Usage {
    fn add_assign(&mut self, rhs: Self) {
        self.prompt_tokens = self.prompt_tokens.saturating_add(rhs.prompt_tokens);
        self.completion_tokens = self.completion_tokens.saturating_add(rhs.completion_tokens);
    }
}

/// What the residency banner in the shell renders: where inference runs, and
/// whether it is local (D13.2). `local == true` is the "nothing leaves this
/// box" claim.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderDescriptor {
    pub provider: String,
    pub model: String,
    pub endpoint: String,
    pub local: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error(
        "refusing a non-local provider endpoint ({url}); set allow_non_local_endpoints = true to use a remote (BYO-key) provider"
    )]
    NonLocalEndpointRefused { url: String },
    #[error("provider config: {0}")]
    Config(String),
    #[error("provider transport: {0}")]
    Transport(String),
    #[error("provider returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("could not decode the provider response: {0}")]
    Decode(String),
}

/// The provider contract. Object-safe via `async_trait` so the agent can hold a
/// `Box<dyn LlmProvider>` (a real client, or the test `MockProvider`).
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(&self, req: ChatRequest) -> Result<ChatTurn, ProviderError>;
    fn descriptor(&self) -> ProviderDescriptor;
}

/// Build the configured provider, or `None` when `provider = "none"` (the copilot
/// is present but unconfigured - the shell shows "no provider configured" and
/// the residency banner explains why). Applies the residency gate in every real
/// constructor.
pub fn build_provider(config: &CopilotConfig) -> Result<Option<Box<dyn LlmProvider>>, ConfigError> {
    match config.provider {
        ProviderKind::None => Ok(None),
        ProviderKind::Ollama
        | ProviderKind::LmStudio
        | ProviderKind::OpenAiCompat
        | ProviderKind::OpenRouter => {
            let base_url = config.resolved_base_url()?;
            let model = config.require_model()?;
            let api_key = config.resolve_api_key()?; // Option: local runtimes need none
            let agent_id = config.resolved_agent_id()?;
            let provider = OpenAiCompat::new(
                config.provider,
                base_url,
                model,
                api_key,
                config.allow_non_local_endpoints,
                config.local_hostnames.clone(),
                config.run_id.clone(),
                agent_id,
            )
            .map_err(ConfigError::Provider)?;
            Ok(Some(Box::new(provider)))
        }
        ProviderKind::Anthropic => {
            let base_url = config.resolved_base_url()?;
            let model = config.require_model()?;
            let api_key = config.resolve_api_key()?.ok_or(ConfigError::MissingField(
                "api_key_ref (Anthropic requires a key)",
            ))?;
            let agent_id = config.resolved_agent_id()?;
            let provider = AnthropicMessages::new(
                base_url,
                model,
                api_key,
                config.allow_non_local_endpoints,
                config.local_hostnames.clone(),
                config.run_id.clone(),
                agent_id,
            )
            .map_err(ConfigError::Provider)?;
            Ok(Some(Box::new(provider)))
        }
    }
}

/// What the residency gate decided about a `base_url`, and what the HTTP
/// client needs to keep enforcing it: `local` for the descriptor banner,
/// plus, only for a checked hostname, the custom DNS resolver that re-runs
/// the same check on every connection (invariant 14).
pub(crate) struct ResidencyOutcome {
    pub local: bool,
    pub dns_resolver: Option<ResidencyDnsResolver>,
}

/// The residency gate itself, shared by both real provider constructors.
///
/// A literal IP or `localhost` is decided instantly, exactly as before this
/// change (invariant D13.2's original behaviour, untouched): local passes
/// with no resolver installed; non-local passes only when
/// `allow_non_local_endpoints` is set (`GENARYX_COPILOT_ALLOW_REMOTE`), and
/// that path is untouched too - no hostname logic runs once that opt-in is
/// set, exactly like before.
///
/// A bare hostname is refused outright UNLESS the operator named it in
/// `local_hostnames` (`GENARYX_COPILOT_LOCAL_HOSTNAMES`): the safe default
/// stays "any hostname other than `localhost` is refused, with no DNS call
/// at all", and only a hostname the operator explicitly listed - a
/// Kubernetes Service name or Compose service name they themselves put in
/// the deployment's manifest - gets resolved and checked at all. When it is
/// checked, EVERY address it resolves to right now must be local (checked
/// once here, for a fast, friendly refusal at construction), and the same
/// check is wired into the returned resolver so it runs again on every
/// connection the client makes (see `resolver.rs`'s module doc for why a
/// build-time-only check is not enough).
pub(crate) fn check_residency(
    base_url: &str,
    allow_non_local_endpoints: bool,
    local_hostnames: &[String],
    lookup: Arc<dyn HostnameLookup>,
) -> Result<ResidencyOutcome, ProviderError> {
    match classify_host(base_url) {
        HostResidency::Literal(true) => Ok(ResidencyOutcome {
            local: true,
            dns_resolver: None,
        }),
        HostResidency::Literal(false) => {
            if allow_non_local_endpoints {
                Ok(ResidencyOutcome {
                    local: false,
                    dns_resolver: None,
                })
            } else {
                Err(ProviderError::NonLocalEndpointRefused {
                    url: base_url.to_string(),
                })
            }
        }
        HostResidency::Hostname(name) => {
            if allow_non_local_endpoints {
                // Unchanged BYO-cloud opt-in: any destination, no hostname
                // check, no DNS call - exactly the pre-existing behaviour.
                return Ok(ResidencyOutcome {
                    local: false,
                    dns_resolver: None,
                });
            }
            if !local_hostnames
                .iter()
                .any(|h| h.eq_ignore_ascii_case(&name))
            {
                return Err(ProviderError::NonLocalEndpointRefused {
                    url: base_url.to_string(),
                });
            }
            resolve_all_local(lookup.as_ref(), &name).map_err(|refusal| {
                ProviderError::NonLocalEndpointRefused {
                    url: format!("{base_url} ({refusal})"),
                }
            })?;
            Ok(ResidencyOutcome {
                local: true,
                dns_resolver: Some(ResidencyDnsResolver::new(lookup)),
            })
        }
    }
}

/// The HTTP client a provider sends through. When the residency gate is in
/// force (`allow_non_local_endpoints` unset) the client follows no redirect
/// and reads no proxy from the environment: a `Location` naming a literal
/// public address, or an `HTTP_PROXY` the process inherited, is a second
/// destination the gate never checked, and neither passes through the
/// resolver that re-checks every connection (invariant 14). With the BYO-cloud
/// opt-in the client keeps reqwest's defaults, exactly as before.
pub(crate) fn residency_client(
    outcome: ResidencyOutcome,
    allow_non_local_endpoints: bool,
) -> Result<reqwest::Client, ProviderError> {
    let mut builder = reqwest::Client::builder();
    if !allow_non_local_endpoints {
        builder = builder
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy();
    }
    if let Some(resolver) = outcome.dns_resolver {
        builder = builder.dns_resolver(resolver);
    }
    builder
        .build()
        .map_err(|e| ProviderError::Transport(e.to_string()))
}

/// The production lookup every real provider constructor uses by default:
/// the OS resolver. Test-only constructors (`new_with_lookup`) inject a
/// fixed table instead.
pub(crate) fn system_lookup() -> Arc<dyn HostnameLookup> {
    Arc::new(SystemLookup)
}

/// Turn a `reqwest::Error` from `.send()` into a `ProviderError`, promoting a
/// residency refusal found in its source chain (`resolver::ResidencyRefusal`,
/// raised by `ResidencyDnsResolver` at connect time - invariant 14) to
/// `NonLocalEndpointRefused` instead of the generic `Transport` every other
/// send failure gets. Without this, a hostname refused at the moment reqwest
/// actually dials it would read as an ordinary network error, indistinguishable
/// from a timeout or a DNS hiccup - the opposite of an operator-readable
/// refusal that stays the same shape whether it is caught at construction or
/// at connection time.
pub(crate) fn map_send_error(url: &str, e: reqwest::Error) -> ProviderError {
    let mut source: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(&e);
    while let Some(err) = source {
        if let Some(refusal) = err.downcast_ref::<crate::resolver::ResidencyRefusal>() {
            return ProviderError::NonLocalEndpointRefused {
                url: format!("{url} ({refusal})"),
            };
        }
        source = err.source();
    }
    ProviderError::Transport(e.to_string())
}
