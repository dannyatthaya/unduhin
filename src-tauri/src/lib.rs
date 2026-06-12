//! Tauri shell for Unduhin.
//!
//! The Rust side here is intentionally thin: it owns a single
//! [`unduhin_core::Core`] instance in [`tauri::State`], exposes one
//! Tauri command per public `Core` method (see [`commands`]), and
//! forwards every [`unduhin_core::CoreEvent`] to the frontend on a
//! single `"unduhin:event"` channel.

pub mod browser_integration;
pub mod commands;
pub mod error;
pub mod i18n;
pub mod manifest;
pub mod pipe;
pub mod taskbar;
pub mod tray;
pub mod window;

use std::path::PathBuf;

use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use unduhin_core::Core;

use crate::window::ConfirmOnQuitBridge;

/// Name of the single Tauri event that carries every `CoreEvent`. The
/// frontend subscribes to this once and dispatches on the `type` tag.
pub const EVENT_CHANNEL: &str = "unduhin:event";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize structured logging (stderr + rotating file). Failure to
    // create the log directory is non-fatal — we fall back to stderr.
    if let Err(e) = unduhin_core::logging::init() {
        eprintln!("warning: failed to initialize file logger: {e}");
    }

    let runtime = tokio::runtime::Runtime::new().expect("failed to start tokio runtime");

    let core = runtime
        .block_on(async {
            let path = resolve_db_path();
            let core = Core::open(&path).await?;
            core.start().await?;
            Ok::<_, unduhin_core::CoreError>(core)
        })
        .expect("failed to open unduhin-core");

    // Seed the Rust-side i18n locale from the persisted `language`
    // setting *before* the tray spawns. Tray menu labels and tooltip
    // are read from this module on first paint; without the seed the
    // first paint would always be in English and only flip on the next
    // `SettingChanged` event.
    {
        let raw = runtime.block_on(async {
            core.get_setting(unduhin_core::settings_keys::LANGUAGE)
                .await
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(str::to_owned))
        });
        i18n::set_locale(i18n::Locale::from_setting(raw.as_deref()));
    }

    let runtime_handle = runtime.handle().clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // Duplicate-launch handler. Brings the existing main window to the
            // foreground so the second `unduhin-app.exe` invocation is a no-op
            // from the user's perspective. Errors are swallowed: there is no
            // sensible recovery path if `show()`/`set_focus()` fail here.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(core.clone())
        .manage(ConfirmOnQuitBridge::new())
        .on_window_event(|window, event| {
            // Only the main window participates in the close-behavior
            // policy. The tray-popover window hides itself on focus loss
            // and is not user-closable.
            if window.label() != "main" {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                // We own the decision; cancel the default close and dispatch
                // to the async handler. `app.exit(0)` (Exit / Ask-confirm)
                // and `window.hide()` (Minimize) are how the handler
                // ultimately resolves the request.
                api.prevent_close();
                let app = window.app_handle().clone();
                let core = app.state::<Core>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    window::handle_close_requested(app, core).await;
                });
            }
        })
        .setup({
            let core = core.clone();
            let runtime = runtime_handle.clone();
            move |app| {
                forward_core_events(app.handle().clone(), core.clone());
                if let Err(e) = manifest::reconcile_native_host_manifest(app.handle()) {
                    tracing::warn!(error = %e, "failed to reconcile native-host manifest path");
                }
                pipe::install(app.handle().clone(), core.clone());
                reconcile_autostart(app.handle().clone(), core.clone(), runtime.clone());
                // tauri.conf.json now has the main window at `visible:
                // false`. We surface it here unless the user asked for
                // `start_minimized` — the tray is the only surface in
                // that case (which 7d also wires).
                let app_handle = app.handle().clone();
                let core_for_show = core.clone();
                tauri::async_runtime::spawn(async move {
                    window::apply_start_minimized(app_handle, core_for_show).await;
                });
                // Spawn the borderless tray popover hidden so the first
                // surface on tray click is instant.
                if let Err(e) = window::ensure_popover_window(app.handle()) {
                    tracing::warn!(error = %e, "failed to create tray popover window");
                }
                // Install the tray *after* the window is created so the
                // initial mirror-seed task has a webview to talk to when
                // the user clicks the menu items.
                if let Err(e) = tray::install(app.handle(), core.clone()) {
                    tracing::warn!(error = %e, "failed to install tray icon");
                }
                Ok(())
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::get_disk_info,
            commands::add_download,
            commands::list_downloads,
            commands::get_download,
            commands::pause_download,
            commands::resume_download,
            commands::retry_download,
            commands::remove_download,
            commands::set_priority,
            commands::set_segments,
            commands::set_category,
            commands::preview_filename,
            commands::pause_all,
            commands::resume_all,
            commands::list_categories,
            commands::add_category,
            commands::update_category,
            commands::remove_category,
            commands::set_category_order,
            commands::get_settings,
            commands::get_setting,
            commands::set_setting,
            commands::probe_media_url,
            commands::fetch_torrent_metadata,
            commands::tool_status,
            commands::install_tool,
            commands::record_update_check,
            commands::get_logs_dir,
            commands::confirm_quit_response,
            commands::quit_app,
            commands::list_schedules,
            commands::add_schedule,
            commands::update_schedule,
            commands::remove_schedule,
            commands::get_quiet_hours_state,
            commands::get_browser_integration_status,
            commands::test_pipe_handoff,
            commands::get_extension_settings,
            commands::apply_extension_settings_patch,
            commands::get_rule_metrics,
            commands::respond_handoff,
            commands::start_handoff_download,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build tauri app")
        .run(move |_app, event| {
            if let RunEvent::ExitRequested { .. } = event {
                // Flush state (bounded), then HARD-EXIT. librqbit's session
                // keeps background tasks alive (DHT, peer sockets, blocking disk
                // I/O); on a normal return the tokio runtime's drop blocks
                // waiting on them, which surfaces as the window hanging on "Not
                // Responding". `process::exit` skips that teardown. Interrupted
                // downloads are recoverable: `Core::open` re-queues any row left
                // `active`/`muxing` on the next launch.
                let core_for_shutdown = _app.state::<Core>().inner().clone();
                let _ = runtime_handle.block_on(async {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        core_for_shutdown.shutdown(),
                    )
                    .await
                });
                std::process::exit(0);
            }
        });
}

fn resolve_db_path() -> PathBuf {
    if let Ok(env) = std::env::var("UNDUHIN_DB") {
        if !env.is_empty() {
            return PathBuf::from(env);
        }
    }
    unduhin_core::default_db_path().unwrap_or_else(|| PathBuf::from("unduhin.db"))
}

/// Read the persisted `autostart` setting and reconcile it with the
/// OS-level state owned by `tauri-plugin-autostart`. The plugin's
/// registry key can be wiped by Windows updates or other tools, so we
/// treat the setting as the source of truth.
///
/// Called from two paths:
/// - **On startup** (via [`reconcile_autostart`]), so a registry flip
///   while the app was closed is fixed up before the user opens
///   Settings.
/// - **On every `SettingChanged { key: "autostart" }`** (via
///   [`forward_core_events`]), so external tools flipping the key while
///   the app is running can't leave the surface silently out of sync.
///   The check runs unconditionally on this signal — it's cheap, and
///   `setting_changed` fires only on the user's own writes through
///   Tauri, not on every settings table modification.
async fn reconcile_autostart_once(app: &AppHandle, core: &Core, source: &'static str) {
    use tauri_plugin_autostart::ManagerExt;
    let desired = core
        .get_setting(unduhin_core::settings_keys::AUTOSTART)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let manager = app.autolaunch();
    let current = manager.is_enabled().unwrap_or(false);
    if desired == current {
        return;
    }
    tracing::warn!(
        source,
        desired,
        current,
        "autostart drift detected — reconciling registry to match persisted setting"
    );
    let result = if desired {
        manager.enable()
    } else {
        manager.disable()
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, "failed to reconcile autostart");
    }
}

/// On startup, read the persisted `autostart` setting and reconcile it
/// with the OS-level state. Defers to [`reconcile_autostart_once`] so the
/// startup path and the live `SettingChanged` path share the same
/// drift-detection log line.
fn reconcile_autostart(app: AppHandle, core: Core, runtime: tokio::runtime::Handle) {
    runtime.spawn(async move {
        reconcile_autostart_once(&app, &core, "startup").await;
    });
}

/// Short human-readable OS line shown on the About page.
/// E.g. `"Windows 11 · x64 · 22631.3593"` on a Win11 23H2 machine.
pub fn os_summary() -> String {
    let arch = std::env::consts::ARCH;
    let arch_pretty = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };

    #[cfg(target_os = "windows")]
    {
        let name = sysinfo::System::name().unwrap_or_else(|| "Windows".into());
        let version = sysinfo::System::os_version().unwrap_or_default();
        let kernel = sysinfo::System::kernel_version().unwrap_or_default();
        let mut parts = vec![name];
        if !version.is_empty() {
            parts.push(version);
        }
        parts.push(arch_pretty.into());
        if !kernel.is_empty() {
            parts.push(kernel);
        }
        parts.join(" · ")
    }

    #[cfg(not(target_os = "windows"))]
    {
        let name = sysinfo::System::name().unwrap_or_else(|| std::env::consts::OS.into());
        let version = sysinfo::System::os_version().unwrap_or_default();
        let mut parts = vec![name];
        if !version.is_empty() {
            parts.push(version);
        }
        parts.push(arch_pretty.into());
        parts.join(" · ")
    }
}

/// Spawn a background task that subscribes to `core`'s event stream and
/// rebroadcasts every event onto the single Tauri channel.
///
/// In addition to the forwarding, two side-effects ride this stream:
/// - **Autostart drift recheck** — every `SettingChanged { key: "autostart" }`
///   re-runs [`reconcile_autostart_once`] so an external tool flipping
///   the registry key while the app is running can't outlast the next
///   settings write.
fn forward_core_events(app: AppHandle, core: Core) {
    let mut rx = core.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(e) = app.emit(EVENT_CHANNEL, &event) {
                        tracing::warn!("failed to emit unduhin:event: {e}");
                    }
                    if let unduhin_core::CoreEvent::SettingChanged { key } = &event {
                        if key == unduhin_core::settings_keys::AUTOSTART {
                            let app = app.clone();
                            let core = core.clone();
                            tauri::async_runtime::spawn(async move {
                                reconcile_autostart_once(&app, &core, "setting_changed").await;
                            });
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("frontend event listener lagged by {n} events");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
