//! Genaryx desktop shell (Tauri 2, decision D2): thin Rust side, all logic in
//! `genaryx-core` (06 §0.9). Today this only serves mock Bus Explorer data;
//! see `events.rs` for the exact spot the real `IngestService` bus replaces it.

mod events;

use events::UiEvent;

/// Recent events for the Bus Explorer, newest first, capped at `limit`.
///
/// Mock data today (see `events::mock_events` and the `FOLLOW-UP WIRING
/// POINT` doc comment on `impl From<StoredEvent> for UiEvent` in
/// `events.rs`). Never panics: there is no failure mode yet, and once this
/// reads from a real `Store` it stays fail-closed by returning what it can
/// rather than trapping the frontend on an `Err`.
#[tauri::command]
fn recent_events(limit: usize) -> Vec<UiEvent> {
    events::mock_events(limit)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![recent_events])
        .run(tauri::generate_context!())
        .expect("error while running the Genaryx desktop application");
}
