//! Genaryx desktop shell (Tauri 2, decision D2): thin Rust side, all logic in
//! `genaryx-core` (06 §0.9). `setup` seeds a real `genaryx-core` `Store` from
//! the demo fixtures and starts the live feeder (see `live.rs`);
//! `recent_events` reads that same Store. `genaryx_api::events::mock_events` is now only
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
//! first STATEFUL connector this app manages: `genaryx_api::memory::bootstrap` spawns and
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
//! The Remote panel's state (see `genaryx_api::remote`, docs/PHASE4.md W4,
//! "Distance") is managed the same non-blocking way once more, but resolves
//! NOTHING auto-discovered beyond a best-effort `wireguard-go` default path:
//! the WG peer, the SSH target, and even that binary path are 100%
//! operator-defined (see `genaryx_api::remote::state`'s module doc). It is
//! the app's SECOND stateful-connector panel after Memory - it holds both a
//! `WgTunnel` and an `SshClient` long-lived, each behind its own cell, for
//! the app's whole life once the operator connects/pins them. Its SSH tail
//! is the one piece of the shared command layer this shell must still
//! supply the delivery for: `commands::remote::remote_ssh_tail_start` builds
//! a `TauriTailSink` from the `AppHandle` and hands it to
//! `genaryx_api::remote::commands::remote_ssh_tail_start` as the generic
//! `TailSink` its reader thread streams through (see that function's own
//! module doc, and `commands.rs`'s `remote` module).
//!
//! The Pocket panel's commands (see `pocket/`, docs/PHASE5.md W2) need NO
//! managed state at all and so are never `app.manage`d in `setup` - every
//! command resolves the Cloud admin key (reusing `money::env::discover`
//! directly) and the relay admin URL (`pocket::env::relay_admin_url`) fresh,
//! per call. See `pocket::commands`'s module doc for the full mint-code /
//! arm-window / render-QR / show-device flow.
//!
//! The Copilot panel's state (see `copilot/`, docs/PHASE6.md C0 /
//! docs/PHASE6-C1.md C1) is managed the same non-blocking way once more:
//! `bootstrap` always builds a `CopilotConfig::default()` (still
//! disabled-by-default - this shell has no `[copilot]` config source yet)
//! `CopilotService`, but since C1 it is built over a REAL `Clients` that
//! reuses every other panel's own `env::discover()` (see
//! `copilot::state`'s module doc) rather than C0's empty `Clients::default()`,
//! so Felyx's tools are already wired to whatever planes this box resolves
//! the day a provider is configured.

mod commands;
mod live;
mod tray;

use genaryx_api::bus::AppState;
use genaryx_api::copilot::CopilotState;
use genaryx_api::crypto::CryptoState;
use genaryx_api::drills::DrillsState;
use genaryx_api::events::UiEvent;
use genaryx_api::evidence::EvidenceState;
use genaryx_api::identity::IdentityState;
use genaryx_api::memory::MemoryState;
use genaryx_api::money::MoneyState;
use genaryx_api::policy::PolicyState;
use genaryx_api::quality::QualityState;
use genaryx_api::remote::RemoteState;
use tauri::Manager;

/// Recent events for the Bus Explorer. The reader itself lives with the bus
/// it reads (`genaryx_api::bus`), so the web shell serves the same rows.
#[tauri::command]
fn recent_events(limit: usize, state: tauri::State<'_, AppState>) -> Vec<UiEvent> {
    genaryx_api::bus::recent_events(limit, &state)
}

/// Where the Bus Explorer's events actually come from, so the UI can say so.
///
/// The frontend must never have to guess this. A console tailing a real
/// environment and a console showing generated fixtures look identical on
/// screen, and the difference is the whole credibility of the product: a
/// screenshot of invented traffic presented as a customer's own is the exact
/// failure the "no fabricated data" rule exists to prevent. Cheap to call and
/// read-only, so a panel can re-read it whenever it likes.
#[tauri::command]
fn bus_status(state: tauri::State<'_, AppState>) -> genaryx_api::bus::BusMode {
    genaryx_api::bus::bus_status(&state)
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
            let (events_dir, bus_mode) = match live::bootstrap(app.handle().clone()) {
                Ok(bus) => (Some(bus.events_dir), bus.mode),
                Err(e) => {
                    eprintln!("genaryx: bus startup failed, Bus Explorer will use mock data: {e}");
                    (
                        None,
                        genaryx_api::bus::BusMode::Unavailable {
                            reason: e.to_string(),
                        },
                    )
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
                let resolved = genaryx_api::money::bootstrap(money_events_dir).await;
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
                let resolved = genaryx_api::policy::bootstrap(policy_events_dir).await;
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
                let resolved = genaryx_api::identity::bootstrap().await;
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
                let resolved = genaryx_api::quality::bootstrap().await;
                let state = quality_handle.state::<QualityState>();
                *state.inner.lock().await = resolved;
            });

            // Crypto panel (docs/PHASE4.md W1): same shape again, resolving
            // the on-demand `qryx` CLI binary rather than pairing with a
            // live service - see `crypto::state`'s module doc.
            app.manage(CryptoState::pending());
            let crypto_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let resolved = genaryx_api::crypto::bootstrap().await;
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
                let resolved = genaryx_api::memory::bootstrap().await;
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
                let resolved = genaryx_api::drills::bootstrap().await;
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
                let resolved = genaryx_api::evidence::bootstrap().await;
                let state = evidence_handle.state::<EvidenceState>();
                *state.inner.lock().await = resolved;
            });

            // Remote panel (docs/PHASE4.md W4, "Distance"): same
            // non-blocking shape once more, resolving only a best-effort
            // `wireguard-go` default path - no environment, tunnel, or SSH
            // client exists until the operator explicitly defines one (see
            // `genaryx_api::remote::state`'s module doc).
            app.manage(RemoteState::pending());
            let remote_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let resolved = genaryx_api::remote::bootstrap().await;
                let state = remote_handle.state::<RemoteState>();
                *state.inner.lock().await = resolved;
            });

            // Copilot panel (docs/PHASE6.md C0): same non-blocking
            // manage-then-spawn-resolve shape once more, simplest of all -
            // `bootstrap` discovers nothing (no `taipan up` descriptor, no
            // binary path), it just builds the C0 default (disabled)
            // `CopilotService` - see `copilot::state`'s module doc.
            app.manage(CopilotState::pending());
            let copilot_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let resolved = genaryx_api::copilot::bootstrap().await;
                let state = copilot_handle.state::<CopilotState>();
                *state.inner.lock().await = resolved;
            });

            app.manage(AppState {
                events_dir,
                mode: bus_mode,
            });

            // Menu-bar mini (docs/PHASE1.md wave 5): reuses the `MoneyState`
            // just managed above via the same `money::commands` functions
            // the window's IPC commands call - no separate Cloud calls, no
            // duplicated connector logic. See `tray.rs`'s module docs.
            tray::setup(app.handle())?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            recent_events,
            bus_status,
            commands::money::money_status,
            commands::money::money_overview,
            commands::money::money_runs,
            commands::money::money_incidents,
            commands::money::money_savings,
            commands::money::money_kill_run,
            commands::money::money_set_budget,
            commands::money::money_ack_incident,
            commands::policy::policy_status,
            commands::policy::policy_list_approvals,
            commands::policy::policy_list_policies,
            commands::policy::policy_decide_approval,
            commands::identity::identity_status,
            commands::identity::identity_list_identities,
            commands::identity::identity_list_alerts,
            commands::identity::identity_list_remediations,
            commands::identity::identity_rescan,
            // Onboard (docs/ONBOARD.md, D15/B2): stateless like Pocket below -
            // every call re-resolves the identity map + passports dir fresh,
            // so there is nothing for `setup` to `app.manage` here.
            commands::onboard::onboard_status,
            commands::onboard::onboard_generate,
            commands::onboard::onboard_write_passport,
            commands::quality::quality_status,
            commands::quality::quality_list_run_summaries,
            commands::quality::quality_run_scores,
            commands::quality::quality_list_baselines,
            commands::crypto::crypto_status,
            commands::crypto::crypto_scan_ncsc,
            commands::crypto::crypto_scan_cbom,
            commands::crypto::crypto_scan_evidence,
            commands::crypto::crypto_verify_evidence,
            commands::memory::memory_status,
            commands::memory::memory_stats,
            commands::memory::memory_recall,
            commands::memory::memory_why,
            commands::memory::memory_forget,
            commands::drills::drills_status,
            commands::drills::drills_run,
            commands::evidence::evidence_status,
            commands::evidence::evidence_build,
            commands::remote::remote_status,
            commands::remote::remote_set_environment,
            commands::remote::remote_hetzner_list,
            commands::remote::remote_cloud_list,
            commands::remote::remote_wg_connect,
            commands::remote::remote_wg_disconnect,
            commands::remote::remote_ssh_check_reachable,
            commands::remote::remote_ssh_read_file,
            commands::remote::remote_ssh_tail_start,
            commands::remote::remote_ssh_tail_stop,
            commands::pocket::pocket_status,
            commands::pocket::pocket_connect,
            commands::pocket::pocket_disconnect,
            commands::graph::agent_graph,
            commands::graph::agent_slice,
            commands::graph::agent_events,
            commands::replay::run_events,
            commands::copilot::copilot_status,
            commands::copilot::copilot_ask,
            commands::copilot::copilot_explain,
            commands::copilot::copilot_log_proposal_approved,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Genaryx desktop application");
}
