//! Window lifecycle policy: `WindowEvent::CloseRequested` honors the
//! persisted `close_behavior` / `confirm_on_quit` settings. The
//! `ask`-path bridges to the frontend's styled `Dialog.vue` via the
//! [`ConfirmOnQuitBridge`] request/response channel.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;
use unduhin_core::{settings_keys, Core, DownloadFilter, Status};

/// Tauri event tag carrying a single quit-confirmation prompt to the
/// frontend. The payload is shaped by [`ConfirmQuitRequest`].
const CONFIRM_QUIT_EVENT: &str = "unduhin:confirm-quit";

/// How long the Rust side waits for the frontend's response before
/// treating the close as cancelled. Long enough to read a dialog;
/// short enough that a hung renderer doesn't leak a oneshot forever.
const CONFIRM_QUIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Shared state for the Rust → Vue confirm-on-quit handshake. Allocates a
/// monotonic `request_id`, parks a `oneshot::Sender<bool>` keyed by that
/// id, and exposes the response API the Tauri command uses to deliver
/// the user's choice.
#[derive(Default)]
pub struct ConfirmOnQuitBridge {
    pending: Mutex<HashMap<u32, oneshot::Sender<bool>>>,
    next_id: AtomicU32,
}

impl ConfirmOnQuitBridge {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserve a new request slot and return its id alongside the
    /// matching receiver. Caller `await`s the receiver after emitting
    /// the corresponding `unduhin:confirm-quit` event.
    fn open(&self) -> (u32, oneshot::Receiver<bool>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("ConfirmOnQuitBridge poisoned")
            .insert(id, tx);
        (id, rx)
    }

    /// Deliver the user's choice. Returns `true` when the request id was
    /// known (and the answer was forwarded), `false` when it had already
    /// timed out or been answered.
    pub fn respond(&self, request_id: u32, allow: bool) -> bool {
        let sender = self
            .pending
            .lock()
            .expect("ConfirmOnQuitBridge poisoned")
            .remove(&request_id);
        match sender {
            Some(tx) => tx.send(allow).is_ok(),
            None => false,
        }
    }
}

#[derive(Serialize, Clone)]
struct ConfirmQuitRequest {
    request_id: u32,
    active_count: u32,
    /// `true` when the prompt was triggered by `close_behavior = "ask"`
    /// (close-the-window framing). `false` when triggered by
    /// `confirm_on_quit` with `close_behavior = "exit"` (quit-with-active
    /// framing). The frontend uses this to pick wording.
    ask_close: bool,
}

/// Outcome of the close-handler decision, surfaced from
/// [`decide_close`] so the call site can drive the UI thread without
/// holding `prevent_close()` open across an `.await`.
enum CloseDecision {
    /// Settings say `minimize` — `window.hide()` and let the app keep
    /// running.
    Minimize,
    /// `exit` + no confirm needed — call `app.exit(0)`.
    Exit,
    /// `ask` or `exit + confirm_on_quit + has-inflight` — fire the
    /// confirm-quit handshake and act on the answer.
    Ask { active_count: u32, ask_close: bool },
}

/// Handle a single `CloseRequested` on the main window. This runs on the
/// Tauri async runtime *after* the caller has already invoked
/// `event.api().prevent_close()`. The function decides what to do and
/// applies the action itself.
pub async fn handle_close_requested(app: AppHandle, core: Core) {
    let decision = decide_close(&core).await;

    match decision {
        CloseDecision::Minimize => {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.hide();
            }
        }
        CloseDecision::Exit => {
            app.exit(0);
        }
        CloseDecision::Ask {
            active_count,
            ask_close,
        } => {
            let bridge = app.state::<ConfirmOnQuitBridge>();
            let (request_id, rx) = bridge.open();

            let payload = ConfirmQuitRequest {
                request_id,
                active_count,
                ask_close,
            };
            if let Err(e) = app.emit(CONFIRM_QUIT_EVENT, &payload) {
                tracing::warn!(error = %e, "failed to emit confirm-quit; cancelling close");
                bridge.respond(request_id, false);
                return;
            }

            let answer = match tokio::time::timeout(CONFIRM_QUIT_TIMEOUT, rx).await {
                Ok(Ok(answer)) => answer,
                Ok(Err(_)) => {
                    // Sender dropped without sending. Treat as cancel.
                    false
                }
                Err(_) => {
                    tracing::warn!("confirm-quit timed out after {:?}", CONFIRM_QUIT_TIMEOUT);
                    // Take the sender out so a late `respond` no-ops.
                    bridge.respond(request_id, false);
                    false
                }
            };

            if answer {
                app.exit(0);
            }
        }
    }
}

async fn decide_close(core: &Core) -> CloseDecision {
    let behavior = core
        .get_setting(settings_keys::CLOSE_BEHAVIOR)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "ask".into());

    if behavior == "minimize" {
        return CloseDecision::Minimize;
    }

    let inflight = inflight_count(core).await;

    if behavior == "ask" {
        return CloseDecision::Ask {
            active_count: inflight,
            ask_close: true,
        };
    }

    // behavior == "exit"
    let confirm = core
        .get_setting(settings_keys::CONFIRM_ON_QUIT)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    if confirm && inflight > 0 {
        CloseDecision::Ask {
            active_count: inflight,
            ask_close: false,
        }
    } else {
        CloseDecision::Exit
    }
}

async fn inflight_count(core: &Core) -> u32 {
    match core.list_downloads(DownloadFilter::default()).await {
        Ok(rows) => rows
            .into_iter()
            .filter(|r| matches!(r.status, Status::Active | Status::Muxing | Status::Queued))
            .count() as u32,
        Err(_) => 0,
    }
}

/// Label of the borderless tray popover window. Lazily-spawned by the
/// tray reducer; surfaced + positioned via [`show_popover_at`].
pub const TRAY_POPOVER_LABEL: &str = "tray-popover";

/// Popover dimensions. Used both at spawn time and to compute the
/// position offset from the tray icon's click coordinates.
pub const TRAY_POPOVER_WIDTH: f64 = 280.0;
pub const TRAY_POPOVER_HEIGHT: f64 = 220.0;

/// Build the popover window hidden during app startup so the first
/// surface is instant on tray click. Idempotent — if the window already
/// exists (e.g. the user toggled fast) this is a no-op.
pub fn ensure_popover_window(app: &AppHandle) -> tauri::Result<()> {
    if app.get_webview_window(TRAY_POPOVER_LABEL).is_some() {
        return Ok(());
    }
    let _window = tauri::WebviewWindowBuilder::new(
        app,
        TRAY_POPOVER_LABEL,
        tauri::WebviewUrl::App("tray-popover.html".into()),
    )
    .title("Unduhin")
    .inner_size(TRAY_POPOVER_WIDTH, TRAY_POPOVER_HEIGHT)
    .resizable(false)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .visible(false)
    .shadow(true)
    .transparent(false)
    .build()?;
    Ok(())
}

/// Position the popover above the tray icon and show it. `tray_x` /
/// `tray_y` come from `TrayIconEvent`'s physical position. The popover
/// is centered horizontally on the icon and anchored 10px above its
/// top edge, clamped to the same monitor's work area so it never
/// overflows the screen.
///
/// Currently unused: the tray right-click now surfaces the native Win32
/// menu instead of this popover (the `always_on_top` popover used to grab
/// focus and cover the menu). Retained so the popover can be re-surfaced
/// from a different trigger later without rebuilding the positioning math.
#[allow(dead_code)]
pub fn show_popover_at(app: &AppHandle, tray_x: f64, tray_y: f64) {
    if ensure_popover_window(app).is_err() {
        return;
    }
    let Some(win) = app.get_webview_window(TRAY_POPOVER_LABEL) else {
        return;
    };

    let scale_factor = win.scale_factor().ok().filter(|f| *f > 0.0).unwrap_or(1.0);
    let width_px = TRAY_POPOVER_WIDTH * scale_factor;
    let height_px = TRAY_POPOVER_HEIGHT * scale_factor;

    let mut x = tray_x - width_px / 2.0;
    let mut y = tray_y - height_px - 10.0;

    if let Ok(Some(monitor)) = win.current_monitor() {
        let mpos = monitor.position();
        let msize = monitor.size();
        let min_x = mpos.x as f64;
        let max_x = (mpos.x + msize.width as i32) as f64 - width_px;
        let min_y = mpos.y as f64;
        let max_y = (mpos.y + msize.height as i32) as f64 - height_px;
        if x < min_x {
            x = min_x;
        }
        if x > max_x {
            x = max_x;
        }
        if y < min_y {
            y = min_y;
        }
        if y > max_y {
            y = max_y;
        }
    }

    let _ = win.set_position(tauri::PhysicalPosition::new(x as i32, y as i32));
    let _ = win.show();
    let _ = win.set_focus();
}

/// Hide the popover without destroying it, so the next surface stays
/// instant. Safe to call from any tray-reducer path.
pub fn hide_popover(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(TRAY_POPOVER_LABEL) {
        let _ = win.hide();
    }
}

/// Read `start_minimized` and surface the main window if the setting is
/// off (or absent). Runs on app startup after the window has been built
/// with `visible: false`.
pub async fn apply_start_minimized(app: AppHandle, core: Core) {
    let minimized = core
        .get_setting(settings_keys::START_MINIMIZED)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if minimized {
        return;
    }
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}
