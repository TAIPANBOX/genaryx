//! Identity: the Phase-3 wave-2 identity-plane surface over Idryx
//! (docs/PHASE3.md, "W2 - Identity panel"), Track A (Tauri/Web) of a
//! two-shell parallel build - the SwiftUI half lives in
//! `crates/ffi/src/idryx/` + `apps/macos`, built independently against the
//! same grounded contract.
//!
//! Mirrors `crate::policy`'s module shape (`env`/`state`/`commands`, same
//! non-blocking bootstrap-in-`setup` wiring), reusing every convention that
//! module established - but simpler and narrower in the ways idryx itself
//! is (07 §4.4 / `crates/connectors/src/idryx.rs`'s module doc):
//!
//! - **Unauthenticated and read-only.** Idryx has no bearer, no signer, no
//!   mutation route at all. [`env`] resolves only a URL (plus, best-effort,
//!   the taipan events section Rescan needs); [`commands`] exposes zero
//!   mutations, so there is no `command::record` journal entry anywhere in
//!   this module - Identity changes nothing in any other plane, ever.
//! - **A load-once snapshot, not a live feed.** `idryx serve` computes its
//!   graph and detectors exactly once at startup and never reloads (no
//!   file-watch, no SIGHUP, no TTL - grounded in the idryx Go source, see
//!   `crates/connectors/src/idryx.rs`'s module doc). Every read this
//!   module's commands return is labeled "as of load" by the frontend,
//!   never implied live; [`commands::identity_rescan`] is the one way to
//!   pick up newer data, by shelling out to `idryx detect` on demand rather
//!   than waiting on a reload that will never happen.
//!
//! [`env`] resolves which Idryx to talk to (the SAME `taipan up` descriptor
//! `policy::env`/`money::env` read, a different service entry, no key at
//! all), with no usable environment being a normal, renderable "no identity
//! plane" state rather than an error. [`state`] bootstraps a
//! [`state::IdentityState`] the same non-blocking way `lib.rs`'s `setup`
//! hook manages every other panel's state, and additionally resolves
//! (best-effort) the `idryx` binary + events files
//! [`commands::identity_rescan`] needs. [`commands`] are the
//! commands the Identity frontend calls: `identity_status` plus
//! three typed reads (`identity_list_identities`/`identity_list_alerts`/
//! `identity_list_remediations`, every one returning a
//! `genaryx_connectors::Idryx*` DTO directly - it already derives
//! `Serialize`, unlike Wardryx's connector types, so no UI-facing mirror
//! struct is needed) plus `identity_rescan`.

pub mod commands;
pub mod env;
pub mod state;

pub use state::{IdentityState, bootstrap};
