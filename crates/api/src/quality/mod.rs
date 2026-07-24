//! Quality: the Phase-4 wave-1 quality-plane surface over Verdryx
//! (docs/PHASE4.md "W1 - Quality (Verdryx) + Crypto (Qryx) panels"),
//! originally built as Track A (Tauri/Web) of a two-shell parallel build;
//! the SwiftUI Track B left with the desktop shells (it lived in
//! `crates/ffi` + `apps/macos`, built independently against the same
//! grounded contract).
//!
//! Mirrors `crate::identity`'s module shape (`env`/`state`/`commands`, the
//! same non-blocking bootstrap-in-`setup` wiring) - see each submodule's doc
//! comment for how Verdryx's own shape (a SQLite store, no serve process, no
//! bearer key) simplifies or reshapes each piece relative to Idryx's REST
//! snapshot:
//!
//! - [`env`] resolves a `verdryx.db` filesystem path (a descriptor entry,
//!   else a well-known fixed location), with no usable path being a normal,
//!   renderable "no quality plane" state.
//! - [`state`] bootstraps [`state::QualityState`] the same non-blocking way
//!   every other panel's state is managed, confirming the resolved path is a
//!   genuine, openable SQLite store (never holding a live connection in
//!   managed state - see its own doc comment for why).
//! - [`commands`] are the commands the Quality frontend calls:
//!   `quality_status` plus three typed reads, every one returning a
//!   `genaryx_connectors::Verdryx*` DTO directly (already `Serialize`, so no
//!   UI-facing mirror struct is needed - same convention Identity's
//!   `Idryx*` DTOs follow). Drift alerts are NOT a command here at all -
//!   they read the existing live event feed; see `commands`'s module doc.

pub mod commands;
pub mod env;
pub mod state;

pub use state::{QualityState, bootstrap};
