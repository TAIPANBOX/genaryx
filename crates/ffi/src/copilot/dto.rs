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
/// registry has no tool to call even once a provider is configured), token
/// usage, and (C2, docs/PHASE6-C2.md) every action Felyx PROPOSES but never
/// performs. Mirrors `genaryx_copilot::Answer`, with
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
    /// C2: recommendations only - rendered as approve/reject cards. Empty
    /// whenever the loop ran no propose tool (every C0/C1 answer, and any C2
    /// answer that only reads). See [`CopilotProposalDto`]'s own doc comment
    /// for why "Approve" never lives on this crate's side of the boundary.
    pub proposals: Vec<CopilotProposalDto>,
}

impl From<genaryx_copilot::Answer> for CopilotAnswerDto {
    fn from(a: genaryx_copilot::Answer) -> Self {
        Self {
            text: a.text,
            tool_trace: a.tool_trace.into_iter().map(CopilotToolDto::from).collect(),
            prompt_tokens: a.usage.prompt_tokens,
            completion_tokens: a.usage.completion_tokens,
            proposals: a
                .proposals
                .into_iter()
                .map(CopilotProposalDto::from)
                .collect(),
        }
    }
}

/// One [`genaryx_copilot::ProposedAction`], flattened for UniFFI (C2,
/// docs/PHASE6-C2.md): a structured recommendation with its evidence, never
/// an executed mutation - this crate's `CopilotHandle` holds no signer (see
/// `genaryx_copilot`'s own crate doc: "Act does not exist"), so there is no
/// `approve()` method anywhere on this boundary. The shell renders this as a
/// card and, on "Approve", routes into the EXISTING human-signed ceremony
/// (`CloudHandle`'s break-glass kill/budget, `WardryxHandle`'s
/// `decide_approval`, `IdryxHandle`'s `rescan`) - never a new signed path
/// here.
///
/// `kind` is flattened to `genaryx_copilot::ActionKind`'s own lowercase wire
/// string (`"kill"` / `"budget"` / `"grant_deny"` / `"rescan"`) rather than a
/// UniFFI `Enum` mirror: the shell only ever switches on the string to pick
/// which existing signed path to call, exactly like it already does for
/// [`super::super::wardryx::ApprovalVerdict`]'s sibling string fields
/// elsewhere on this boundary (e.g. `ApprovalRecord.decision`). `params` is
/// serialized to a JSON string (`params_json`) rather than carried as
/// `serde_json::Value` directly: UniFFI has no arbitrary-JSON type, so every
/// other `Value`-shaped field on this crate's boundary is a `String` the
/// Swift side decodes itself (mirrors how `crates/ffi/src/cloud/dto.rs`
/// flattens connector-side JSON into typed fields, just without a fixed
/// schema here - a proposal's `params` shape varies by `kind`).
#[derive(Debug, Clone, uniffi::Record)]
pub struct CopilotProposalDto {
    pub kind: String,
    /// The subject: a run id (Kill/Budget), an approval id (GrantDeny), or an
    /// agent id / `"all"` (Rescan).
    pub target: String,
    /// `serde_json::to_string(&params)` - `{"usd_cap":5.0}` for Budget,
    /// `{"verdict":"grant"}` for GrantDeny, `"{}"` for Kill/Rescan (never
    /// fails in practice: a `serde_json::Value` is always encodable, but the
    /// fallback keeps this conversion infallible rather than panicking on an
    /// unreachable edge).
    pub params_json: String,
    pub rationale: String,
    pub confidence: f64,
    pub evidence_refs: Vec<String>,
    /// Non-empty only when Wardryx is configured and governs this action's
    /// target - see `genaryx_copilot::ProposedAction::policy_context`'s own
    /// doc comment.
    pub policy_context: Vec<String>,
}

impl From<genaryx_copilot::ProposedAction> for CopilotProposalDto {
    fn from(p: genaryx_copilot::ProposedAction) -> Self {
        let kind = match p.kind {
            genaryx_copilot::ActionKind::Kill => "kill",
            genaryx_copilot::ActionKind::Budget => "budget",
            genaryx_copilot::ActionKind::GrantDeny => "grant_deny",
            genaryx_copilot::ActionKind::Rescan => "rescan",
        }
        .to_string();
        Self {
            kind,
            target: p.target,
            params_json: serde_json::to_string(&p.params).unwrap_or_else(|_| "{}".to_string()),
            rationale: p.rationale,
            confidence: f64::from(p.confidence),
            evidence_refs: p.evidence_refs,
            policy_context: p.policy_context,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `ActionKind` maps to the exact lowercase wire string the shell
    /// switches on (docs/PHASE6-C2.md), `params` round-trips through
    /// `params_json` as real JSON (not just `Debug`-stringified), and every
    /// other field carries straight through unchanged - proven once per
    /// kind so a future `ActionKind` variant that forgets to extend the
    /// match in `CopilotProposalDto::from` fails to compile rather than
    /// silently falling through.
    #[test]
    fn copilot_proposal_dto_maps_every_kind_to_its_wire_string_and_serializes_params() {
        let cases = [
            (genaryx_copilot::ActionKind::Kill, "kill"),
            (genaryx_copilot::ActionKind::Budget, "budget"),
            (genaryx_copilot::ActionKind::GrantDeny, "grant_deny"),
            (genaryx_copilot::ActionKind::Rescan, "rescan"),
        ];
        for (kind, wire) in cases {
            let action = genaryx_copilot::ProposedAction {
                kind,
                target: "reconciliation-batch".to_string(),
                params: serde_json::json!({ "usd_cap": 5.0 }),
                rationale: "burn tripled after a policy hold".to_string(),
                confidence: 0.82,
                evidence_refs: vec!["incident:182".to_string()],
                policy_context: vec!["agent://meridian/*".to_string()],
            };
            let dto = CopilotProposalDto::from(action);
            assert_eq!(dto.kind, wire);
            assert_eq!(dto.target, "reconciliation-batch");
            assert_eq!(dto.rationale, "burn tripled after a policy hold");
            assert!((dto.confidence - 0.82).abs() < 1e-6);
            assert_eq!(dto.evidence_refs, vec!["incident:182".to_string()]);
            assert_eq!(dto.policy_context, vec!["agent://meridian/*".to_string()]);

            let decoded: serde_json::Value = serde_json::from_str(&dto.params_json)
                .unwrap_or_else(|e| panic!("params_json must be real JSON for {wire}: {e}"));
            assert_eq!(decoded["usd_cap"], 5.0);
        }
    }

    /// An empty `params` (`{}`, Kill/Rescan's actual shape) round-trips to
    /// the literal `"{}"` string, never `"null"` or a panic.
    #[test]
    fn copilot_proposal_dto_serializes_empty_params_as_an_empty_json_object() {
        let action = genaryx_copilot::ProposedAction::new(
            genaryx_copilot::ActionKind::Kill,
            "r-1",
            serde_json::json!({}),
            "runaway",
            0.9,
            vec![],
        );
        let dto = CopilotProposalDto::from(action);
        assert_eq!(dto.params_json, "{}");
    }

    /// `CopilotAnswerDto::from` carries `Answer.proposals` through via
    /// `CopilotProposalDto::from` (not dropped, not left as a default-empty
    /// `Vec` alongside a genuinely non-empty source) - the one new field
    /// this DTO gained in C2.
    #[test]
    fn copilot_answer_dto_carries_proposals_through() {
        let answer = genaryx_copilot::Answer {
            text: "I'd recommend killing this run.".to_string(),
            tool_trace: vec![],
            usage: genaryx_copilot::Usage {
                prompt_tokens: 10,
                completion_tokens: 5,
            },
            proposals: vec![genaryx_copilot::ProposedAction::new(
                genaryx_copilot::ActionKind::Kill,
                "r-1",
                serde_json::json!({}),
                "runaway",
                0.9,
                vec![],
            )],
        };
        let dto = CopilotAnswerDto::from(answer);
        assert_eq!(dto.proposals.len(), 1);
        assert_eq!(dto.proposals[0].kind, "kill");
        assert_eq!(dto.proposals[0].target, "r-1");
    }
}
