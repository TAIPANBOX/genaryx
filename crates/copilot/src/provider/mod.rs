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

use async_trait::async_trait;
use serde_json::Value;

use crate::config::{ConfigError, CopilotConfig, ProviderKind};

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
            let provider = OpenAiCompat::new(
                config.provider,
                base_url,
                model,
                api_key,
                config.allow_non_local_endpoints,
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
            let provider =
                AnthropicMessages::new(base_url, model, api_key, config.allow_non_local_endpoints)
                    .map_err(ConfigError::Provider)?;
            Ok(Some(Box::new(provider)))
        }
    }
}
