//! Remote: the Phase-4 wave-4 "Distance" surface over Hetzner (read-only
//! inventory) + WireGuard (the primary console<->Cloud channel, decision
//! D11) + SSH (ops: reachability, remote descriptor read, remote log tail)
//! (docs/PHASE4.md "W4 - Distance"), Track A (Tauri/React) of a two-shell
//! parallel build - the SwiftUI half lives in `crates/ffi` + `apps/macos`,
//! built independently against the same grounded contract.
//!
//! Mirrors `crate::identity`/`crate::memory`'s module shape (`env`/`state`/
//! `commands`) with one twist beyond Memory's own "first stateful connector"
//! precedent: this panel holds TWO independent long-lived connections behind
//! ONE managed state - a `WgTunnel` and an `SshClient` - rather than Memory's
//! one `EngramClient`, plus a third, genuinely stateless connector
//! (`HetznerClient`, "no persistent connection" by the connector's own
//! design) that needs no managed state at all.
//!
//! - [`env`] resolves a best-effort DEFAULT `wireguard-go` binary path (the
//!   SAME well-known-then-PATH convention every other panel's `env` module
//!   uses) - never an authority: the operator's saved environment carries its
//!   own `wireguard_go_bin` string that can override it, since v1 has no
//!   auto-discovered "remote environment" at all (it is 100% operator-defined,
//!   see `state`'s module doc).
//! - [`state`] holds the operator-defined environment, the console's own WG
//!   identity, the live tunnel, the live SSH client, and any in-flight remote
//!   tail, EACH behind its own cell so a mutation never has to replace the
//!   whole panel state - see its own doc comment.
//! - [`commands`] are the `#[tauri::command]`s the Remote frontend calls:
//!   `remote_status`, `remote_set_environment`, `remote_hetzner_list`
//!   (stateless), `remote_wg_connect`/`remote_wg_disconnect`, and
//!   `remote_ssh_check_reachable`/`remote_ssh_read_file`/
//!   `remote_ssh_tail_start`/`remote_ssh_tail_stop` (the last streaming
//!   remote log lines to the frontend over two dedicated Tauri events,
//!   `remote:tail-line`/`remote:tail-ended`, mirroring `live.rs`'s own
//!   `app_handle.emit` idiom for its `bus:event` feed).
//!
//! ## v1 scope (docs/PHASE4.md W4)
//!
//! This is the LOCAL, buildable-and-testable half of Distance: the
//! connectors and panel are correct and fail-closed, but `wireguard-go`
//! needs root to create a tun device, so a `remote_wg_connect` run as the
//! operator on a plain dev box is expected to genuinely fail with a
//! privilege error, shown honestly as `Failed`, never a fabricated
//! `Connected` (see `commands`'s module doc). The live tunnel is exercised
//! on the Hetzner campaign, not here; this module adds no sudo/helper flow
//! (a later packaging task).

pub mod commands;
pub mod env;
pub mod state;

pub use state::{RemoteState, bootstrap};
