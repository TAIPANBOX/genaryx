//! Evidence Center: the Phase-4 wave-3 evidence-assembly surface
//! (docs/PHASE4.md W3), originally built as Track A (Tauri/React) of a
//! two-shell parallel build; the SwiftUI Track B left with the desktop
//! shells (it lived in `crates/ffi` + `apps/macos`, built independently
//! against the same frozen `genaryx_connectors::build_evidence_pack`
//! contract).
//!
//! Unlike every prior panel, this one does NOT introduce a new plane
//! connection of its own: it ASSEMBLES a pack from sources every other panel
//! (or the Money plane specifically) already resolves - see `commands.rs`'s
//! module doc for the full "why reuse Money's CloudClient" rationale, and
//! `env.rs`'s for how the three local-tool sources (qryx/idryx/tokenfuse) are
//! resolved.
//!
//! - [`env`] resolves the qryx/idryx/tokenfuse local-tool sources, each
//!   independently - best-effort, honest, never panics.
//! - [`state`] bootstraps [`state::EvidenceState`] the same non-blocking way
//!   every other panel's state is managed.
//! - [`commands`] are the commands the Evidence frontend calls:
//!   `evidence_status` plus [`commands::evidence_build`] - the one on-demand
//!   "Build evidence pack" action, taking BOTH `EvidenceState` and
//!   `MoneyState` (see its own doc comment).

pub mod commands;
pub mod env;
pub mod state;

pub use state::{EvidenceState, bootstrap};
