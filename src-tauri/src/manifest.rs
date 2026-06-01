//! Reconcile the native-host manifest's `"path"` field with
//! the current install location.
//!
//! The committed template at `src-tauri/native-host/com.unduhin.host.json`
//! ships with `"path": "PLACEHOLDER_ABS_PATH"`. The NSIS hook
//! (`src-tauri/nsis-hooks/hooks.nsi`) rewrites that placeholder to the
//! real install path at install time, which covers users who ran the
//! installer.
//!
//! Two cases the NSIS hook can't help with:
//! - `cargo tauri dev` / `cargo tauri build --debug` invocations, where
//!   the manifest sits in `target/{profile}/native-host/` and never
//!   passes through the installer.
//! - A user who moved `$INSTDIR` after install (rare but possible) —
//!   the registry key still points to the manifest but `"path"` inside
//!   it no longer matches reality.
//!
//! [`reconcile_native_host_manifest`] handles both. It is idempotent
//! and any I/O failure is warn-logged rather than raised — a borked
//! manifest is a tray-notification-worthy issue, not a fatal one.

#[cfg(windows)]
pub fn reconcile_native_host_manifest(_app: &tauri::AppHandle) -> anyhow::Result<()> {
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    let exe = env::current_exe()?;
    let install_dir = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("current_exe has no parent"))?
        .to_path_buf();

    // The manifest is staged into `<install_dir>/native-host/` for
    // installed builds. Tauri v2's resource resolver may also route it
    // through `<install_dir>/resources/native-host/` depending on
    // bundler version — check both and short-circuit on the first that
    // exists.
    let candidates = [
        install_dir
            .join("native-host")
            .join("com.unduhin.host.json"),
        install_dir
            .join("resources")
            .join("native-host")
            .join("com.unduhin.host.json"),
    ];

    let manifest_path: PathBuf = match candidates.iter().find(|p| p.exists()) {
        Some(p) => p.clone(),
        None => {
            tracing::debug!(
                "native-host manifest not found in any of the expected install locations; \
                 skipping reconcile (dev shell or stripped build?)"
            );
            return Ok(());
        }
    };

    let host_exe = match manifest_path
        .parent()
        .map(|p| p.join("unduhin-native-host.exe"))
    {
        Some(path) => path,
        None => return Ok(()),
    };

    let contents = fs::read_to_string(&manifest_path)?;
    if !contents.contains("PLACEHOLDER_ABS_PATH") {
        // Already rewritten by the NSIS hook (or a previous run of this
        // function). Nothing to do.
        return Ok(());
    }

    // JSON requires backslashes to be escaped.
    let escaped = host_exe.display().to_string().replace('\\', "\\\\");
    let rewritten = contents.replace("PLACEHOLDER_ABS_PATH", &escaped);

    fs::write(&manifest_path, rewritten)?;
    tracing::info!(
        manifest = %manifest_path.display(),
        host = %host_exe.display(),
        "rewrote native-host manifest path"
    );
    Ok(())
}

#[cfg(not(windows))]
pub fn reconcile_native_host_manifest(_app: &tauri::AppHandle) -> anyhow::Result<()> {
    // Non-Windows Tauri builds compile but the native host integration
    // is Windows-only this release.
    Ok(())
}
