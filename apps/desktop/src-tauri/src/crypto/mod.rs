//! Crypto: the Phase-4 wave-1 crypto-plane surface over Qryx
//! (docs/PHASE4.md "W1 - Quality (Verdryx) + Crypto (Qryx) panels"), Track A
//! (Tauri/Web) of a two-shell parallel build - the SwiftUI half lives in
//! `crates/ffi` + `apps/macos`, built independently against the same
//! grounded contract.
//!
//! Mirrors `crate::identity`'s module shape (`env`/`state`/`commands`) but
//! simpler still: qryx is a pure on-demand CLI with no serve process and no
//! taipan descriptor entry at all - see each submodule's doc comment.
//!
//! - [`env`] resolves the well-known `~/.taipan/bin/qryx` binary plus a
//!   default on-demand scan target, with no resolved binary being a normal,
//!   renderable "no crypto plane" state.
//! - [`state`] bootstraps [`state::CryptoState`] the same non-blocking way
//!   every other panel's state is managed - simpler than the rest, since
//!   there is no liveness probe to await (see its own doc comment).
//! - [`commands`] are the `#[tauri::command]`s the Crypto frontend calls:
//!   `crypto_status` plus the scan/evidence/verify actions, every one
//!   returning a `genaryx_connectors::{NcscReport,EvidenceReport,VerifyOutcome}`
//!   DTO directly (already `Serialize`, so no UI-facing mirror struct is
//!   needed).

pub mod commands;
pub mod env;
pub mod state;

pub use state::{CryptoState, bootstrap};
