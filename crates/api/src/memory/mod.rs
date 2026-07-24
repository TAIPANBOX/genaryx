//! Memory: the Phase-4 wave-2 memory-plane surface over Engram's
//! `engram-mcp` MCP stdio server (docs/PHASE4.md "W2 - Memory (Engram MCP) +
//! Drills (Mockryx) panels"), originally built as Track A (Tauri/React) of a
//! two-shell parallel build; the SwiftUI Track B left with the desktop
//! shells (it lived in `crates/ffi` + `apps/macos`, built independently
//! against the same grounded contract).
//!
//! Mirrors `crate::identity`'s module shape (`env`/`state`/`commands`, the
//! same non-blocking bootstrap-in-`setup` wiring) with one structural twist
//! `env.rs`/`state.rs` each call out in their own doc comments:
//! `EngramClient` is the console's first STATEFUL connector - it owns one
//! long-lived `engram-mcp` child process rather than being stateless-per-call
//! like every W0/W1 connector - so this module's managed state OWNS that
//! process (behind a mutex) for the panel's whole lifetime instead of
//! opening/discarding a connection per read the way `quality::state` does.
//!
//! - [`env`] resolves the `engram-mcp` binary AND a real `.engram` store
//!   together (both required; see its own doc comment for why this module
//!   does not split them the way Identity splits its optional Rescan
//!   binary from its required Idryx URL).
//! - [`state`] spawns the ONE `engram-mcp` process at bootstrap and keeps it
//!   in managed state behind a `std::sync::Mutex` for the rest of the app's
//!   life - see its own doc comment for why a std (not tokio) mutex.
//! - [`commands`] are the commands the Memory frontend calls:
//!   `memory_status` plus `memory_stats`/`memory_recall`/`memory_why` (every
//!   one returning a `genaryx_connectors::Engram*` DTO directly - already
//!   `Serialize`, so no UI-facing mirror struct is needed) plus
//!   `memory_forget` (the one admin mutation, irreversible, frontend-gated).

pub mod commands;
pub mod env;
pub mod state;

pub use state::{MemoryState, bootstrap};
