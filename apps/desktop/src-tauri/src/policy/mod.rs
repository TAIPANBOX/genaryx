//! Policy + Approvals: the Phase-2 wave-2 policy-plane surface over Wardryx
//! (docs/PHASE2.md, "Wave 2 data contract + UX"), Track A (Tauri/Web) of a
//! two-shell parallel build - the SwiftUI half lives in `crates/ffi/src/wardryx/`
//! + `apps/macos`, built independently against the same data contract.
//!
//! Mirrors `crate::money`'s module shape exactly (`env`/`state`/`commands`,
//! same non-blocking bootstrap-in-`setup` wiring), reusing every convention
//! that module established (the `command::record` journal, the
//! fail-closed "always journal the attempt" rule) rather than inventing new
//! ones - but everything underneath is simpler, because Wardryx (07 §4.3)
//! is bearer-only with no device/pairing story at all:
//!
//! - [`env`] resolves which Wardryx to talk to and with which admin bearer
//!   (the SAME `taipan up` descriptor `money::env` reads, a different
//!   service entry), with no usable environment being a normal, renderable
//!   "no policy plane" state rather than an error.
//! - [`state`] bootstraps a [`state::PolicyState`]: `lib.rs`'s `setup` hook
//!   manages [`state::PolicyState::pending`] immediately, then spawns
//!   [`state::bootstrap`] in the background to resolve it.
//! - [`commands`] are the `#[tauri::command]`s the Policy frontend calls:
//!   two typed reads (`policy_list_approvals`/`policy_list_policies`) plus
//!   one mutation (`policy_decide_approval`) that journals a
//!   `console_command` onto the same live-wire bus the Bus Explorer already
//!   tails (`crate::live`), via `genaryx_core::command::record`.
//!
//! Deliberately NOT coupled to `crate::money`: every module here keeps its
//! own small mirrors of the handful of conventions the two share (see
//! `env`/`state`'s own doc comments) rather than importing from it, the
//! same "parallel, not shared" precedent `crates/connectors/src/wardryx.rs`
//! already set relative to `cloud_rest.rs`.

pub mod commands;
pub mod env;
pub mod state;

pub use state::{PolicyState, bootstrap};
