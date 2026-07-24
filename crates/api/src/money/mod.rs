//! Money + Overview: the Phase-1 money-plane surface over TokenFuse Cloud
//! (docs/PHASE1.md wave 3).
//!
//! - [`env`] resolves which Cloud to talk to and with which admin key (a
//!   `taipan up` descriptor under `~/.taipan/environments/`, or a local dev
//!   fallback), with no usable environment being a normal, renderable state
//!   rather than an error.
//! - [`state`] bootstraps a paired `CloudClient` into console-managed state
//!   ([`state::MoneyState`]): `crates/web`'s `Ctx::bootstrap` creates
//!   [`state::MoneyState::pending`] immediately, then `Ctx::resolve` spawns
//!   [`state::bootstrap`] in the background to resolve it (non-blocking on
//!   purpose - see `state.rs`'s module docs).
//! - [`commands`] are the commands the Overview/Money frontend
//!   views call: typed reads, plus three ES256-signed mutations that each
//!   journal a `console_command` onto the same live-wire bus the Bus
//!   Explorer already tails (`crate::bus`), via `genaryx_core::command::record`.

pub mod commands;
pub mod env;
pub mod state;

pub use state::{MoneyState, bootstrap};
