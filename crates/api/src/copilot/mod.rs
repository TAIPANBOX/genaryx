//! Copilot: the Phase-6 (C0) AI-copilot surface over `genaryx-copilot`
//! (Felyx), docs/PHASE6.md, itrat-console/13.
//!
//! Mirrors `crate::identity`'s module shape (`state`/`commands`, same
//! non-blocking bootstrap-in-`setup` wiring), simpler in one way this panel
//! itself still is: the copilot's OWN config is a fixed, local
//! `CopilotConfig::default()` (see `state.rs`'s module doc for why), so
//! there is no `env.rs` here for a `[copilot]` provider source. There IS
//! environment discovery since C1 (docs/PHASE6-C1.md), just not a new kind:
//! `state::resolve_clients` reuses Idryx/Wardryx/TokenFuse Cloud/Qryx/
//! Verdryx/Engram's OWN `env::discover()` (`crate::identity`/`policy`/
//! `money`/`crypto`/`quality`/`memory`) to wire Felyx's tools, rather than
//! this module introducing a seventh, duplicated resolution path.
//!
//! C0 shipped the read-only conversational surface: [`state`] bootstraps a
//! disabled-by-default `CopilotService` ("no LLM configured on this box" is
//! a normal, renderable state, never an error); [`commands`] exposes
//! [`commands::copilot_status`] (the residency banner's data) and
//! [`commands::copilot_ask`] (one question/answer round trip through
//! Felyx). C1 adds [`commands::copilot_explain`] (the "Explain with Felyx"
//! cross-plane root-cause flow) and wires real connector clients into
//! `state::bootstrap` (see `state.rs`'s module doc). There is no mutation
//! command here and never will be: the `genaryx-copilot` crate holds no
//! signer at all (its own `src/lib.rs` doc comment: "Act does not exist"),
//! so nothing in this module can change any other plane's state either -
//! Felyx can only read and, from a later cut, propose.

pub mod commands;
pub mod state;

pub use state::{CopilotState, bootstrap};
