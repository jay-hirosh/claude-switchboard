mod app_state;
pub mod auth;
pub mod branding;
pub mod cli;
mod commands;
pub mod jsonl_parser;
pub mod logging;
pub mod migration;
pub mod notifier;
mod poll_loop;
mod process_detection;
pub mod providers;
pub mod os_scheduler;
pub mod scheduler;
pub mod scheduler_glue;
pub mod sessions;
pub mod store;
mod tray;
mod tray_icon;
mod updater;
pub mod usage_api;
pub mod warmup;

use app_state::AppState;
use std::sync::Arc;

/// Move `window` to the tray icon's center, with a panic-safe fallback.
///
/// `tauri_plugin_positioner::WindowExt::move_window(Position::TrayCenter)`
/// panics inside the plugin's `calculate_position` (`ext.rs`) via
/// `window.current_monitor()?.unwrap()` when the window isn't on any monitor
/// — e.g. shown before it has been placed, or mid display-reconfiguration.
/// Under this crate's `panic = "abort"` release profile that aborts the whole
/// app (and in dev it tears down the process via the worker-thread panic).
/// We check `current_monitor()` ourselves first and fall back to
/// `Position::TopRight` (monitor work-area only, never tray-dependent) when
/// there's no current monitor — the same fallback used during first-run setup.
pub(crate) fn move_to_tray_center<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    use tauri_plugin_positioner::{Position, WindowExt};
    let on_monitor = window.current_monitor().ok().flatten().is_some();
    let _ = window.move_window(if on_monitor {
        Position::TrayCenter
    } else {
        Position::TopRight
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let log_dir = logging::log_dir();
    let _log_guard = logging::init(log_dir.clone());

    // Built and exported before the DB is opened: the export is a pure
    // function of the command list, and doing it here means `cargo run`
    // regenerates bindings.ts even when another instance already holds
    // the DB lock (which otherwise exits the process first).
    // tauri-specta's Builder::commands replaces previously registered commands rather
    // than appending, so debug-only handlers must be folded into the same collect_commands! call.
    #[cfg(not(debug_assertions))]
    let specta_builder = tauri_specta::Builder::<tauri::Wry>::new()
        .commands(tauri_specta::collect_commands![
            commands::get_current_usage,
            commands::get_pricing,
            commands::get_session_history,
            commands::get_daily_trends,
            commands::get_model_breakdown,
            commands::get_daily_model_breakdown,
            commands::get_project_breakdown,
            commands::get_cache_stats,
            commands::start_oauth_flow,
            commands::has_claude_code_creds,
            commands::update_settings,
            commands::get_settings,
            commands::resize_window,
            commands::force_refresh,
            commands::check_for_updates_now,
            commands::install_update,
            commands::list_accounts,
            commands::add_account_from_claude_code,
            commands::remove_account,
            commands::swap_to_account,
            commands::detect_running_claude_code,
            commands::refresh_account,
            commands::warmup_account_now,
            commands::set_account_schedule,
            commands::set_warmup_enabled,
            commands::grant_warmup_consent,
            commands::revoke_warmup_consent,
            commands::get_warmup_consent_granted,
            commands::get_warmup_state,
            commands::os_scheduler_register,
            commands::os_scheduler_unregister,
            commands::os_scheduler_is_registered,
            commands::list_providers,
            commands::upsert_provider,
            commands::delete_provider,
            commands::list_provider_presets,
            commands::list_available_terminals,
            commands::launch_provider_session,
            commands::get_provider_launch_command,
            commands::get_default_provider,
            commands::set_default_provider,
            commands::clear_default_provider,
        ]);

    #[cfg(debug_assertions)]
    let specta_builder = tauri_specta::Builder::<tauri::Wry>::new()
        .commands(tauri_specta::collect_commands![
            commands::get_current_usage,
            commands::get_pricing,
            commands::get_session_history,
            commands::get_daily_trends,
            commands::get_model_breakdown,
            commands::get_daily_model_breakdown,
            commands::get_project_breakdown,
            commands::get_cache_stats,
            commands::start_oauth_flow,
            commands::has_claude_code_creds,
            commands::update_settings,
            commands::get_settings,
            commands::resize_window,
            commands::force_refresh,
            commands::check_for_updates_now,
            commands::install_update,
            commands::list_accounts,
            commands::add_account_from_claude_code,
            commands::remove_account,
            commands::swap_to_account,
            commands::detect_running_claude_code,
            commands::refresh_account,
            commands::debug_force_threshold,
            commands::warmup_account_now,
            commands::set_account_schedule,
            commands::set_warmup_enabled,
            commands::grant_warmup_consent,
            commands::revoke_warmup_consent,
            commands::get_warmup_consent_granted,
            commands::get_warmup_state,
            commands::os_scheduler_register,
            commands::os_scheduler_unregister,
            commands::os_scheduler_is_registered,
            commands::list_providers,
            commands::upsert_provider,
            commands::delete_provider,
            commands::list_provider_presets,
            commands::list_available_terminals,
            commands::launch_provider_session,
            commands::get_provider_launch_command,
            commands::get_default_provider,
            commands::set_default_provider,
            commands::clear_default_provider,
        ]);

    #[cfg(debug_assertions)]
    specta_builder
        .export(
            specta_typescript::Typescript::default()
                .bigint(specta_typescript::BigIntExportBehavior::Number)
                .header("// @ts-nocheck"),
            "../src/lib/generated/bindings.ts",
        )
        .expect("failed to export specta bindings");

    let data_dir = store::default_dir();

    let db_result = store::Db::open(&data_dir).unwrap_or_else(|e| {
        tracing::error!("fatal: cannot open or recover the database: {e}");
        std::process::exit(1);
    });

    // Mark startup migration complete (no-op idempotent set; reserved for
    // future migrations).
    {
        let conn = db_result.conn();
        if let Err(e) = crate::migration::mark_complete(&conn) {
            tracing::warn!("failed to set migration_completed flag: {e:#}");
        }
    }
    let db_recovered = db_result.recovered;
    let db = Arc::new(db_result);
    let pricing = Arc::new(jsonl_parser::PricingTable::bundled().expect("pricing"));

    // One shared HTTP client for all outbound requests (usage API, token
    // exchange, and identity fetcher).  Built once with the canonical timeout
    // configuration so every caller benefits from connection-pool reuse.
    let http_client = Arc::new(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("http client"),
    );

    let usage_client = Arc::new(
        usage_api::UsageClient::new(http_client.clone(), env!("CARGO_PKG_VERSION").to_string()),
    );

    let persisted_settings = db
        .load_settings()
        .unwrap_or_else(|e| {
            tracing::warn!("failed to load persisted settings, using defaults: {e}");
            None
        })
        .unwrap_or_default();

    let auth = Arc::new(auth::AuthOrchestrator::new(data_dir.clone(), http_client.clone()));

    let accounts = Arc::new(crate::auth::accounts::AccountManager::new(data_dir.clone()));

    let app_state = Arc::new(AppState {
        db: db.clone(),
        auth,
        usage: usage_client,
        http_client: http_client.clone(),
        pricing: pricing.clone(),
        settings: parking_lot::RwLock::new(persisted_settings),
        cached_usage: parking_lot::RwLock::new(None),
        force_refresh: tokio::sync::Notify::new(),
        accounts,
        cached_usage_by_slot: parking_lot::RwLock::new(std::collections::HashMap::new()),
        active_slot: parking_lot::RwLock::new(None),
        backoff_by_slot: parking_lot::RwLock::new(std::collections::HashMap::new()),
        schedule_by_slot: parking_lot::RwLock::new(std::collections::HashMap::new()),
        keychain_guardian: parking_lot::Mutex::new(None),
        warmup: app_state::WarmupState::default(),
        tray_position_known: std::sync::atomic::AtomicBool::new(false),
    });

    // One-time per pricing revision: recompute historical event costs with
    // the current table (covers events costed 0.0 before their model had a
    // pricing entry, and events costed under corrected rates).
    match app_state.db.repriced_version() {
        Ok(Some(v)) if v >= crate::jsonl_parser::pricing::PRICING_VERSION => {}
        _ => {
            match app_state.db.reprice_outdated_events(&pricing) {
                Ok(n) => {
                    if n > 0 {
                        tracing::info!("repriced {n} session events to pricing v{}", crate::jsonl_parser::pricing::PRICING_VERSION);
                    }
                    if let Err(e) = app_state
                        .db
                        .set_repriced_version(crate::jsonl_parser::pricing::PRICING_VERSION)
                    {
                        tracing::warn!("set_repriced_version failed: {e}");
                    }
                }
                Err(e) => tracing::warn!("reprice migration failed: {e}"),
            }
        }
    }

    // Rehydrate per-slot usage caches from the most recent persisted API
    // snapshots so the popover and account rows have last-known-good data
    // before the poll loop's first fetch. Without this, a cold start during
    // a rate-limit window shows "usage unavailable" until a fetch succeeds.
    {
        let managed = app_state.accounts.list().unwrap_or_default();
        let hydrated = poll_loop::hydrated_caches(&app_state.db, &managed);
        if !hydrated.is_empty() {
            tracing::info!("hydrated usage cache for {} slot(s)", hydrated.len());
            *app_state.cached_usage_by_slot.write() = hydrated;
        }
        // Snapshots accrue on every successful poll; keep the table bounded.
        if let Err(e) = app_state.db.prune_snapshots(50) {
            tracing::warn!("prune_snapshots failed: {e}");
        }
    }


    tauri::Builder::default()
        .manage(app_state)
        .manage(std::sync::Arc::new(crate::updater::UpdaterGuard::default()))
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            use tauri::{Emitter, Manager};
            if let Some(w) = app.get_webview_window("popover") {
                crate::move_to_tray_center(&w);
                let _ = w.show();
                let _ = w.set_focus();
                let _ = w.app_handle().emit("popover_shown", ());
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(specta_builder.invoke_handler())
        .setup(move |app| {
            use tauri::Manager;
            let handle = app.handle().clone();
            let state: Arc<AppState> = app.state::<Arc<AppState>>().inner().clone();

            // The official provider row must exist before the Providers tab
            // can render a list. Idempotent, so it also repairs a row deleted
            // by hand and populates databases created before this feature.
            if let Err(e) = state.db.seed_official_provider() {
                tracing::warn!("failed to seed official provider row: {e:#}");
            }
            match crate::providers::launcher::sweep_scripts(
                &crate::providers::launcher::script_dir(),
            ) {
                Ok(n) if n > 0 => tracing::info!("swept {n} stale launch script(s)"),
                Ok(_) => {}
                Err(e) => tracing::warn!("launch script sweep failed: {e:#}"),
            }
            // Recurring sweep — a startup-only pass leaves token-bearing
            // scripts on disk for the entire uptime of a resident menu-bar app.
            tokio::spawn(async {
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(1800));
                tick.tick().await; // consume the immediate first tick
                loop {
                    tick.tick().await;
                    if let Err(e) = crate::providers::launcher::sweep_scripts(
                        &crate::providers::launcher::script_dir(),
                    ) {
                        tracing::warn!("periodic launch script sweep failed: {e:#}");
                    }
                }
            });

            // Regular activation policy on macOS: the app shows up in
            // Cmd+Tab and gets a Dock icon, alongside the tray icon. The
            // earlier `Accessory` policy made this a menubar-only utility,
            // but the user wants it switchable like a normal app.
            #[cfg(target_os = "macos")]
            {
                app.set_activation_policy(tauri::ActivationPolicy::Regular);
            }

            // Force the popover to its configured fixed size on every launch.
            if let Some(popover) = app.get_webview_window("popover") {
                use tauri::{LogicalSize, Size};
                let _ = popover.set_size(Size::Logical(LogicalSize::new(360.0, 380.0)));

                // Intercept the OS close button: by default Tauri DESTROYS the
                // window, after which get_webview_window("popover") returns None
                // and the app can never reopen. Hide instead so the window
                // survives for next show().
                use tauri::Emitter;
                let popover_clone = popover.clone();
                popover.on_window_event(move |ev| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = ev {
                        api.prevent_close();
                        let _ = popover_clone.hide();
                        let _ = popover_clone.app_handle().emit("popover_hidden", ());
                    }
                });
            }

            // Apply native vibrancy to the popover so it reads as a Control
            // Center / Raycast-style menubar widget instead of a flat panel.
            // The radius MUST match the `--radius-lg` token used by `#root`'s
            // border-radius — otherwise the NSVisualEffectView stays
            // rectangular and a sharp-cornered dark plate is visible behind
            // the rounded HTML surface.
            #[cfg(target_os = "macos")]
            if let Some(popover) = app.get_webview_window("popover") {
                use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
                let _ = apply_vibrancy(
                    &popover,
                    NSVisualEffectMaterial::HudWindow,
                    Some(NSVisualEffectState::Active),
                    Some(14.0),
                );
            }
            #[cfg(target_os = "windows")]
            if let Some(_popover) = app.get_webview_window("popover") {
                // We do not apply mica/acrylic on Windows because it fills the entire
                // sharp rectangular bounds of the frameless window, causing white/gray
                // corners to be visible outside our CSS `border-radius`. The CSS fallback
                // (oklch 0.86 alpha) looks better than having sharp artifact corners.
            }

            // Tray icon — configure the one Tauri auto-created from the
            // `trayIcon` block in tauri.conf.json. Don't build a NEW one
            // (that would create a second NSStatusItem that competes with
            // the visible config-driven one — when the user reported "two
            // duplicated icons" earlier, that was this exact double-creation,
            // and removing the config block left us with only the invisible
            // programmatic item).
            use tauri::menu::{MenuBuilder, MenuItem};
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};

            let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let check_updates = MenuItem::with_id(
                app,
                "check_updates",
                "Check for Updates…",
                true,
                None::<&str>,
            )?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = MenuBuilder::new(app)
                .items(&[&show, &check_updates, &quit])
                .build()?;

            if let Some(tray) = app.tray_by_id("main") {
                tracing::info!("attaching menu + handlers to config-created tray");
                let _ = tray.set_menu(Some(menu));
                let _ = tray.set_show_menu_on_left_click(false);
                tray.on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("popover") {
                            use tauri::Emitter;
                            crate::move_to_tray_center(&w);
                            let _ = w.show();
                            let _ = w.set_focus();
                            let _ = app.emit("popover_shown", ());
                        }
                    }
                    "check_updates" => {
                        let app_clone = app.clone();
                        tauri::async_runtime::spawn(async move {
                            crate::updater::check_and_emit(&app_clone).await;
                        });
                    }
                    "quit" => app.exit(0),
                    _ => {}
                });
                tray.on_tray_icon_event(|tray, event| {
                    tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
                    tray.app_handle()
                        .state::<Arc<AppState>>()
                        .tray_position_known
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("popover") {
                            if w.is_visible().unwrap_or(false) {
                                let _ = w.hide();
                                use tauri::Emitter;
                                let _ = w.app_handle().emit("popover_hidden", ());
                            } else {
                                use tauri::Emitter;
                                crate::move_to_tray_center(&w);
                                let _ = w.show();
                                let _ = w.set_focus();
                                let _ = w.app_handle().emit("popover_shown", ());
                            }
                        }
                    }
                });
            } else {
                tracing::error!(
                    "tray_by_id('main') returned None — tauri.conf.json `trayIcon` block missing?"
                );
            }

            // First-run UX: a tray-only launch on a fresh install looks like
            // nothing happened — the user can't find the menubar icon and
            // assumes the app didn't open. Auto-show the popover the very
            // first time we boot on this machine, then drop a marker so every
            // subsequent launch is silent (tray-only, as designed).
            //
            // Trigger on EITHER signal:
            //   - Sentinel file missing → first launch after install/upgrade.
            //   - No managed accounts → user needs the AuthPanel anyway.
            let first_run_marker = data_dir.join(".first_run_done");
            let no_marker = !first_run_marker.exists();
            let no_accounts = state
                .accounts
                .list()
                .map(|v| v.is_empty())
                .unwrap_or(true);
            if no_marker || no_accounts {
                if let Some(popover) = app.get_webview_window("popover") {
                    use tauri::Emitter;
                    use tauri_plugin_positioner::{Position, WindowExt};
                    // Position::TrayCenter would panic here — the positioner
                    // plugin caches the tray rect from `on_tray_event` calls,
                    // and no tray event has fired yet during `setup`. Use
                    // TopRight, which only needs the monitor work-area and is
                    // visually close to the menubar tray icon anyway.
                    let _ = popover.move_window(Position::TopRight);
                    let _ = popover.show();
                    let _ = popover.set_focus();
                    let _ = popover.app_handle().emit("popover_shown", ());
                }
                if no_marker {
                    if let Err(e) = std::fs::write(&first_run_marker, b"") {
                        tracing::warn!(
                            "could not write first-run marker {first_run_marker:?}: {e}"
                        );
                    }
                }
            }

            // Emit db_reset if the DB was corrupt and had to be recreated.
            // We do this here (inside `setup`) so the app handle is available.
            // The event is fired from a short-lived task to avoid blocking the
            // setup hook; the frontend subscribes before the first render, so
            // the slight async delay is harmless.
            if db_recovered {
                let h = handle.clone();
                tauri::async_runtime::spawn(async move {
                    use tauri::Emitter;
                    let _ = h.emit("db_reset", ());
                    tracing::warn!("emitted db_reset event — DB was corrupt and has been recreated");
                });
            }

            {
                let h = handle.clone();
                let dir = data_dir.clone();
                let identity_fetcher = state.auth.identity_arc();
                tauri::async_runtime::spawn(async move {
                    use tauri::Emitter;
                    match crate::auth::accounts::migrate_legacy(&dir, identity_fetcher).await {
                        Ok(report) if !report.imported_slots.is_empty() => {
                            tracing::info!(
                                "migrated {} legacy account(s)",
                                report.imported_slots.len()
                            );
                            let _ = h.emit("migrated_accounts", &report.imported_slots);
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!("legacy migration failed: {e}");
                        }
                    }
                });
            }

            poll_loop::spawn(handle.clone(), state.clone());
            crate::updater::run_scheduler(handle.clone());

            // Backfill the SQLite mirror for accounts that were added before
            // the mirror existed (or before this fix shipped). Without this,
            // get_warmup_state / set_warmup_enabled / warmup_account_now all
            // operate on rows that don't exist and silently no-op (or error
            // on the load path), making the warm-up feature appear broken.
            // The legacy-migration spawn above runs in a tokio task and we
            // want this to wait on it, so do the reconcile inside its own
            // task that runs after the migration future would have completed.
            {
                let recon_state = state.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) =
                        commands::reconcile_sqlite_account_mirror(&recon_state)
                    {
                        tracing::warn!("startup SQLite mirror reconcile failed: {e:#}");
                    }
                });
            }

            // In-app warm-up dispatcher: wakes every 30 seconds, walks
            // accounts with warmup_enabled = 1, calls tick_for_account per
            // account. Mirrors the poll_loop::spawn pattern above.
            {
                let warmup_state = state.clone();
                tauri::async_runtime::spawn(async move {
                    let mut interval = tokio::time::interval(
                        std::time::Duration::from_secs(30),
                    );
                    interval.set_missed_tick_behavior(
                        tokio::time::MissedTickBehavior::Delay,
                    );
                    loop {
                        interval.tick().await;
                        if let Err(e) =
                            scheduler_glue::walk_due_accounts(&warmup_state).await
                        {
                            tracing::warn!("warmup dispatcher iter failed: {e:#}");
                        }
                    }
                });
            }

            if let Some(root) = jsonl_parser::walker::claude_projects_root() {
                let bf_root = root.clone();
                let bf_state = state.clone();
                tauri::async_runtime::spawn(async move {
                    if let Ok(files) = jsonl_parser::walker::discover_jsonl_files(&bf_root) {
                        for f in files {
                            let _ = jsonl_parser::walker::ingest_file(
                                &bf_state.db,
                                &bf_state.pricing,
                                &f,
                                &bf_root,
                            );
                        }
                    }
                });

                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<usize>();
                let handle_for_events = handle.clone();
                tauri::async_runtime::spawn(async move {
                    use tauri::Emitter;
                    while let Some(n) = rx.recv().await {
                        let _ = handle_for_events.emit("session_ingested", n);
                    }
                });
                // The WatcherHandle owns the notify-debouncer that drives the
                // OS file watcher. Drop it and the debouncer is destroyed, the
                // watcher stops, and no new JSONL writes are ever ingested —
                // the report appears to "stop updating" mid-session and only
                // refreshes when the app restarts (because the backfill above
                // re-scans every file from scratch). Leak it so it lives for
                // the process lifetime, which is the lifetime we want anyway.
                match jsonl_parser::watcher::start(
                    state.db.clone(),
                    state.pricing.clone(),
                    root,
                    tx,
                ) {
                    Ok(handle) => {
                        Box::leak(Box::new(handle));
                    }
                    Err(e) => {
                        tracing::error!("jsonl watcher failed to start: {e}");
                        use tauri::Emitter;
                        let _ = handle.emit("watcher_error", e.to_string());
                    }
                }
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, _event| {
            // Dock-click on macOS (NSApplicationDelegate applicationShouldHandleReopen):
            // when the user activates the app while no window is visible, the
            // popover stays hidden and the click looks broken. Re-show it.
            // RunEvent::Reopen is macOS-only; Windows taskbar activation is
            // handled separately by the single-instance plugin.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { has_visible_windows, .. } = _event {
                if !has_visible_windows {
                    use tauri::Manager;
                    if let Some(w) = _app.get_webview_window("popover") {
                        use tauri::Emitter;
                        let _ = w.show();
                        let _ = w.set_focus();
                        let _ = w.app_handle().emit("popover_shown", ());
                    }
                }
            }
        });
}
