//! Windows taskbar-icon progress indicator (ITaskbarList3).
//!
//! Wraps the COM dance behind two cheap calls: [`set_progress`] and
//! [`clear`]. Both are routed through the Tauri main thread because
//! `ITaskbarList3` requires the COM apartment that owns the HWND. The
//! single-threaded apartment is initialised lazily on the main thread the
//! first time a call lands, and the resulting interface pointer is cached
//! in a `thread_local!` for the lifetime of the app.
//!
//! Non-Windows builds compile this module as a no-op shim so the rest of
//! the crate (the tray reducer) doesn't need `cfg` gates at every call
//! site.

use tauri::AppHandle;

/// Reflect aggregate progress on the main window's taskbar icon.
///
/// - `total == 0` is treated as "no active downloads" and clears the bar.
/// - `done > total` is clamped to `total` so a sloppy aggregate (which
///   the engine never produces, but defensive code is cheap) doesn't make
///   ITaskbarList3 reject the call.
pub fn set_progress(app: &AppHandle, done: u64, total: u64) {
    #[cfg(target_os = "windows")]
    {
        let app = app.clone();
        let _ = app.clone().run_on_main_thread(move || {
            imp::set_progress_on_main(&app, done, total);
        });
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, done, total);
    }
}

/// Force-clear the taskbar progress indicator. Cheap if nothing is set.
pub fn clear(app: &AppHandle) {
    #[cfg(target_os = "windows")]
    {
        let app = app.clone();
        let _ = app.clone().run_on_main_thread(move || {
            imp::clear_on_main(&app);
        });
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::cell::RefCell;

    use tauri::{AppHandle, Manager};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList, TBPF_NOPROGRESS, TBPF_NORMAL};

    thread_local! {
        /// Cached interface pointer for the calling thread. `None` until
        /// the first successful CoCreateInstance; `Some(Err(()))` after a
        /// permanent failure so we don't retry forever.
        static TASKBAR: RefCell<Option<Result<ITaskbarList3, ()>>> = const { RefCell::new(None) };
    }

    /// Initialise COM as STA (idempotent — `CoInitializeEx` returns
    /// `RPC_E_CHANGED_MODE` if the apartment was already set to another
    /// mode, and `S_FALSE` if it's already the same one — both are
    /// non-fatal for our purposes). Then create the `ITaskbarList3` and
    /// call its required `HrInit`.
    fn create_taskbar() -> Result<ITaskbarList3, ()> {
        unsafe {
            // SAFETY: COM init on the main thread is idempotent; the
            // duplicate-init HRESULTs (`S_FALSE`, `RPC_E_CHANGED_MODE`)
            // are tolerated by ignoring the result. ITaskbarList3's
            // contract requires HrInit() before any other call.
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let tb: ITaskbarList3 = CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| {
                    tracing::warn!(error = %e, "taskbar: CoCreateInstance(TaskbarList) failed");
                })?;
            tb.HrInit().map_err(|e| {
                tracing::warn!(error = %e, "taskbar: ITaskbarList::HrInit failed");
            })?;
            Ok(tb)
        }
    }

    fn with_taskbar<F: FnOnce(&ITaskbarList3)>(f: F) {
        TASKBAR.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                *slot = Some(create_taskbar());
            }
            if let Some(Ok(tb)) = slot.as_ref() {
                f(tb);
            }
        });
    }

    /// Resolve the main window's HWND. `None` when the window is gone
    /// (app shutting down) or when `hwnd()` returns an error.
    fn main_hwnd(app: &AppHandle) -> Option<HWND> {
        let win = app.get_webview_window("main")?;
        win.hwnd().ok()
    }

    pub(super) fn set_progress_on_main(app: &AppHandle, done: u64, total: u64) {
        let Some(hwnd) = main_hwnd(app) else { return };
        if total == 0 {
            with_taskbar(|tb| unsafe {
                let _ = tb.SetProgressState(hwnd, TBPF_NOPROGRESS);
            });
            return;
        }
        let done = done.min(total);
        with_taskbar(|tb| unsafe {
            let _ = tb.SetProgressState(hwnd, TBPF_NORMAL);
            let _ = tb.SetProgressValue(hwnd, done, total);
        });
    }

    pub(super) fn clear_on_main(app: &AppHandle) {
        let Some(hwnd) = main_hwnd(app) else { return };
        with_taskbar(|tb| unsafe {
            let _ = tb.SetProgressState(hwnd, TBPF_NOPROGRESS);
        });
    }
}
