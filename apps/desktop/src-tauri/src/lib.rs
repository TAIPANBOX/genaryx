//! Genaryx desktop shell (Tauri 2, decision D2): thin Rust side, all logic in
//! `genaryx-core` (06 §0.9). `setup` seeds a real `genaryx-core` `Store` from
//! the demo fixtures and starts the live feeder (see `live.rs`);
//! `recent_events` reads that same Store. `events::mock_events` is now only
//! the fail-closed fallback for when startup seeding or a Store read fails.

mod events;
mod live;

use events::UiEvent;
use live::AppState;
use tauri::Manager;

/// Recent events for the Bus Explorer, newest first, capped at `limit`.
///
/// Reads the real `genaryx-core` `Store` seeded at startup (see
/// `live::bootstrap`) through its own short-lived reader connection (WAL
/// mode lets this coexist with the live feeder's writer thread). Never
/// panics and never surfaces an `Err` to the frontend: a missing store
/// (startup seeding failed) or a failed query both fall back to
/// `events::mock_events`, so the Bus Explorer always renders something
/// rather than trapping on a broken bus.
#[tauri::command]
fn recent_events(limit: usize, state: tauri::State<'_, AppState>) -> Vec<UiEvent> {
    if let Some(dir) = &state.events_dir {
        let db_path = dir.join("console.sqlite");
        match genaryx_core::store::Store::open(&db_path) {
            Ok(store) => match store.recent_events(limit) {
                Ok(rows) => return rows.into_iter().map(UiEvent::from).collect(),
                Err(e) => eprintln!(
                    "genaryx: recent_events query failed, falling back to mock data: {e}"
                ),
            },
            Err(e) => eprintln!(
                "genaryx: could not open store for recent_events, falling back to mock data: {e}"
            ),
        }
    }
    events::mock_events(limit)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let events_dir = match live::bootstrap(app.handle().clone()) {
                Ok(dir) => Some(dir),
                Err(e) => {
                    eprintln!(
                        "genaryx: startup store seeding failed, Bus Explorer will use mock data: {e}"
                    );
                    None
                }
            };
            app.manage(AppState { events_dir });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![recent_events])
        .run(tauri::generate_context!())
        .expect("error while running the Genaryx desktop application");
}
