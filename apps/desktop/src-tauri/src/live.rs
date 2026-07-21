//! The Tauri-specific edge of the live bus feeder.
//!
//! Everything generic (the tailer, the demo feeder, the live-vs-demo
//! decision, the WAL/ownership notes) now lives in `genaryx_api::bus::feed`,
//! shared with the web shell (Phase-0 exit gate: "both shells show the same
//! live event stream from the shared core"). What is left here is the one
//! thing that cannot be shared: how a `UiEvent` actually reaches this
//! shell's frontend. [`TauriSink`] is this shell's
//! `genaryx_api::bus::feed::EventSink`, delivering over a Tauri window
//! event; the web shell's own `EventSink` delivers the same event to its SSE
//! subscribers instead.

use genaryx_api::bus::feed::{self, EventSink};
use genaryx_api::events::UiEvent;
use tauri::{AppHandle, Emitter};

/// Tauri event name the frontend `listen()`s for; payload is one [`UiEvent`].
pub const LIVE_EVENT: &str = "bus:event";

/// This shell's [`EventSink`]: forwards one [`UiEvent`] at a time to the app
/// window as a Tauri event. A failed emit (say, the window has already
/// closed) is logged and dropped rather than propagated: `EventSink::emit`
/// returns nothing, so the generic feeder in `genaryx_api::bus::feed` has no
/// `Result` to inspect here, and a bus that stopped reading because one
/// delivery failed would be worse than the one dropped event.
struct TauriSink(AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, event: UiEvent) {
        if let Err(e) = self.0.emit(LIVE_EVENT, event) {
            eprintln!("genaryx: failed to emit live event: {e}");
        }
    }
}

/// Open the console's bus and start the background thread that feeds it, via
/// the shared `genaryx_api::bus::feed::bootstrap` (see its module doc for the
/// live-vs-demo decision and everything else that happens inside). This
/// wrapper only supplies the desktop's own [`TauriSink`] so that shared
/// feeder can reach this shell's window.
pub fn bootstrap(app_handle: AppHandle) -> genaryx_core::Result<feed::BusBootstrap> {
    feed::bootstrap(TauriSink(app_handle))
}
