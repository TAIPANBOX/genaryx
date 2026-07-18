//! Wire DTOs and error taxonomy for [`super::CopilotHandle`], flattening
//! `genaryx_copilot`'s plain Rust types into UniFFI `Record`/`Error` shapes -
//! the same "this crate defines its own same-shaped counterparts" convention
//! `crates/ffi/src/wardryx/dto.rs`'s own doc comment establishes. Imported
//! with no `Conn`-style alias prefix here (unlike `wardryx/dto.rs`/
//! `idryx/dto.rs`): `genaryx_copilot`'s own names (`Answer`, `ToolInvocation`,
//! `CopilotError`) do not collide with anything this module defines -
//! [`CopilotAnswerDto`], [`CopilotToolDto`], and [`CopilotFfiError`] are all
//! already distinctly named, so every `genaryx_copilot::*` reference below is
//! qualified inline instead.

// ============================================================================
// DTOs
// ============================================================================

/// The residency banner's data (docs/PHASE6.md C0-W2: "the residency banner
/// ('local: ... via Ollama' vs 'remote: ..., BYO key' vs 'no provider
/// configured')"). Exactly one shape is populated at a time: enabled carries
/// `provider`/`model`/`endpoint`/`local` (mirrors
/// `genaryx_copilot::ProviderDescriptor` field for field); disabled carries
/// `disabled_reason` instead. Flattened into one optional-fields Record
/// rather than a UniFFI enum with associated data - the simplest shape for
/// the Swift view to switch on with a single `if`/`else` over `enabled`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct CopilotStatusDto {
    pub enabled: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    /// `true` when inference never leaves this box - the "nothing leaves
    /// this box" claim (`genaryx_copilot::ProviderDescriptor::local`'s own
    /// doc comment). `None` when disabled (there is no provider to be local
    /// or not).
    pub local: Option<bool>,
    /// Set only when `enabled == false`: the exact reason
    /// `CopilotService::disabled_reason` gives (e.g. "No copilot provider is
    /// configured...").
    pub disabled_reason: Option<String>,
}

impl From<&genaryx_copilot::CopilotService> for CopilotStatusDto {
    fn from(service: &genaryx_copilot::CopilotService) -> Self {
        match service.descriptor() {
            Some(d) => Self {
                enabled: true,
                provider: Some(d.provider),
                model: Some(d.model),
                endpoint: Some(d.endpoint),
                local: Some(d.local),
                disabled_reason: None,
            },
            None => Self {
                enabled: false,
                provider: None,
                model: None,
                endpoint: None,
                local: None,
                disabled_reason: service.disabled_reason().map(str::to_string),
            },
        }
    }
}

/// One tool call the agent loop executed - the evidence surface next to the
/// model's own text (docs/PHASE6.md: "the `tool_trace`... so the shell can
/// render evidence next to the model text", the anti-hallucination promise:
/// numbers come from tools, shown verbatim). Field-for-field mirror of
/// `genaryx_copilot::ToolInvocation`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct CopilotToolDto {
    pub name: String,
    pub ok: bool,
    /// A short, truncated preview of the tool's JSON result.
    pub result_preview: String,
}

impl From<genaryx_copilot::ToolInvocation> for CopilotToolDto {
    fn from(t: genaryx_copilot::ToolInvocation) -> Self {
        Self {
            name: t.name,
            ok: t.ok,
            result_preview: t.result_preview,
        }
    }
}

/// One finished answer: the model's text, every tool it ran (empty in C0 -
/// [`super::CopilotHandle::create`] wires `Clients::default()`, so the
/// registry has no tool to call even once a provider is configured), and
/// token usage. Mirrors `genaryx_copilot::Answer`, with
/// `usage.prompt_tokens`/`usage.completion_tokens` flattened directly onto
/// this Record rather than nested behind a one-off `CopilotUsageDto` - there
/// is exactly one consumer of `Usage` on this boundary, so nesting would only
/// be one more type the Swift shell has to unwrap for no benefit.
#[derive(Debug, Clone, uniffi::Record)]
pub struct CopilotAnswerDto {
    pub text: String,
    pub tool_trace: Vec<CopilotToolDto>,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

impl From<genaryx_copilot::Answer> for CopilotAnswerDto {
    fn from(a: genaryx_copilot::Answer) -> Self {
        Self {
            text: a.text,
            tool_trace: a.tool_trace.into_iter().map(CopilotToolDto::from).collect(),
            prompt_tokens: a.usage.prompt_tokens,
            completion_tokens: a.usage.completion_tokens,
        }
    }
}

// ============================================================================
// error taxonomy
// ============================================================================

/// Every failure mode a [`super::CopilotHandle`] call can surface, fail-closed
/// throughout (06 §0.5: no panics/unwraps cross the FFI boundary). Collapsed
/// from two upstream source enums - `genaryx_copilot::ConfigError`
/// ([`super::CopilotHandle::create`]'s own fallible step) and
/// `genaryx_copilot::CopilotError` ([`super::CopilotHandle::ask`]'s) - into
/// one taxonomy here, since the Swift shell renders both failure kinds the
/// same way (a plain message), with one exception: [`Self::NoProvider`] gets
/// its own variant rather than folding into [`Self::Failed`] so
/// `CopilotModel.swift` can special-case it into a quiet "no provider
/// configured" message instead of a red error banner - it is `ask`'s NORMAL
/// outcome against the C0 default (`provider = "none"`), never a bug.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CopilotFfiError {
    /// `ask()` against a disabled service. Mirrors
    /// `genaryx_copilot::CopilotError::NoProvider` exactly.
    #[error("no copilot provider is configured")]
    NoProvider,
    /// A configured provider failed to build (e.g. a non-local endpoint
    /// without opt-in, a missing model/key) - collapsed from
    /// `genaryx_copilot::ConfigError`. Not reachable from
    /// [`super::CopilotHandle::create`]'s own C0 call (`CopilotConfig::default()`,
    /// `provider = "none"`, never fails to build), but a real, honest outcome
    /// once a later wave lets an operator configure a real provider through
    /// this handle.
    #[error("copilot config: {reason}")]
    Config { reason: String },
    /// Any other `ask()` failure: the provider's own transport/HTTP/decode
    /// error, or the agent loop hitting its iteration bound - collapsed from
    /// `genaryx_copilot::CopilotError::{Provider, IterationLimit}`.
    #[error("copilot request failed: {reason}")]
    Failed { reason: String },
}

impl From<genaryx_copilot::CopilotError> for CopilotFfiError {
    fn from(e: genaryx_copilot::CopilotError) -> Self {
        match e {
            genaryx_copilot::CopilotError::NoProvider => CopilotFfiError::NoProvider,
            other => CopilotFfiError::Failed {
                reason: other.to_string(),
            },
        }
    }
}

impl From<genaryx_copilot::ConfigError> for CopilotFfiError {
    fn from(e: genaryx_copilot::ConfigError) -> Self {
        CopilotFfiError::Config {
            reason: e.to_string(),
        }
    }
}
