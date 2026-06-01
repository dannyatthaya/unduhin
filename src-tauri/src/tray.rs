//! System tray icon, state-machine reducer, and menu.
//!
//! Subscribes to the core event bus on a fresh receiver (independent of
//! [`crate::forward_core_events`]) and recomputes a four-state model:
//! `Idle | Downloading(n) | Paused | Error`. Icon swaps fire only when
//! the *variant* changes — Win32 tray icons can't carry text badges, so
//! a `Downloading(3) → Downloading(4)` transition just refreshes the
//! tooltip + popover, not the icon.
//!
//! The `Error` state is sticky: once any row enters `Failed`, the tray
//! stays in `Error` until no `Failed` rows remain (the user retried or
//! removed them). This matches the user's expectation that errors need
//! attention before the tray "calms down".

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

use unduhin_core::{Core, CoreEvent, DownloadFilter, DownloadId, Status};

use crate::{i18n, taskbar};

/// The single Tauri event the tray emits when the user clicks *Add URL…*
/// in the tray menu. The downloads view subscribes once and opens the
/// existing `AddUrlDialog` in response.
const OPEN_ADD_URL_EVENT: &str = "unduhin:open-add-url";
/// Emitted when the tray's "Check for updates…" item is clicked; the
/// frontend routes to Settings → About and runs the update check.
const CHECK_UPDATES_EVENT: &str = "unduhin:check-updates";

const TRAY_ICON_ID: &str = "tray";

const ICON_IDLE: &[u8] = include_bytes!("../icons/tray/idle.png");
const ICON_DOWNLOADING: &[u8] = include_bytes!("../icons/tray/downloading.png");
const ICON_PAUSED: &[u8] = include_bytes!("../icons/tray/paused.png");
const ICON_ERROR: &[u8] = include_bytes!("../icons/tray/error.png");

/// Aggregate state the tray reflects. `Downloading` carries the in-flight
/// row count so the tooltip + popover can show it, but the icon is the
/// same for every `Downloading(n)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrayState {
    Idle,
    Downloading { n: u32 },
    Paused,
    Error,
}

impl TrayState {
    /// Two states share an icon iff they share a variant. The `n` inside
    /// `Downloading` does not change the icon — see module docs.
    fn icon_variant(self) -> IconVariant {
        match self {
            TrayState::Idle => IconVariant::Idle,
            TrayState::Downloading { .. } => IconVariant::Downloading,
            TrayState::Paused => IconVariant::Paused,
            TrayState::Error => IconVariant::Error,
        }
    }

    fn tooltip(self) -> String {
        match self {
            TrayState::Idle => i18n::t("tray", "tooltipIdle"),
            TrayState::Downloading { n } => {
                i18n::t_with("tray", "tooltipDownloading", &[("n", &n)])
            }
            TrayState::Paused => i18n::t("tray", "tooltipPaused"),
            TrayState::Error => i18n::t("tray", "tooltipError"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IconVariant {
    Idle,
    Downloading,
    Paused,
    Error,
}

impl IconVariant {
    fn bytes(self) -> &'static [u8] {
        match self {
            IconVariant::Idle => ICON_IDLE,
            IconVariant::Downloading => ICON_DOWNLOADING,
            IconVariant::Paused => ICON_PAUSED,
            IconVariant::Error => ICON_ERROR,
        }
    }
}

/// One row in the tray's status + progress mirror. We track downloaded /
/// total bytes so the taskbar can show an aggregate progress bar driven
/// off the same broadcast subscription as the icon — no second event
/// pipeline.
#[derive(Debug, Clone, Copy)]
pub struct RowView {
    pub status: Status,
    pub downloaded: u64,
    pub total: Option<u64>,
}

impl Default for RowView {
    fn default() -> Self {
        // `Queued` is the right default for an empty entry inserted via
        // `entry(...).or_default()` — it doesn't bias the tray state into
        // any of the priority buckets (`Active`, `Paused`, or `Failed`).
        // A genuine row will overwrite this immediately on `DownloadAdded`.
        Self {
            status: Status::Queued,
            downloaded: 0,
            total: None,
        }
    }
}

/// Pure reducer: applies one `CoreEvent` to the row mirror and
/// recomputes the tray state. Lifted so unit tests can drive it without
/// spinning up Tauri.
pub fn apply_event(mirror: &mut HashMap<DownloadId, RowView>, event: &CoreEvent) {
    match event {
        CoreEvent::DownloadAdded { id, snapshot } => {
            mirror.insert(
                *id,
                RowView {
                    status: snapshot.status,
                    downloaded: snapshot.downloaded_bytes,
                    total: snapshot.total_bytes,
                },
            );
        }
        CoreEvent::StatusChanged { id, to, .. } => {
            let row = mirror.entry(*id).or_default();
            row.status = *to;
        }
        CoreEvent::ProgressUpdate {
            id,
            downloaded,
            total,
            ..
        } => {
            let row = mirror.entry(*id).or_default();
            row.downloaded = *downloaded;
            // Engine only ever transitions `total` from None → Some; we
            // keep it sticky to avoid a momentary `Some → None → Some`
            // wobble dropping the aggregate denominator on the taskbar.
            if total.is_some() {
                row.total = *total;
            }
        }
        CoreEvent::Completed { id, bytes } => {
            let row = mirror.entry(*id).or_default();
            row.status = Status::Completed;
            row.downloaded = *bytes;
            if row.total.is_none() {
                row.total = Some(*bytes);
            }
        }
        CoreEvent::Failed { id, .. } => {
            let row = mirror.entry(*id).or_default();
            row.status = Status::Failed;
        }
        CoreEvent::Removed { id } => {
            mirror.remove(id);
        }
        // `QueueEmptied` is a one-shot signal with no per-row payload —
        // the icon and tooltip already reflect the drained state via the
        // status mirror.
        _ => {}
    }
}

/// Compute the tray state from the current status mirror. See the
/// module docs for the priority rules (Error sticks until no Failed
/// rows remain).
pub fn compute_state(mirror: &HashMap<DownloadId, RowView>) -> TrayState {
    let mut any_failed = false;
    let mut downloading = 0u32;
    let mut any_paused = false;
    for row in mirror.values() {
        match row.status {
            Status::Failed => any_failed = true,
            Status::Active | Status::Muxing => downloading += 1,
            Status::Paused => any_paused = true,
            _ => {}
        }
    }
    if any_failed {
        TrayState::Error
    } else if downloading > 0 {
        TrayState::Downloading { n: downloading }
    } else if any_paused {
        TrayState::Paused
    } else {
        TrayState::Idle
    }
}

/// Aggregate progress across rows that are actively transferring. Used
/// only by the taskbar — the tooltip and popover live off the status
/// counts. Returns `(done, total)`. `total == 0` is the taskbar's "no
/// progress" sentinel (also returned when no row has a known size yet).
///
/// Only `Active` and `Muxing` rows contribute. A paused row's bytes
/// shouldn't drag the aggregate denominator while it's idle.
pub fn aggregate_progress(mirror: &HashMap<DownloadId, RowView>) -> (u64, u64) {
    let mut done = 0u64;
    let mut total = 0u64;
    let mut have_total = false;
    for row in mirror.values() {
        if !matches!(row.status, Status::Active | Status::Muxing) {
            continue;
        }
        done = done.saturating_add(row.downloaded);
        match row.total {
            Some(t) => {
                total = total.saturating_add(t);
                have_total = true;
            }
            None => {
                // An active row with unknown size — ITaskbarList3 has no
                // "indeterminate" state below `TBPF_INDETERMINATE` (which
                // we don't surface). Treat as no aggregate so we don't
                // show a misleading bar.
                return (0, 0);
            }
        }
    }
    if !have_total {
        (0, 0)
    } else {
        (done.min(total), total)
    }
}

#[derive(Clone)]
struct MenuItems {
    show: MenuItem<tauri::Wry>,
    pause_all: MenuItem<tauri::Wry>,
    resume_all: MenuItem<tauri::Wry>,
    add_url: MenuItem<tauri::Wry>,
    check_updates: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
}

/// Track the last applied state so we can avoid redundant icon swaps
/// (Win32's `SetIcon` is cheap but still worth skipping on every tick).
struct TrayInner {
    mirror: HashMap<DownloadId, RowView>,
    last_variant: Option<IconVariant>,
    last_state: Option<TrayState>,
    /// Last aggregate progress we pushed to the taskbar. Used to skip
    /// redundant `SetProgressValue` calls — at high tick rates these
    /// would flicker the taskbar bar on slow machines.
    last_taskbar: Option<(u64, u64)>,
}

impl TrayInner {
    fn new() -> Self {
        Self {
            mirror: HashMap::new(),
            last_variant: None,
            last_state: None,
            last_taskbar: None,
        }
    }
}

/// Spawn the tray icon, menu, and the background reducer task. Returns
/// after the icon has been added so callers can rely on it being
/// visible. Errors from `TrayIconBuilder::build` are surfaced (e.g.
/// missing dependency on Linux); the rest are non-fatal and logged.
pub fn install(app: &AppHandle, core: Core) -> tauri::Result<()> {
    let menu_items = MenuItems {
        show: MenuItem::with_id(app, "show", i18n::t("tray", "menuShow"), true, None::<&str>)?,
        pause_all: MenuItem::with_id(
            app,
            "pause_all",
            i18n::t("tray", "menuPauseAll"),
            false,
            None::<&str>,
        )?,
        resume_all: MenuItem::with_id(
            app,
            "resume_all",
            i18n::t("tray", "menuResumeAll"),
            false,
            None::<&str>,
        )?,
        add_url: MenuItem::with_id(
            app,
            "add_url",
            i18n::t("tray", "menuAddUrl"),
            true,
            None::<&str>,
        )?,
        check_updates: MenuItem::with_id(
            app,
            "check_updates",
            i18n::t("tray", "menuCheckUpdates"),
            true,
            None::<&str>,
        )?,
        quit: MenuItem::with_id(app, "quit", i18n::t("tray", "menuQuit"), true, None::<&str>)?,
    };
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &menu_items.show,
            &menu_items.pause_all,
            &menu_items.resume_all,
            &menu_items.add_url,
            &separator,
            &menu_items.check_updates,
            &menu_items.quit,
        ],
    )?;

    let initial_image = Image::from_bytes(ICON_IDLE)?;

    let tray = TrayIconBuilder::with_id(TRAY_ICON_ID)
        .icon(initial_image)
        .tooltip(TrayState::Idle.tooltip())
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event({
            let menu_app = app.clone();
            move |_tray_app, event: MenuEvent| {
                let id = event.id.as_ref().to_string();
                handle_menu_event(&menu_app, &id);
            }
        })
        .on_tray_icon_event({
            let click_app = app.clone();
            move |_tray, event: TrayIconEvent| {
                handle_tray_event(&click_app, event);
            }
        })
        .build(app)?;

    // `.build(app)` already registers the tray with Tauri's
    // `TrayIconManager`, so `app.tray_by_id(TRAY_ICON_ID)` resolves
    // from any thread for the lifetime of the app. The returned handle
    // can be dropped — Tauri keeps the icon alive internally.
    drop(tray);

    let inner = Arc::new(Mutex::new(TrayInner::new()));

    // Seed the mirror from the DB so a freshly-launched app with paused
    // downloads doesn't sit on `Idle` until the next event ticks.
    {
        let core = core.clone();
        let app_handle = app.clone();
        let menu_items = menu_items.clone();
        let inner = inner.clone();
        tauri::async_runtime::spawn(async move {
            if let Ok(rows) = core.list_downloads(DownloadFilter::default()).await {
                let mut guard = inner.lock().expect("tray inner poisoned");
                for row in rows {
                    guard.mirror.insert(
                        row.id,
                        RowView {
                            status: row.status,
                            downloaded: row.downloaded_bytes,
                            total: row.total_bytes,
                        },
                    );
                }
                let state = compute_state(&guard.mirror);
                drop(guard);
                // Initial seed reflects truth — quiet-hours suppression
                // is for *new* transitions, not the app's resting state.
                let _ = apply_state(&app_handle, &menu_items, &inner, state, false);
            }
        });
    }

    // Subscribe to the core event bus on a fresh receiver so
    // `forward_core_events`' loop is undisturbed. Updates the mirror,
    // recomputes the state, and applies any icon/menu/tooltip change.
    let mut rx = core.subscribe();
    let app_handle = app.clone();
    let inner_for_loop = inner.clone();
    let menu_items_for_loop = menu_items.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    // Language change: refresh i18n locale, re-label
                    // every menu item, and force a tooltip re-apply.
                    if let CoreEvent::SettingChanged { key } = &event {
                        if key == unduhin_core::settings_keys::LANGUAGE {
                            let raw = core
                                .get_setting(unduhin_core::settings_keys::LANGUAGE)
                                .await
                                .ok()
                                .flatten()
                                .and_then(|v| v.as_str().map(str::to_owned));
                            i18n::set_locale(i18n::Locale::from_setting(raw.as_deref()));
                            if let Err(e) = refresh_menu_labels(&menu_items_for_loop) {
                                tracing::warn!(error = %e, "failed to refresh tray menu labels");
                            }
                            // Re-apply the tooltip using the cached
                            // state so the new locale shows up
                            // without waiting for the next event.
                            let last = {
                                let guard = inner_for_loop.lock().expect("tray inner poisoned");
                                guard.last_state
                            };
                            if let Some(state) = last {
                                // Clear the cached state so apply_state
                                // doesn't early-return on equality.
                                inner_for_loop
                                    .lock()
                                    .expect("tray inner poisoned")
                                    .last_state = None;
                                let quiet = core.quiet_hours_state().await.active;
                                let _ = apply_state(
                                    &app_handle,
                                    &menu_items_for_loop,
                                    &inner_for_loop,
                                    state,
                                    quiet,
                                );
                            }
                            continue;
                        }
                    }

                    let state = {
                        let mut guard = inner_for_loop.lock().expect("tray inner poisoned");
                        apply_event(&mut guard.mirror, &event);
                        compute_state(&guard.mirror)
                    };
                    // Pull the gate fresh per event so a quiet-hours
                    // schedule change takes effect on the next tick
                    // without plumbing another signal.
                    let quiet = core.quiet_hours_state().await.active;
                    if let Err(e) = apply_state(
                        &app_handle,
                        &menu_items_for_loop,
                        &inner_for_loop,
                        state,
                        quiet,
                    ) {
                        tracing::warn!(error = %e, "failed to apply tray state");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("tray reducer lagged by {n} events");
                    // After lagging, the mirror is potentially stale.
                    // Re-seed from the DB to recover.
                    if let Ok(rows) = core.list_downloads(DownloadFilter::default()).await {
                        let state = {
                            let mut guard = inner_for_loop.lock().expect("tray inner poisoned");
                            guard.mirror.clear();
                            for row in rows {
                                guard.mirror.insert(
                                    row.id,
                                    RowView {
                                        status: row.status,
                                        downloaded: row.downloaded_bytes,
                                        total: row.total_bytes,
                                    },
                                );
                            }
                            compute_state(&guard.mirror)
                        };
                        // Recovery seed: same reasoning as the initial
                        // seed — show truth, the gate kicks in on the
                        // next genuine event.
                        let _ = apply_state(
                            &app_handle,
                            &menu_items_for_loop,
                            &inner_for_loop,
                            state,
                            false,
                        );
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    Ok(())
}

/// Apply a freshly-computed `TrayState` to the icon, tooltip, and menu
/// item enable bits. Skips redundant icon swaps so a `Downloading(3) →
/// Downloading(4)` transition doesn't flicker.
///
/// When `quiet_hours` is true, the icon and tooltip are intentionally
/// left frozen so a state change overnight doesn't visually "ping" the
/// user. Taskbar progress is still pushed — quiet hours mutes
/// *notifications*, not the truth about whether bytes are flowing.
fn apply_state(
    app: &AppHandle,
    items: &MenuItems,
    inner: &Arc<Mutex<TrayInner>>,
    state: TrayState,
    quiet_hours: bool,
) -> tauri::Result<()> {
    // Compute the taskbar aggregate up front so we can push it even when
    // `state` itself is unchanged (a `ProgressUpdate` mid-Downloading
    // doesn't move `TrayState` but should still tick the taskbar bar).
    // The taskbar is unaffected by quiet hours.
    let (taskbar_target, prev_taskbar) = {
        let guard = inner.lock().expect("tray inner poisoned");
        let target = match state {
            // Error / Paused / Idle: clear the bar entirely. The user
            // wants to see "demanding attention" or "doing nothing", not
            // a half-full progress mark.
            TrayState::Downloading { .. } => aggregate_progress(&guard.mirror),
            _ => (0, 0),
        };
        (target, guard.last_taskbar)
    };
    if prev_taskbar != Some(taskbar_target) {
        let (done, total) = taskbar_target;
        if total == 0 {
            taskbar::clear(app);
        } else {
            taskbar::set_progress(app, done, total);
        }
        inner.lock().expect("tray inner poisoned").last_taskbar = Some(taskbar_target);
    }

    // Quiet hours: refresh menu enable bits but leave the icon + tooltip
    // exactly as the user last saw them. Intentionally leaving
    // `last_variant` / `last_state` untouched so once quiet hours end,
    // the next event sees the staleness and applies the deferred change.
    if quiet_hours {
        let (any_pausable, any_resumable) = {
            let guard = inner.lock().expect("tray inner poisoned");
            let p = guard
                .mirror
                .values()
                .any(|r| matches!(r.status, Status::Active | Status::Muxing | Status::Queued));
            let r = guard
                .mirror
                .values()
                .any(|r| matches!(r.status, Status::Paused));
            (p, r)
        };
        items.pause_all.set_enabled(any_pausable)?;
        items.resume_all.set_enabled(any_resumable)?;
        return Ok(());
    }

    let (should_swap_icon, prev_state) = {
        let mut guard = inner.lock().expect("tray inner poisoned");
        let swap = guard.last_variant != Some(state.icon_variant());
        let prev = guard.last_state;
        guard.last_variant = Some(state.icon_variant());
        guard.last_state = Some(state);
        (swap, prev)
    };

    if prev_state == Some(state) {
        return Ok(());
    }

    let Some(tray) = app.tray_by_id(TRAY_ICON_ID) else {
        // Tray was dropped (app shutting down). Silently no-op.
        return Ok(());
    };

    if should_swap_icon {
        let image = Image::from_bytes(state.icon_variant().bytes())?;
        tray.set_icon(Some(image))?;
    }
    tray.set_tooltip(Some(state.tooltip()))?;

    // Refresh menu enable bits. `Pause all` / `Resume all` are only
    // meaningful when there's at least one matching row.
    let guard = inner.lock().expect("tray inner poisoned");
    let any_pausable = guard
        .mirror
        .values()
        .any(|r| matches!(r.status, Status::Active | Status::Muxing | Status::Queued));
    let any_resumable = guard
        .mirror
        .values()
        .any(|r| matches!(r.status, Status::Paused));
    drop(guard);

    items.pause_all.set_enabled(any_pausable)?;
    items.resume_all.set_enabled(any_resumable)?;

    Ok(())
}

/// Re-read the tray menu labels from the i18n module and apply them to
/// the existing `MenuItem` handles. Cheap — Tauri's `set_text` just
/// pushes the new label down to the OS without recreating the menu.
fn refresh_menu_labels(items: &MenuItems) -> tauri::Result<()> {
    items.show.set_text(i18n::t("tray", "menuShow"))?;
    items.pause_all.set_text(i18n::t("tray", "menuPauseAll"))?;
    items
        .resume_all
        .set_text(i18n::t("tray", "menuResumeAll"))?;
    items.add_url.set_text(i18n::t("tray", "menuAddUrl"))?;
    items
        .check_updates
        .set_text(i18n::t("tray", "menuCheckUpdates"))?;
    items.quit.set_text(i18n::t("tray", "menuQuit"))?;
    Ok(())
}

fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        "show" => show_main_window(app),
        "pause_all" => {
            let core = app.state::<Core>().inner().clone();
            tauri::async_runtime::spawn(async move {
                let _ = pause_all(&core).await;
            });
        }
        "resume_all" => {
            let core = app.state::<Core>().inner().clone();
            tauri::async_runtime::spawn(async move {
                let _ = resume_all(&core).await;
            });
        }
        "add_url" => {
            show_main_window(app);
            if let Err(e) = app.emit(OPEN_ADD_URL_EVENT, ()) {
                tracing::warn!(error = %e, "failed to emit open-add-url");
            }
        }
        "check_updates" => {
            show_main_window(app);
            if let Err(e) = app.emit(CHECK_UPDATES_EVENT, ()) {
                tracing::warn!(error = %e, "failed to emit check-updates");
            }
        }
        "quit" => {
            // Bypass close-behavior policy — the user explicitly asked.
            app.exit(0);
        }
        other => {
            tracing::warn!("unhandled tray menu item: {other}");
        }
    }
}

fn handle_tray_event(app: &AppHandle, event: TrayIconEvent) {
    // Left-click toggles the main window. Right-click is intentionally not
    // handled here: Tauri shows the native Win32 context menu (wired via
    // `.menu()` with `show_menu_on_left_click(false)`) on its own.
    // Previously we *also* surfaced the `always_on_top` popover on
    // right-click, which grabbed focus and covered/dismissed the native
    // menu — so right-click never yielded a usable menu. Letting the native
    // menu surface alone fixes that (and exposes the new "Check for
    // updates…" item).
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    } = event
    {
        toggle_main_window(app);
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

fn toggle_main_window(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    match win.is_visible() {
        Ok(true) => {
            // Already on screen — hide so the click feels like a toggle.
            // (Don't try to "raise to front" instead; that creates a
            // jarring focus steal when the user clicked the tray to
            // dismiss the window.)
            let _ = win.hide();
        }
        _ => {
            let _ = win.show();
            let _ = win.unminimize();
            let _ = win.set_focus();
        }
    }
}

async fn pause_all(core: &Core) -> usize {
    let rows = match core.list_downloads(DownloadFilter::default()).await {
        Ok(rows) => rows,
        Err(_) => return 0,
    };
    let mut n = 0;
    for r in rows {
        if matches!(r.status, Status::Queued | Status::Active) && core.pause(r.id).await.is_ok() {
            n += 1;
        }
    }
    n
}

async fn resume_all(core: &Core) -> usize {
    let rows = match core
        .list_downloads(DownloadFilter {
            status: Some(Status::Paused),
            category_id: None,
        })
        .await
    {
        Ok(rows) => rows,
        Err(_) => return 0,
    };
    let mut n = 0;
    for r in rows {
        if core.resume(r.id).await.is_ok() {
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use unduhin_core::{DownloadRecord, DownloadSource};

    fn rec(id: DownloadId, status: Status) -> DownloadRecord {
        rec_with(id, status, 0, None)
    }

    fn rec_with(
        id: DownloadId,
        status: Status,
        downloaded: u64,
        total: Option<u64>,
    ) -> DownloadRecord {
        DownloadRecord {
            id,
            url: "https://example.com".into(),
            filename: "f".into(),
            output_path: PathBuf::from("."),
            total_bytes: total,
            downloaded_bytes: downloaded,
            status,
            error: None,
            category_id: None,
            priority: 0,
            segments: 1,
            created_at: chrono::Utc::now(),
            completed_at: None,
            etag: None,
            last_modified: None,
            segments_meta: None,
            media_info: None,
            headers: None,
            source: DownloadSource::Manual,
            speed_samples: None,
        }
    }

    fn added(id: DownloadId, status: Status) -> CoreEvent {
        CoreEvent::DownloadAdded {
            id,
            snapshot: Box::new(rec(id, status)),
        }
    }

    fn changed(id: DownloadId, from: Status, to: Status) -> CoreEvent {
        CoreEvent::StatusChanged { id, from, to }
    }

    #[test]
    fn idle_when_empty() {
        let mirror = HashMap::new();
        assert_eq!(compute_state(&mirror), TrayState::Idle);
    }

    #[test]
    fn downloading_counts_active_and_muxing() {
        let mut mirror = HashMap::new();
        apply_event(&mut mirror, &added(1, Status::Active));
        apply_event(&mut mirror, &added(2, Status::Muxing));
        apply_event(&mut mirror, &added(3, Status::Queued));
        assert_eq!(compute_state(&mirror), TrayState::Downloading { n: 2 });
    }

    #[test]
    fn paused_state_only_when_no_active() {
        let mut mirror = HashMap::new();
        apply_event(&mut mirror, &added(1, Status::Paused));
        assert_eq!(compute_state(&mirror), TrayState::Paused);

        // Adding an active flips to Downloading.
        apply_event(&mut mirror, &added(2, Status::Active));
        assert_eq!(compute_state(&mirror), TrayState::Downloading { n: 1 });
    }

    #[test]
    fn error_beats_everything() {
        let mut mirror = HashMap::new();
        apply_event(&mut mirror, &added(1, Status::Active));
        apply_event(&mut mirror, &added(2, Status::Paused));
        apply_event(&mut mirror, &added(3, Status::Failed));
        assert_eq!(compute_state(&mirror), TrayState::Error);
    }

    #[test]
    fn error_clears_when_failed_row_removed() {
        let mut mirror = HashMap::new();
        apply_event(&mut mirror, &added(1, Status::Failed));
        assert_eq!(compute_state(&mirror), TrayState::Error);

        apply_event(&mut mirror, &CoreEvent::Removed { id: 1 });
        assert_eq!(compute_state(&mirror), TrayState::Idle);
    }

    #[test]
    fn error_clears_when_failed_row_retried() {
        let mut mirror = HashMap::new();
        apply_event(&mut mirror, &added(1, Status::Failed));
        // Retry path emits StatusChanged(Failed -> Queued).
        apply_event(&mut mirror, &changed(1, Status::Failed, Status::Queued));
        assert_eq!(compute_state(&mirror), TrayState::Idle);
    }

    #[test]
    fn completed_event_marks_completed() {
        let mut mirror = HashMap::new();
        apply_event(&mut mirror, &added(1, Status::Active));
        apply_event(&mut mirror, &CoreEvent::Completed { id: 1, bytes: 0 });
        assert_eq!(compute_state(&mirror), TrayState::Idle);
    }

    #[test]
    fn icon_variant_collapses_count_changes() {
        assert_eq!(
            TrayState::Downloading { n: 3 }.icon_variant(),
            TrayState::Downloading { n: 4 }.icon_variant(),
        );
    }

    #[test]
    fn aggregate_progress_sums_active_rows_only() {
        let mut mirror = HashMap::new();
        apply_event(
            &mut mirror,
            &CoreEvent::DownloadAdded {
                id: 1,
                snapshot: Box::new(rec_with(1, Status::Active, 300, Some(1000))),
            },
        );
        apply_event(
            &mut mirror,
            &CoreEvent::DownloadAdded {
                id: 2,
                snapshot: Box::new(rec_with(2, Status::Muxing, 500, Some(2000))),
            },
        );
        // Paused row must not contribute.
        apply_event(
            &mut mirror,
            &CoreEvent::DownloadAdded {
                id: 3,
                snapshot: Box::new(rec_with(3, Status::Paused, 9_000, Some(10_000))),
            },
        );
        assert_eq!(aggregate_progress(&mirror), (800, 3000));
    }

    #[test]
    fn aggregate_progress_returns_zero_when_total_unknown() {
        let mut mirror = HashMap::new();
        apply_event(
            &mut mirror,
            &CoreEvent::DownloadAdded {
                id: 1,
                snapshot: Box::new(rec_with(1, Status::Active, 300, None)),
            },
        );
        assert_eq!(aggregate_progress(&mirror), (0, 0));
    }

    #[test]
    fn progress_update_refreshes_bytes_but_not_status() {
        let mut mirror = HashMap::new();
        apply_event(
            &mut mirror,
            &CoreEvent::DownloadAdded {
                id: 1,
                snapshot: Box::new(rec_with(1, Status::Active, 0, Some(1000))),
            },
        );
        apply_event(
            &mut mirror,
            &CoreEvent::ProgressUpdate {
                id: 1,
                downloaded: 450,
                total: Some(1000),
                speed_bps: 100_000.0,
                eta: None,
            },
        );
        let row = mirror.get(&1).copied().unwrap();
        assert_eq!(row.status, Status::Active);
        assert_eq!(row.downloaded, 450);
        assert_eq!(row.total, Some(1000));
        assert_eq!(aggregate_progress(&mirror), (450, 1000));
    }
}
