//! Credentials: I15 "key lifecycle health" - a read-only plane over the
//! TokenFuse gateway's key-lifecycle report (`GET /v1/keys`,
//! `genaryx_connectors::GatewayClient`, the FIRST direct gateway REST read
//! this console makes). Mirrors `crate::identity`'s module shape
//! (`env`/`state`/`commands`, same non-blocking bootstrap-in-`setup` wiring)
//! and, like Identity, is READ-ONLY: no `console_actor`, no
//! `genaryx_core::command::record` journal entry, no signer - this plane
//! changes nothing in any other plane, and the gateway's `/v1/keys` route
//! itself has no write counterpart for this console to call even if it
//! wanted to (`identity/mod.rs`'s own wording for the identical rule).
//!
//! [`env`] resolves which gateway to talk to: the SAME `services.gateway.url`
//! `crate::drills::env` reads off a `taipan up` descriptor - explicitly NOT
//! `services.cloud` (TokenFuse Cloud's separate admin API, `crate::money`'s
//! target). No key, no auth; a descriptor with no gateway service (or none
//! found at all) resolves to `None`, a normal, renderable "no credentials
//! plane" state, never an error. [`state`] bootstraps a
//! [`state::CredentialsState`] the same non-blocking way every other plane's
//! `setup`/`Ctx::resolve` does. [`commands`] exposes `credentials_status`
//! (the state-tagged connection DTO) and `credentials_keys` (the gateway's
//! report, straight through - `GatewayKeysReport` already derives
//! `Serialize`, no UI-facing mirror struct needed, the exact idryx precedent
//! `identity::commands`'s module doc names).

pub mod commands;
pub mod env;
pub mod state;

pub use state::{CredentialsState, bootstrap};
