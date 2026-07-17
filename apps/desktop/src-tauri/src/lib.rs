//! Genaryx desktop shell (Tauri 2, decision D2): thin Rust side, all logic in
//! `genaryx-core` (06 §0.9). `setup` seeds a real `genaryx-core` `Store` from
//! the demo fixtures and starts the live feeder (see `live.rs`);
//! `recent_events` reads that same Store. `events::mock_events` is now only
//! the fail-closed fallback for when startup seeding or a Store read fails.
//!
//! `setup` also manages the Money panel's state (see `money/`): a paired
//! `CloudClient` over TokenFuse Cloud, resolved in the background so a slow
//! or absent Cloud never delays the window opening. The Policy panel's
//! state (see `policy/`, docs/PHASE2.md Wave 2) is managed the same way,
//! independently: a `WardryxClient` over Wardryx, with its own "no policy
//! plane" clean-empty-state contract.
//!
//! Finally, `setup` builds the menu-bar/tray "mini" (see `tray.rs`): a system
//! tray icon whose menu shows a live burn readout plus a "kill last runaway"
//! action, over the same `MoneyState` the Money panel's own IPC commands
//! read and mutate through.

mod events;
mod live;
mod money;
mod policy;
mod tray;

use events::UiEvent;
use live::AppState;
use money::MoneyState;
use policy::PolicyState;
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

            // Money panel: manage the `Bootstrapping` placeholder immediately
            // (so every money_* command has state to read from the instant
            // the app starts), then resolve the real connection in the
            // background - see `money/state.rs`'s module docs for why this is
            // a `spawn`, never a `block_on`, inside `setup`.
            app.manage(MoneyState::pending());
            let money_handle = app.handle().clone();
            let money_events_dir = events_dir.clone();
            tauri::async_runtime::spawn(async move {
                let resolved = money::bootstrap(money_events_dir).await;
                let state = money_handle.state::<MoneyState>();
                *state.inner.lock().await = resolved;
            });

            // Policy panel (docs/PHASE2.md wave 2): same non-blocking
            // manage-then-spawn-resolve shape as the Money panel just above,
            // independent state, independent background task.
            app.manage(PolicyState::pending());
            let policy_handle = app.handle().clone();
            let policy_events_dir = events_dir.clone();
            tauri::async_runtime::spawn(async move {
                let resolved = policy::bootstrap(policy_events_dir).await;
                let state = policy_handle.state::<PolicyState>();
                *state.inner.lock().await = resolved;
            });

            app.manage(AppState { events_dir });

            // Menu-bar mini (docs/PHASE1.md wave 5): reuses the `MoneyState`
            // just managed above via the same `money::commands` functions
            // the window's IPC commands call - no separate Cloud calls, no
            // duplicated connector logic. See `tray.rs`'s module docs.
            tray::setup(app.handle())?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            recent_events,
            money::commands::money_status,
            money::commands::money_overview,
            money::commands::money_runs,
            money::commands::money_incidents,
            money::commands::money_savings,
            money::commands::money_kill_run,
            money::commands::money_set_budget,
            money::commands::money_ack_incident,
            policy::commands::policy_status,
            policy::commands::policy_list_approvals,
            policy::commands::policy_list_policies,
            policy::commands::policy_decide_approval,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Genaryx desktop application");
}
