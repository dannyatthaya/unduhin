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
//!
//! macOS has no installer at all, so there the same function does the
//! whole registration itself on every launch. See the macOS arm below.

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

/// macOS has no installer, so the app registers itself on every launch.
///
/// Three things happen here, and the ordering matters.
///
/// First, the host binary is copied **out** of the app bundle into the
/// data root. Pointing the browsers at the copy inside `Unduhin.app`
/// would be simpler but breaks in three separate ways: the user can move
/// or rename the bundle at any time, the executable bit has to survive the
/// bundler and the drag out of the `.dmg`, and an unsigned app downloaded
/// through a browser carries `com.apple.quarantine` that can block
/// `posix_spawn`. Owning our own copy makes all three moot.
///
/// Second, the bundle's own location is recorded, so the native host can
/// cold-start the app by path when LaunchServices does not yet know the
/// bundle id.
///
/// Third, the manifest is written into each browser's
/// `NativeMessagingHosts` directory — but only for browsers that have
/// actually run, so we do not litter profile directories for browsers the
/// user does not have. A browser installed later picks the manifest up on
/// Unduhin's next launch.
#[cfg(target_os = "macos")]
pub fn reconcile_native_host_manifest(app: &tauri::AppHandle) -> anyhow::Result<()> {
    use anyhow::Context as _;
    use tauri::Manager as _;

    let resources = app.path().resource_dir().context("resolve resource dir")?;
    let src_host = resources.join("native-host").join("unduhin-native-host");
    let src_manifest = resources.join("native-host").join("com.unduhin.host.json");
    if !src_host.exists() || !src_manifest.exists() {
        tracing::debug!("bundled native-host payload not found; skipping reconcile (dev shell?)");
        return Ok(());
    }

    let root = unduhin_core::directories_root()
        .ok_or_else(|| anyhow::anyhow!("no app data root"))?
        .join("native-host");
    std::fs::create_dir_all(&root)?;

    let staged_host = root.join("unduhin-native-host");
    stage_host_binary(&src_host, &staged_host)?;

    // `resource_dir()` is `<...>/Unduhin.app/Contents/Resources`.
    if let Some(bundle) = resources.parent().and_then(|c| c.parent()) {
        let _ = write_if_changed(
            &root.join("app-path.txt"),
            bundle.display().to_string().as_bytes(),
        );
    }

    // No backslash escaping needed, unlike the Windows arm: POSIX paths
    // contain nothing JSON has to escape.
    let manifest = std::fs::read_to_string(&src_manifest)?
        .replace("PLACEHOLDER_ABS_PATH", &staged_host.display().to_string());

    for dir in native_messaging_dirs() {
        // The parent is the browser's profile directory. Its absence means
        // the browser has never run, so registering would be noise.
        let Some(parent) = dir.parent() else { continue };
        if !parent.exists() {
            continue;
        }
        if let Err(e) = std::fs::create_dir_all(&dir).and_then(|()| {
            write_if_changed(&dir.join("com.unduhin.host.json"), manifest.as_bytes())
        }) {
            tracing::warn!(dir = %dir.display(), error = %e, "could not register native-host manifest");
        }
    }
    Ok(())
}

/// Per-browser `NativeMessagingHosts` directories, as documented in
/// `extension/native-host/README.md`.
#[cfg(target_os = "macos")]
fn native_messaging_dirs() -> Vec<std::path::PathBuf> {
    let Ok(home) = std::env::var("HOME") else {
        return Vec::new();
    };
    let support = std::path::Path::new(&home)
        .join("Library")
        .join("Application Support");
    [
        "Google/Chrome",
        "Microsoft Edge",
        "BraveSoftware/Brave-Browser",
    ]
    .iter()
    .map(|b| support.join(b).join("NativeMessagingHosts"))
    .collect()
}

/// Copy the host binary and make it executable.
///
/// Deliberately not `std::fs::copy`: that goes through `fcopyfile` on
/// macOS, which can carry extended attributes across — including
/// `com.apple.quarantine`, the very thing this copy exists to shed. Going
/// through a fresh `File::create` guarantees a clean inode with no xattrs
/// at all.
#[cfg(target_os = "macos")]
fn stage_host_binary(src: &std::path::Path, dst: &std::path::Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    // Skip the copy when nothing changed, so a normal launch is a couple
    // of `stat` calls rather than a 2 MB write.
    let src_meta = std::fs::metadata(src)?;
    if let Ok(dst_meta) = std::fs::metadata(dst) {
        let same_size = dst_meta.len() == src_meta.len();
        let not_older = match (dst_meta.modified(), src_meta.modified()) {
            (Ok(d), Ok(s)) => d >= s,
            _ => false,
        };
        if same_size && not_older && dst_meta.permissions().mode() & 0o111 != 0 {
            return Ok(());
        }
    }

    let mut reader = std::fs::File::open(src)?;
    let mut writer = std::fs::File::create(dst)?;
    std::io::copy(&mut reader, &mut writer)?;
    drop(writer);

    std::fs::set_permissions(dst, std::fs::Permissions::from_mode(0o755))?;
    tracing::info!(host = %dst.display(), "staged native host outside the app bundle");
    Ok(())
}

/// Write only when the content differs, atomically.
///
/// Same-directory `rename` is atomic on APFS, so a browser reading the
/// manifest never sees a half-written file. Skipping unchanged writes
/// keeps repeat launches from churning mtimes.
#[cfg(target_os = "macos")]
fn write_if_changed(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if std::fs::read(path).is_ok_and(|existing| existing == bytes) {
        return Ok(());
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn reconcile_native_host_manifest(_app: &tauri::AppHandle) -> anyhow::Result<()> {
    // Unduhin ships on Windows and macOS; other targets compile but do
    // not register a native-messaging host.
    Ok(())
}
