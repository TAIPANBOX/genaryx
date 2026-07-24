//! Pocket: the Phase-5 wave-2 "Connect TokenFuse Pocket" panel
//! (docs/PHASE5.md W2, itrat-console/13 D12.2a) - mints a pairing code for
//! the phone and one for the watch at the Cloud, arms both of the relay's
//! pairing windows, renders the QR (both codes) the phone scans (a later
//! wave, W3, hands the watch its own code over WatchConnectivity), and
//! shows the paired device(s) + Disconnect. Originally built as Track A
//! (Tauri/React) of the two-shell parallel build; the SwiftUI Track B left
//! with the desktop shells (it lived in `crates/ffi::pocket` + `apps/macos`).
//!
//! Unlike every other panel in this app, Pocket holds NO console-managed
//! state at all: every command resolves its own Cloud admin key (reusing
//! `crate::money::env::discover` directly - minting a pairing code needs
//! exactly the same admin bearer Money's own device pairing does) and its
//! own relay admin URL ([`env::relay_admin_url`]), fresh, per call. There is
//! no persistent connection worth holding onto between an operator's
//! Connect/status-poll/Disconnect actions (mirrors
//! `remote::commands::remote_hetzner_list`'s identical "stateless by
//! design" shape) - see [`commands`]'s module doc for the full flow.

pub mod commands;
pub mod env;
