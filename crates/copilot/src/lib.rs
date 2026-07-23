//! Felyx, the Genaryx AI copilot (D13), `crates/copilot`.
//!
//! Build contract: `docs/PHASE6.md`. Architecture: `itrat-console/13`, D13.
//!
//! # The model, in three sentences
//!
//! - **Read** tools execute autonomously (they are the same reads any viewer
//!   already has).
//! - **Propose** tools return a [`action::ProposedAction`] object; rendering it
//!   as a card and routing an acceptance into the EXISTING human-signed
//!   ceremony is the host's job. Nothing has happened yet.
//! - **Act does not exist.** This crate has NO dependency on `genaryx-signing`
//!   and holds no signer; it is structurally unable to produce an `X-Fuse`
//!   signature (D13.3/D13.4). "An AI cannot press the buttons" is a fact about
//!   the dependency graph, not a prompt.
//!
//! C0 (this cut) ships the read path only: the provider abstraction, the
//! loopback residency gate, the typed read-tool registry over the existing
//! connectors, the hand-rolled agent loop, and natural-language answers whose
//! numbers come from tools, never from the model doing arithmetic in prose.
//! [`action::ProposedAction`] is defined now but only emitted from C2.

pub mod action;
pub mod agent;
pub mod config;
pub mod provider;
pub mod residency;
pub mod service;
pub mod tools;

pub use action::{ActionKind, CopilotAnnotation, ProposedAction};
pub use agent::{Answer, CopilotError, Felyx, ToolInvocation};
pub use config::{ConfigError, CopilotConfig, ProviderKind, SecretRef};
pub use provider::{
    ChatRequest, ChatTurn, LlmProvider, Message, ProviderDescriptor, ProviderError, Role, ToolCall,
    ToolSpec, Usage, build_provider,
};
pub use service::CopilotService;
pub use tools::{Clients, TokenfuseTraces, ToolError, ToolRegistry};
