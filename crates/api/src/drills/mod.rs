//! Drills: the Phase-4 wave-2 drills-plane surface over Mockryx
//! (docs/PHASE4.md "W2 - Memory (Engram MCP) + Drills (Mockryx) panels"),
//! Track A (Tauri/React) of a two-shell parallel build - the SwiftUI half
//! lives in `crates/ffi` + `apps/macos`, built independently against the
//! same grounded contract.
//!
//! Mirrors `crate::crypto`'s module shape (`env`/`state`/`commands`, the
//! same non-blocking bootstrap-in-`setup` wiring, no `Unreachable` state):
//! mockryx is a pure on-demand CLI, like qryx - no serve process, no live
//! feed, invoked fresh for every run.
//!
//! - [`env`] resolves the `mockryx` binary AND the TokenFuse gateway URL
//!   together (both required; see its own doc comment for why), plus an
//!   optional bearer and a best-effort default scenario directory.
//! - [`state`] bootstraps [`state::DrillsState`] the same non-blocking way
//!   every other panel's state is managed.
//! - [`commands`] are the commands the Drills frontend calls:
//!   `drills_status` plus [`commands::drills_run`] (returns a
//!   `genaryx_connectors::MockryxReport` directly - already `Serialize`, so
//!   no UI-facing mirror struct is needed), never auto-triggered.

pub mod commands;
pub mod env;
pub mod state;

pub use state::{DrillsState, bootstrap};
