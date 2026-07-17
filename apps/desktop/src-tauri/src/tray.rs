//! The menu-bar "mini" (Phase-1 wave 5, docs/PHASE1.md; 08 §2, 09 Ф1
//! upstream): a system tray icon whose menu shows a live burn readout plus a
//! "kill last runaway" action, sharing the exact same `MoneyState` /
//! `money::commands` plumbing the main window's Overview + Money panels
//! already read and mutate through - no separate Cloud calls, no duplicated
//! connector logic (mirrors `apps/macos/Sources/Genaryx/GenaryxApp.swift`'s
//! `MenuBarExtra`, which extends the same way over `CloudModel`).
//!
//! Fail-closed (06 §0.5): any Cloud/read error - including the ordinary
//! `Bootstrapping` window every fresh launch starts in - degrades the tray to
//! a "no data" / disabled-kill state, never a crash, never a silent no-op.
//! The kill action always goes through [`money::commands::money_kill_run`],
//! so a menu-bar kill still journals a `console_command` exactly like a
//! Money-panel kill does (`money::commands::finish_mutation`).
//!
//! Confirm-before-mutate (matches `ConfirmButton.tsx` / `ConfirmButton.swift`'s
//! "never a single click straight to a signed mutation" rule): a native tray
//! menu item has no room for an inline Confirm/Cancel pair, so this expresses
//! the same idle -> confirming -> pending shape as an arm/confirm text toggle
//! instead - a first click arms (relabels the item and starts a short
//! timeout), a second click on the *same* target within that window fires
//! the kill; anything else (a different target, a timeout, a stale click)
//! just re-arms rather than ever killing a run the operator did not just see
//! named on the menu.
//!
//! Break-glass reason (Phase-2 wave 3B): `money_kill_run` now requires a
//! non-empty justification (`crates/core`'s `require_break_glass_reason`,
//! and the shell's own front-line copy of it). A native menu item has no
//! text field either, so the tray's kill always journals the fixed
//! [`TRAY_KILL_REASON`] string rather than an operator-typed one - honest
//! about being a menu-bar action, and distinct enough from the Money panel's
//! own free-text reasons that the two are never confused in the audit
//! trail.

use crate::money::{self, MoneyState};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry, include_image};

/// Matches the Overview/Money panels' own refresh cadence (`OverviewView`'s
/// `refreshInterval` / `MoneyView`'s `refreshInterval`), so the mini never
/// looks "more live" or "more stale" than the panel it mirrors.
const REFRESH_INTERVAL: Duration = Duration::from_secs(20);

/// How long "Kill last runaway" stays armed after a first click before
/// silently reverting to its normal label - see this module's doc.
const ARM_TIMEOUT: Duration = Duration::from_secs(5);

const KILL_ITEM_ID: &str = "kill_last_runaway";
const SHOW_ITEM_ID: &str = "show_window";
const MAIN_WINDOW_LABEL: &str = "main";

/// The break-glass justification `money::commands::money_kill_run` now
/// requires (Phase-2 wave 3B, `require_break_glass_reason`). A native tray
/// menu item has no room for a free-text field (this module's own doc
/// explains why the arm/confirm two-click dance stands in for
/// `ConfirmButton`'s inline confirm step); this fixed, honest string is the
/// tray's whole justification, distinct enough from a real operator-typed
/// reason that anyone reading the journal can tell the two apart.
const TRAY_KILL_REASON: &str = "menu-bar tray: kill last runaway (armed + confirmed via tray menu, no free-text reason field available)";

/// The highest-`spent_usd` non-killed run as of the last successful refresh -
/// the same run the Money panel's own runs table would rank highest by
/// spend, recomputed from a fresh `money_runs` read every tick.
#[derive(Clone)]
struct RunawayTarget {
    run_id: String,
    spent_usd: f64,
}

/// State shared between the periodic refresh task and the menu-click
/// handler. Plain `Arc`+`Mutex`, not Tauri-managed state: nothing outside
/// this module ever needs to read it, so there is no reason to route it
/// through `AppHandle::state()`.
struct TrayRuntime {
    burn_item: MenuItem<Wry>,
    kill_item: MenuItem<Wry>,
    /// What "Kill last runaway" currently targets, or `None` when there is
    /// nothing killable (no active runs, or the last refresh failed).
    target: Mutex<Option<RunawayTarget>>,
    /// `Some(run_id)` while a confirmation is armed for that run. A refresh
    /// tick that lands while this is set skips re-rendering `kill_item`
    /// entirely, so the armed label survives until the operator confirms or
    /// the timeout task below clears it.
    armed: Mutex<Option<String>>,
    /// Previous `(sampled_at, total_spent_usd)`, to derive a `$/hr` burn
    /// rate across two ticks once one exists - "nice to have"; the readout
    /// still renders without it (e.g. on the very first tick).
    last_sample: Mutex<Option<(Instant, f64)>>,
}

/// Build the tray icon and start its refresh loop. Called once from
/// `lib.rs`'s `setup` hook. Ordering relative to `money::bootstrap`'s
/// background resolution does not matter - a not-yet-ready `MoneyState` just
/// reads back as `MoneyError::Bootstrapping` through the same
/// `money::commands` functions the window's own IPC commands call, which
/// this module renders the same honest way the frontend does.
pub fn setup(app: &AppHandle<Wry>) -> tauri::Result<()> {
    let burn_item = MenuItem::with_id(
        app,
        "burn_readout",
        "Genaryx: connecting…",
        false,
        None::<&str>,
    )?;
    let kill_item = MenuItem::with_id(
        app,
        KILL_ITEM_ID,
        "Kill last runaway (no data)",
        false,
        None::<&str>,
    )?;
    let show_item = MenuItem::with_id(app, SHOW_ITEM_ID, "Show Genaryx", true, None::<&str>)?;
    let separator_a = PredefinedMenuItem::separator(app)?;
    let separator_b = PredefinedMenuItem::separator(app)?;
    let quit_item = PredefinedMenuItem::quit(app, Some("Quit Genaryx"))?;

    let menu = Menu::with_items(
        app,
        &[
            &burn_item,
            &kill_item,
            &separator_a,
            &show_item,
            &separator_b,
            &quit_item,
        ],
    )?;

    let runtime = Arc::new(TrayRuntime {
        burn_item,
        kill_item,
        target: Mutex::new(None),
        armed: Mutex::new(None),
        last_sample: Mutex::new(None),
    });

    let menu_runtime = runtime.clone();
    let _tray = TrayIconBuilder::new()
        .icon(include_image!("icons/32x32.png"))
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip("Genaryx")
        .on_menu_event(move |app, event| handle_menu_event(app, event, &menu_runtime))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    let loop_app = app.clone();
    let loop_runtime = runtime;
    tauri::async_runtime::spawn(async move {
        loop {
            refresh(&loop_app, &loop_runtime).await;
            tokio::time::sleep(REFRESH_INTERVAL).await;
        }
    });

    Ok(())
}

/// Show and focus the main window - the tray's left-click action, and also
/// "Show Genaryx" in the menu. Best-effort: a failure here is a degraded
/// convenience action, never something worth crashing or blocking the tray
/// over.
fn show_main_window(app: &AppHandle<Wry>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        eprintln!("genaryx: tray could not find the \"{MAIN_WINDOW_LABEL}\" window to show");
        return;
    };
    if let Err(e) = window.show() {
        eprintln!("genaryx: tray failed to show the main window: {e}");
    }
    if let Err(e) = window.set_focus() {
        eprintln!("genaryx: tray failed to focus the main window: {e}");
    }
}

fn handle_menu_event(app: &AppHandle<Wry>, event: MenuEvent, runtime: &Arc<TrayRuntime>) {
    if event.id() == SHOW_ITEM_ID {
        show_main_window(app);
        return;
    }
    if event.id() == KILL_ITEM_ID {
        let app = app.clone();
        let runtime = runtime.clone();
        tauri::async_runtime::spawn(async move {
            on_kill_clicked(&app, &runtime).await;
        });
    }
    // "Quit Genaryx" is a `PredefinedMenuItem`: the OS/muda terminates the
    // app natively and never reaches this handler.
}

/// One click on "Kill last runaway": arm on the first click, kill (through
/// the exact same signed+journaled path the Money panel's own kill button
/// uses) on a second click that still targets the same run - see this
/// module's doc for why a mismatched or stale click re-arms instead of
/// firing.
async fn on_kill_clicked(app: &AppHandle<Wry>, runtime: &Arc<TrayRuntime>) {
    let Some(target) = runtime.target.lock().unwrap().clone() else {
        return; // fail-closed: nothing displayed to kill (still loading, or no active runs).
    };

    let already_armed_for_target = {
        let mut armed = runtime.armed.lock().unwrap();
        let matched = armed.as_deref() == Some(target.run_id.as_str());
        *armed = if matched {
            None
        } else {
            Some(target.run_id.clone())
        };
        matched
    };

    if !already_armed_for_target {
        let label = format!(
            "Confirm: kill {} ({})? Click again",
            short_run_id(&target.run_id),
            format_usd(target.spent_usd)
        );
        log_menu_result("arm kill_last_runaway", runtime.kill_item.set_text(label));

        let app = app.clone();
        let runtime = runtime.clone();
        let run_id = target.run_id;
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(ARM_TIMEOUT).await;
            let still_armed = {
                let mut armed = runtime.armed.lock().unwrap();
                let matched = armed.as_deref() == Some(run_id.as_str());
                if matched {
                    *armed = None;
                }
                matched
            };
            if still_armed {
                refresh(&app, &runtime).await; // repaint back to the normal label from live data.
            }
        });
        return;
    }

    let result = money::commands::money_kill_run(
        target.run_id.clone(),
        TRAY_KILL_REASON.to_string(),
        app.state::<MoneyState>(),
    )
    .await;
    match result {
        Ok(outcome) => eprintln!(
            "genaryx: tray killed run {} via console_command (bus_recorded={})",
            target.run_id, outcome.bus_recorded
        ),
        Err(e) => eprintln!("genaryx: tray kill_run failed for {}: {e:?}", target.run_id),
    }
    refresh(app, runtime).await;
}

/// Refresh the burn readout and (unless a confirmation is currently armed -
/// see `TrayRuntime::armed`'s doc) the "Kill last runaway" target/label, from
/// the exact same `money::commands` reads the Overview/Money panels use.
/// Never panics: every `Err` from either read degrades the relevant item to
/// an honest "no data" state instead.
async fn refresh(app: &AppHandle<Wry>, runtime: &Arc<TrayRuntime>) {
    let overview = money::commands::money_overview(app.state::<MoneyState>()).await;
    let burn_text = match &overview {
        Ok(o) => format_burn_line(runtime, o),
        Err(money::commands::MoneyError::Bootstrapping) => "Genaryx: connecting…".to_string(),
        Err(money::commands::MoneyError::NoEnvironment) => "Genaryx: no environment".to_string(),
        Err(money::commands::MoneyError::PairingFailed { .. }) => {
            "Genaryx: pairing failed".to_string()
        }
        Err(_) => "Genaryx: no data".to_string(),
    };
    log_menu_result("update burn_readout", runtime.burn_item.set_text(burn_text));

    if runtime.armed.lock().unwrap().is_some() {
        return; // an operator confirmation is in flight - do not relabel out from under it.
    }

    let runs = money::commands::money_runs(app.state::<MoneyState>()).await;
    match runs {
        Ok(list) => {
            let top = list
                .iter()
                .filter(|r| !r.killed)
                .reduce(|a, b| if b.spent_usd > a.spent_usd { b } else { a });
            match top {
                Some(run) => {
                    *runtime.target.lock().unwrap() = Some(RunawayTarget {
                        run_id: run.run_id.clone(),
                        spent_usd: run.spent_usd,
                    });
                    let label = format!(
                        "Kill last runaway - {} ({})",
                        short_run_id(&run.run_id),
                        format_usd(run.spent_usd)
                    );
                    log_menu_result(
                        "update kill_last_runaway text",
                        runtime.kill_item.set_text(label),
                    );
                    log_menu_result(
                        "enable kill_last_runaway",
                        runtime.kill_item.set_enabled(true),
                    );
                }
                None => {
                    *runtime.target.lock().unwrap() = None;
                    log_menu_result(
                        "update kill_last_runaway text",
                        runtime
                            .kill_item
                            .set_text("Kill last runaway (no active runs)"),
                    );
                    log_menu_result(
                        "disable kill_last_runaway",
                        runtime.kill_item.set_enabled(false),
                    );
                }
            }
        }
        Err(_) => {
            *runtime.target.lock().unwrap() = None;
            log_menu_result(
                "update kill_last_runaway text",
                runtime.kill_item.set_text("Kill last runaway (no data)"),
            );
            log_menu_result(
                "disable kill_last_runaway",
                runtime.kill_item.set_enabled(false),
            );
        }
    }
}

/// "Spent $X.XX (+$Y.YY/hr) - N active runs", degrading to just the two
/// totals when no prior sample exists yet to derive a rate from (the very
/// first tick after launch) - mirrors `OverviewView`'s own tiles, condensed
/// to one line.
fn format_burn_line(runtime: &TrayRuntime, overview: &money::commands::OverviewDto) -> String {
    let now = Instant::now();
    let mut last = runtime.last_sample.lock().unwrap();
    let rate_per_hour = last.and_then(|(prev_at, prev_spent)| {
        let elapsed_secs = now.duration_since(prev_at).as_secs_f64();
        (elapsed_secs >= 1.0)
            .then(|| (overview.total_spent_usd - prev_spent) / elapsed_secs * 3600.0)
    });
    *last = Some((now, overview.total_spent_usd));
    drop(last);

    let runs_word = if overview.active_runs == 1 {
        "run"
    } else {
        "runs"
    };
    match rate_per_hour {
        Some(rate) if rate.is_finite() => {
            let sign = if rate < 0.0 { "-" } else { "+" };
            format!(
                "Spent {} ({sign}{}/hr) - {} active {runs_word}",
                format_usd(overview.total_spent_usd),
                format_usd(rate.abs()),
                overview.active_runs
            )
        }
        _ => format!(
            "Spent {} - {} active {runs_word}",
            format_usd(overview.total_spent_usd),
            overview.active_runs
        ),
    }
}

/// 2 decimals normally, 4 for sub-dollar amounts - mirrors
/// `MoneyFormat.usd` (SwiftUI) / `formatUsd` (`lib/format.ts`) exactly, so
/// all three surfaces render the same figure identically.
fn format_usd(value: f64) -> String {
    let decimals = if value != 0.0 && value.abs() < 1.0 {
        4
    } else {
        2
    };
    format!("${value:.decimals$}")
}

/// First 12 characters of a run id plus an ellipsis if it does not already
/// fit - a native tray menu has far less width than `RunsTable`'s own `run`
/// column (which shows the id in full with a hover tooltip; menu items have
/// no equivalent affordance).
fn short_run_id(run_id: &str) -> String {
    const MAX_CHARS: usize = 12;
    if run_id.chars().count() <= MAX_CHARS {
        return run_id.to_string();
    }
    let prefix: String = run_id.chars().take(MAX_CHARS).collect();
    format!("{prefix}…")
}

/// Best-effort menu-item update: a failure to relabel/enable a native menu
/// item is logged, never panics and never blocks the caller (fail-closed,
/// same convention as `live.rs`'s own best-effort logging for a bad feeder
/// tick).
fn log_menu_result(action: &str, result: tauri::Result<()>) {
    if let Err(e) = result {
        eprintln!("genaryx: tray failed to {action}: {e}");
    }
}
