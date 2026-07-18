//! Copilot: the Phase-6 (C0) AI-copilot surface over `genaryx-copilot`
//! (Felyx), docs/PHASE6.md, itrat-console/13.
//!
//! Mirrors `crate::identity`'s module shape (`state`/`commands`, same
//! non-blocking bootstrap-in-`setup` wiring), simpler in the one way this
//! panel itself is: there is no environment to discover. Idryx/Wardryx/
//! TokenFuse Cloud all live behind a `taipan up` descriptor this app scans
//! for; the copilot's C0 config is a fixed, local `CopilotConfig::default()`
//! (see `state.rs`'s module doc for why), so there is no `env.rs` here at
//! all.
//!
//! C0 ships the read-only conversational surface only: [`state`] bootstraps
//! a disabled-by-default `CopilotService` ("no LLM configured on this box"
//! is a normal, renderable state, never an error); [`commands`] exposes
//! [`commands::copilot_status`] (the residency banner's data) and
//! [`commands::copilot_ask`] (one question/answer round trip through
//! Felyx). There is no mutation command here and never will be: the
//! `genaryx-copilot` crate holds no signer at all (its own `src/lib.rs` doc
//! comment: "Act does not exist"), so nothing in this module can change any
//! other plane's state either - Felyx can only read and, from a later cut,
//! propose.

pub mod commands;
pub mod state;

pub use state::{CopilotState, bootstrap};
