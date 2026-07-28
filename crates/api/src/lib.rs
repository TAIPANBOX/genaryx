//! The console's command layer, with no shell of its own.
//!
//! One function per command the UI can call, plus the per-plane state each
//! needs and the DTOs the frontend decodes. Nothing here knows whether it was
//! reached over any transport, which is the point: `genaryx-web` wraps these
//! functions rather than reimplementing them, so a command means exactly one
//! thing no matter how the operator reached the console. (The since-removed
//! desktop shells wrapped the same functions, which is why the split exists.)
//!
//! What each plane owns is unchanged from when this code lived inside the
//! desktop shell: `env` resolves what to talk to, `state` holds the resolved
//! client, `commands` are the callable surface. The only difference is the
//! signature: a command takes `&MoneyState` directly, so it is callable (and
//! testable) without a window, an app handle, or a running event loop.
//!
//! Deliberately NOT here: anything that needs a shell to exist (the removed
//! desktop shell's tray was the canonical example). The live bus feeder
//! and the Remote panel's SSH tail are both a partial exception of the same
//! shape: the tailer, the demo feeder, and the live-vs-demo decision
//! ([`bus::feed`]), and the tail's reader thread
//! ([`remote::commands::remote_ssh_tail_start`]), both live here, each
//! generic over a small sink trait each shell implements for its own surface
//! ([`bus::feed::EventSink`], [`remote::commands::TailSink`] - today the web
//! shell's SSE broadcast; the removed desktop shell's was a Tauri window
//! event, which is why this seam exists) - only that one final delivery call
//! stays in the shell. The DATA every plane reads ([`bus::AppState`], [`bus::BusMode`],
//! [`events::UiEvent`]) lives here too, because every shell reads it.

pub mod bus;
pub mod console_actor;
pub mod events;
pub mod graph;
pub mod replay;

pub mod admission;
pub mod copilot;
pub mod credentials;
pub mod crypto;
pub mod drills;
pub mod evidence;
pub mod identity;
pub mod memory;
pub mod money;
pub mod onboard;
pub mod policy;
pub mod quality;
pub mod remote;
pub mod routines;

pub use bus::{AppState, BusMode};
pub use events::UiEvent;
