//! The console's command layer, with no shell of its own.
//!
//! One function per command the UI can call, plus the per-plane state each
//! needs and the DTOs the frontend decodes. Nothing here knows whether it was
//! reached over Tauri IPC or over HTTP, which is the point: the desktop shell
//! and `genaryx-web` both wrap these functions rather than reimplementing
//! them, so a command cannot mean two different things depending on how the
//! operator opened the console.
//!
//! What each plane owns is unchanged from when this code lived inside
//! `src-tauri`: `env` resolves what to talk to, `state` holds the resolved
//! client, `commands` are the callable surface. The only difference is the
//! signature: a command takes `&MoneyState` instead of a Tauri `State`
//! wrapper, so it is directly callable (and directly testable) without a
//! window, an app handle, or a running event loop.
//!
//! Deliberately NOT here: anything that needs a shell to exist. The tray
//! stays in its shell (only the desktop has one at all). The live bus feeder
//! and the Remote panel's SSH tail are both a partial exception of the same
//! shape: the tailer, the demo feeder, and the live-vs-demo decision
//! ([`bus::feed`]), and the tail's reader thread
//! ([`remote::commands::remote_ssh_tail_start`]), both live here, each
//! generic over a small sink trait each shell implements for its own surface
//! ([`bus::feed::EventSink`], [`remote::commands::TailSink`] - a Tauri window
//! event, an SSE broadcast) - only that one final delivery call stays in the
//! shell. The DATA every plane reads ([`bus::AppState`], [`bus::BusMode`],
//! [`events::UiEvent`]) lives here too, because both shells read it.

pub mod bus;
pub mod events;
pub mod graph;
pub mod replay;

pub mod copilot;
pub mod crypto;
pub mod drills;
pub mod evidence;
pub mod identity;
pub mod memory;
pub mod money;
pub mod onboard;
pub mod pocket;
pub mod policy;
pub mod quality;
pub mod remote;

pub use bus::{AppState, BusMode};
pub use events::UiEvent;
