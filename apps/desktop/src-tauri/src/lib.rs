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
//! plane" clean-empty-state contract. The Identity panel's state (see
//! `identity/`, docs/PHASE3.md Wave 2) is managed the same non-blocking way
//! again, independently: an `IdryxClient` over Idryx - unauthenticated and
//! read-only, so unlike Money/Policy it journals nothing and needs no
//! events-dir handle at all.
//!
//! `setup` builds the menu-bar/tray "mini" (see `tray.rs`): a system tray
//! icon whose menu shows a live burn readout plus a "kill last runaway"
//! action, over the same `MoneyState` the Money panel's own IPC commands
//! read and mutate through.
//!
//! Finally, `.plugin(tauri_plugin_notification::init())` wires the OS
//! notification plugin (docs/PHASE2.md Wave 3, "Actionable notifications").
//! No Rust command of this crate's own reads or writes a notification: the
//! frontend (`src/lib/notifications.ts`) watches the same `bus:event` feed
//! `live.rs` already emits and calls straight into the plugin's JS API.
//!
//! `graph::agent_graph`/`agent_slice`/`agent_events` (docs/PHASE3.md Wave 3:
//! the delegation graph + Agent 360) need no managed state of their own at
//! all - they read the same `AppState.events_dir` `recent_events` already
//! reads, straight off the shared `genaryx-core` `Store`, so there is
//! nothing to bootstrap in `setup` for them.
//!
//! `replay::run_events` (docs/PHASE3.md Wave 4: Run Replay) is the same
//! shape again - a fourth reader of `AppState.events_dir`'s Store, no managed
//! state of its own. The playback clock itself (play/pause/scrub/speed) is
//! pure frontend state over that one fetched list; see `replay.rs`'s module
//! doc.
//!
//! The Quality panel's state (see `quality/`, docs/PHASE4.md Wave 1) is
//! managed the same non-blocking way again, independently: a resolved
//! `verdryx.db` path (Verdryx has no serve process to pair with - see
//! `quality::state`'s module doc). The Crypto panel's state (see `crypto/`,
//! docs/PHASE4.md Wave 1) is managed the same way once more: a resolved
//! `qryx` binary plus a default scan target, on-demand rather than a live
//! connection. Quality's drift alerts need no managed state of their own at
//! all - they read the SAME `AppState.events_dir` `recent_events` already
//! reads, filtered client-side to `source == "verdryx"`, exactly like the
//! Policy panel's Decision Stream filters to `source == "wardryx"`.
//!
//! The Memory panel's state (see `memory/`, docs/PHASE4.md Wave 2) is the
//! first STATEFUL connector this app manages: `memory::bootstrap` spawns and
//! keeps alive ONE long-lived `engram-mcp` process for the whole app
//! lifetime (see `memory::state`'s module doc for why re-spawning per call
//! is never acceptable). The Drills panel's state (see `drills/`,
//! docs/PHASE4.md Wave 2) is managed the same non-blocking way once more,
//! mirroring Crypto's on-demand-CLI shape exactly: a resolved `mockryx`
//! binary plus the TokenFuse gateway to rehearse against, never a live
//! connection. Memory's live timeline (`engram.*` bus events) and Drills'
//! findings both need no extra managed state either - Memory's timeline
//! reads the SAME `AppState.events_dir` `recent_events` filtered to
//! `source == "engram"`, and Drills' findings are simply part of the
//! `MockryxReport` `drills_run` already returns.
//!
//! The Evidence Center's state (see `evidence/`, docs/PHASE4.md W3) is
//! managed the same non-blocking way once more, resolving three independent
//! local-tool sources (qryx/idryx/tokenfuse - see `evidence::state`'s module
//! doc for why there is no single Ready/NoEnvironment gate here). It
//! introduces NO new Cloud connection of its own: its build command reuses
//! the Money panel's already-paired `CloudClient` straight out of
//! `MoneyState` (see `evidence::commands`'s module doc), so it needs no
//! `events_dir` handle either - journaling reads `MoneyClient.bus` at build
//! time, not a handle stored on `EvidenceState` itself.
//!
//! The Remote panel's state (see `remote/`, docs/PHASE4.md W4, "Distance")
//! is managed the same non-blocking way once more, but resolves NOTHING
//! auto-discovered beyond a best-effort `wireguard-go` default path: the WG
//! peer, the SSH target, and even that binary path are 100%
//! operator-defined (see `remote::state`'s module doc). It is the app's
//! SECOND stateful-connector panel after Memory - it holds both a `WgTunnel`
//! and an `SshClient` long-lived, each behind its own cell, for the app's
//! whole life once the operator connects/pins them.
//!
//! The Pocket panel's commands (see `pocket/`, docs/PHASE5.md W2) need NO
//! managed state at all and so are never `app.manage`d in `setup` - every
//! command resolves the Cloud admin key (reusing `money::env::discover`
//! directly) and the relay admin URL (`pocket::env::relay_admin_url`) fresh,
//! per call. See `pocket::commands`'s module doc for the full mint-code /
//! arm-window / render-QR / show-device flow.

mod crypto;
mod drills;
mod events;
mod evidence;
mod graph;
mod identity;
mod live;
mod memory;
mod money;
mod pocket;
mod policy;
mod quality;
mod remote;
mod replay;
mod tray;

use crypto::CryptoState;
use drills::DrillsState;
use events::UiEvent;
use evidence::EvidenceState;
use identity::IdentityState;
use live::AppState;
use memory::MemoryState;
use money::MoneyState;
use policy::PolicyState;
use quality::QualityState;
use remote::RemoteState;
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
                Err(e) => {
                    eprintln!("genaryx: recent_events query failed, falling back to mock data: {e}")
                }
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
        // Phase-2 wave 3 (docs/PHASE2.md, "Actionable notifications"): native
        // OS notifications for `approval_requested` bus events, driven
        // entirely from the frontend (`src/lib/notifications.ts`) over this
        // plugin's JS API - see `Cargo.toml`'s dependency comment for what is
        // (and is not) wired on desktop.
        .plugin(tauri_plugin_notification::init())
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

            // Identity panel (docs/PHASE3.md wave 2): same non-blocking
            // manage-then-spawn-resolve shape as Money/Policy above,
            // independent state, independent background task. Unlike
            // Money/Policy, `bootstrap` takes no `events_dir`: Identity
            // journals nothing onto the console's own live-wire bus (see
            // `identity`'s module doc), so it has no use for that directory
            // at all - only the SEPARATE taipan-descriptor `events` section
            // `identity::env::discover` reads matters here, and that comes
            // off the same descriptor as the idryx URL, not from this app's
            // own startup seeding.
            app.manage(IdentityState::pending());
            let identity_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let resolved = identity::bootstrap().await;
                let state = identity_handle.state::<IdentityState>();
                *state.inner.lock().await = resolved;
            });

            // Quality panel (docs/PHASE4.md W1): same non-blocking
            // manage-then-spawn-resolve shape as Money/Policy/Identity
            // above, independent state, independent background task. Reads
            // Verdryx's `verdryx.db` directly (no serve process to pair
            // with) - see `quality::state`'s module doc.
            app.manage(QualityState::pending());
            let quality_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let resolved = quality::bootstrap().await;
                let state = quality_handle.state::<QualityState>();
                *state.inner.lock().await = resolved;
            });

            // Crypto panel (docs/PHASE4.md W1): same shape again, resolving
            // the on-demand `qryx` CLI binary rather than pairing with a
            // live service - see `crypto::state`'s module doc.
            app.manage(CryptoState::pending());
            let crypto_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let resolved = crypto::bootstrap().await;
                let state = crypto_handle.state::<CryptoState>();
                *state.inner.lock().await = resolved;
            });

            // Memory panel (docs/PHASE4.md W2): same non-blocking
            // manage-then-spawn-resolve shape once more, but `bootstrap`
            // itself does real work here (spawns + handshakes the one
            // long-lived `engram-mcp` process this panel keeps for the rest
            // of the app's life) - see `memory::state`'s module doc for why
            // that still belongs in a background task rather than blocking
            // `setup`.
            app.manage(MemoryState::pending());
            let memory_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let resolved = memory::bootstrap().await;
                let state = memory_handle.state::<MemoryState>();
                *state.inner.lock().await = resolved;
            });

            // Drills panel (docs/PHASE4.md W2): same shape again, resolving
            // the on-demand `mockryx` CLI binary plus the TokenFuse gateway
            // to rehearse against - never a live connection, mirroring
            // Crypto's `qryx` shape exactly (see `drills::state`'s module
            // doc).
            app.manage(DrillsState::pending());
            let drills_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let resolved = drills::bootstrap().await;
                let state = drills_handle.state::<DrillsState>();
                *state.inner.lock().await = resolved;
            });

            // Evidence Center (docs/PHASE4.md W3): same shape again,
            // resolving the three independent local-tool sources
            // (qryx/idryx/tokenfuse) - see `evidence::state`'s module doc.
            // No `events_dir`/Cloud pairing of its own: `evidence_build`
            // reads the Money panel's `MoneyState` directly at call time
            // (see `evidence::commands`'s module doc), so nothing else is
            // threaded through here.
            app.manage(EvidenceState::pending());
            let evidence_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let resolved = evidence::bootstrap().await;
                let state = evidence_handle.state::<EvidenceState>();
                *state.inner.lock().await = resolved;
            });

            // Remote panel (docs/PHASE4.md W4, "Distance"): same
            // non-blocking shape once more, resolving only a best-effort
            // `wireguard-go` default path - no environment, tunnel, or SSH
            // client exists until the operator explicitly defines one (see
            // `remote::state`'s module doc).
            app.manage(RemoteState::pending());
            let remote_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let resolved = remote::bootstrap().await;
                let state = remote_handle.state::<RemoteState>();
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
            identity::commands::identity_status,
            identity::commands::identity_list_identities,
            identity::commands::identity_list_alerts,
            identity::commands::identity_list_remediations,
            identity::commands::identity_rescan,
            quality::commands::quality_status,
            quality::commands::quality_list_run_summaries,
            quality::commands::quality_run_scores,
            quality::commands::quality_list_baselines,
            crypto::commands::crypto_status,
            crypto::commands::crypto_scan_ncsc,
            crypto::commands::crypto_scan_cbom,
            crypto::commands::crypto_scan_evidence,
            crypto::commands::crypto_verify_evidence,
            memory::commands::memory_status,
            memory::commands::memory_stats,
            memory::commands::memory_recall,
            memory::commands::memory_why,
            memory::commands::memory_forget,
            drills::commands::drills_status,
            drills::commands::drills_run,
            evidence::commands::evidence_status,
            evidence::commands::evidence_build,
            remote::commands::remote_status,
            remote::commands::remote_set_environment,
            remote::commands::remote_hetzner_list,
            remote::commands::remote_wg_connect,
            remote::commands::remote_wg_disconnect,
            remote::commands::remote_ssh_check_reachable,
            remote::commands::remote_ssh_read_file,
            remote::commands::remote_ssh_tail_start,
            remote::commands::remote_ssh_tail_stop,
            pocket::commands::pocket_status,
            pocket::commands::pocket_connect,
            pocket::commands::pocket_disconnect,
            graph::agent_graph,
            graph::agent_slice,
            graph::agent_events,
            replay::run_events,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Genaryx desktop application");
}
