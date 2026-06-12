//! Sync the bundled browser extension into the app-managed canonical folder.
//!
//! The installer stages the built extension under `<install_dir>/extension/`
//! (a `bundle.resources` entry, same mechanism as `native-host/`). Users
//! Load-unpacked the *canonical* copy at `%LOCALAPPDATA%\unduhin\extension`
//! (the same data root as the db/logs/binaries) exactly once; every app
//! launch afterwards calls [`sync`], which refreshes that folder whenever
//! the bundled version differs and reports the new version so the pipe
//! server can tell a running extension to `chrome.runtime.reload()`.
//!
//! Chrome may have the canonical folder loaded while we replace it, so the
//! swap is staged: copy the bundle to a `extension.staging` sibling, then a
//! two-step rename (`extension` → `extension.old`, staging → `extension`).
//! If the rename loses to an open handle, fall back to copying over in
//! place with `manifest.json` written **last** — a half-written tree must
//! never carry the new version marker, because the version is what the
//! extension uses to decide it is safe to reload.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;

/// Subdirectory of the unduhin data root users Load-unpacked from.
const CANONICAL_SUBDIR: &str = "extension";

/// The canonical unpacked-extension folder, created on demand so the
/// Settings → Browser "Open folder" affordance never opens a missing path.
/// Rooted in [`unduhin_core::directories_root`] — the same
/// `%LOCALAPPDATA%\unduhin\` the db, logs, and binaries live in.
pub fn canonical_dir() -> anyhow::Result<PathBuf> {
    let dir = unduhin_core::directories_root()
        .context("no local data root available")?
        .join(CANONICAL_SUBDIR);
    fs::create_dir_all(&dir)
        .with_context(|| format!("create canonical extension dir {}", dir.display()))?;
    Ok(dir)
}

/// Locate the extension payload staged by the installer (or by
/// `cargo tauri dev`, which copies `bundle.resources` next to the dev exe).
/// Mirrors the candidate probing in [`crate::manifest`].
fn bundled_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let install_dir = exe.parent()?;
    [
        install_dir.join("extension"),
        install_dir.join("resources").join("extension"),
    ]
    .into_iter()
    .find(|p| p.join("manifest.json").is_file())
}

/// Read the `version` field of a Chrome extension `manifest.json` under
/// `dir`. `None` for a missing or unparseable manifest — callers treat
/// that as "no extension staged here".
pub fn staged_version(dir: &Path) -> Option<String> {
    let raw = fs::read_to_string(dir.join("manifest.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    json.get("version")?.as_str().map(str::to_owned)
}

/// Bring the canonical folder up to the bundled version. Returns
/// `Some(version)` when the folder contents were replaced, `None` when the
/// versions already matched (or no bundle is staged — dev shells that
/// never built the extension).
pub fn sync() -> anyhow::Result<Option<String>> {
    let Some(bundled) = bundled_dir() else {
        tracing::debug!("no bundled extension found next to the exe; skipping sync");
        return Ok(None);
    };
    let canonical = canonical_dir()?;
    sync_dirs(&bundled, &canonical)
}

/// Testable core of [`sync`]: compare versions, stage, swap.
fn sync_dirs(bundled: &Path, canonical: &Path) -> anyhow::Result<Option<String>> {
    let bundled_version = staged_version(bundled).with_context(|| {
        format!(
            "bundled extension at {} has no readable manifest version",
            bundled.display()
        )
    })?;
    if staged_version(canonical).as_deref() == Some(bundled_version.as_str()) {
        tracing::debug!(version = %bundled_version, "canonical extension already current");
        return Ok(None);
    }

    let parent = canonical
        .parent()
        .context("canonical extension dir has no parent")?;
    let staging = parent.join("extension.staging");
    let old = parent.join("extension.old");

    // Fresh staging copy every run; a leftover from a crashed sync is stale.
    if staging.exists() {
        fs::remove_dir_all(&staging).context("remove stale staging dir")?;
    }
    copy_dir_recursive(bundled, &staging).context("stage bundled extension")?;
    // Best-effort: a lingering `.old` only blocks the rename swap below.
    if old.exists() {
        let _ = fs::remove_dir_all(&old);
    }

    let canonical_populated = canonical.join("manifest.json").exists()
        || fs::read_dir(canonical)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false);
    if !canonical_populated {
        // Nothing to displace — `canonical_dir` pre-creates the (empty)
        // folder, so copy into it rather than renaming over it.
        copy_over(&staging, canonical)?;
        let _ = fs::remove_dir_all(&staging);
        tracing::info!(version = %bundled_version, dir = %canonical.display(), "staged extension into fresh canonical dir");
        return Ok(Some(bundled_version));
    }

    match fs::rename(canonical, &old) {
        Ok(()) => {
            if let Err(e) = fs::rename(&staging, canonical) {
                // The canonical name is momentarily free; put the previous
                // tree back before falling back, so Chrome never observes
                // an empty folder.
                tracing::warn!(error = %e, "staging→canonical rename failed; restoring previous tree");
                let _ = fs::rename(&old, canonical);
                copy_over(&staging, canonical)?;
                let _ = fs::remove_dir_all(&staging);
            } else {
                let _ = fs::remove_dir_all(&old);
            }
        }
        Err(e) => {
            // Open handle (Chrome, Explorer) — copy over in place.
            tracing::debug!(error = %e, "canonical rename blocked; copying over in place");
            copy_over(&staging, canonical)?;
            let _ = fs::remove_dir_all(&staging);
        }
    }
    tracing::info!(version = %bundled_version, dir = %canonical.display(), "canonical extension updated");
    Ok(Some(bundled_version))
}

/// Copy `src` into `dst` file by file, writing the top-level
/// `manifest.json` last so a partially copied tree never advertises the
/// new version. Files that no longer exist in `src` are left behind —
/// only the rename swap removes them, and stale chunks are inert because
/// `manifest.json` no longer references them.
fn copy_over(src: &Path, dst: &Path) -> anyhow::Result<()> {
    copy_dir_recursive_inner(src, dst, true)?;
    let manifest_src = src.join("manifest.json");
    if manifest_src.is_file() {
        fs::copy(&manifest_src, dst.join("manifest.json")).context("copy manifest.json last")?;
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    copy_dir_recursive_inner(src, dst, false)
}

fn copy_dir_recursive_inner(src: &Path, dst: &Path, skip_root_manifest: bool) -> anyhow::Result<()> {
    fs::create_dir_all(dst).with_context(|| format!("create dir {}", dst.display()))?;
    for entry in fs::read_dir(src).with_context(|| format!("read dir {}", src.display()))? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive_inner(&from, &to, false)?;
        } else {
            if skip_root_manifest && entry.file_name() == "manifest.json" {
                continue;
            }
            fs::copy(&from, &to)
                .with_context(|| format!("copy {} → {}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_ext(dir: &Path, version: &str, extra: &[(&str, &str)]) {
        fs::create_dir_all(dir.join("background")).unwrap();
        fs::write(
            dir.join("manifest.json"),
            format!(r#"{{"manifest_version":3,"name":"Unduhin","version":"{version}"}}"#),
        )
        .unwrap();
        fs::write(dir.join("background").join("service-worker.js"), "// sw").unwrap();
        for (name, body) in extra {
            fs::write(dir.join(name), body).unwrap();
        }
    }

    #[test]
    fn fresh_canonical_gets_populated() {
        let tmp = tempfile::tempdir().unwrap();
        let bundled = tmp.path().join("bundle");
        let canonical = tmp.path().join("extension");
        write_ext(&bundled, "1.2.0", &[]);
        fs::create_dir_all(&canonical).unwrap();

        let synced = sync_dirs(&bundled, &canonical).unwrap();
        assert_eq!(synced.as_deref(), Some("1.2.0"));
        assert_eq!(staged_version(&canonical).as_deref(), Some("1.2.0"));
        assert!(canonical.join("background/service-worker.js").is_file());
        assert!(!tmp.path().join("extension.staging").exists());
    }

    #[test]
    fn same_version_is_a_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let bundled = tmp.path().join("bundle");
        let canonical = tmp.path().join("extension");
        write_ext(&bundled, "1.2.0", &[]);
        write_ext(&canonical, "1.2.0", &[("user-marker.txt", "untouched")]);

        let synced = sync_dirs(&bundled, &canonical).unwrap();
        assert_eq!(synced, None);
        // No-op must not touch the tree at all.
        assert!(canonical.join("user-marker.txt").is_file());
    }

    #[test]
    fn upgrade_replaces_tree_and_drops_removed_files() {
        let tmp = tempfile::tempdir().unwrap();
        let bundled = tmp.path().join("bundle");
        let canonical = tmp.path().join("extension");
        write_ext(&bundled, "1.3.0", &[("new-chunk.js", "// new")]);
        write_ext(&canonical, "1.2.0", &[("old-chunk.js", "// old")]);

        let synced = sync_dirs(&bundled, &canonical).unwrap();
        assert_eq!(synced.as_deref(), Some("1.3.0"));
        assert_eq!(staged_version(&canonical).as_deref(), Some("1.3.0"));
        assert!(canonical.join("new-chunk.js").is_file());
        // Rename-swap path: files removed upstream disappear.
        assert!(!canonical.join("old-chunk.js").exists());
        assert!(!tmp.path().join("extension.old").exists());
        assert!(!tmp.path().join("extension.staging").exists());
    }

    #[test]
    fn copy_over_writes_manifest_last() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        write_ext(&src, "9.9.9", &[("zz-late.js", "// late")]);
        fs::create_dir_all(&dst).unwrap();

        copy_over(&src, &dst).unwrap();
        assert_eq!(staged_version(&dst).as_deref(), Some("9.9.9"));
        assert!(dst.join("zz-late.js").is_file());

        // The non-manifest pass alone must NOT have produced a manifest —
        // that's what guarantees "manifest lands last" in `copy_over`.
        let dst2 = tmp.path().join("dst2");
        copy_dir_recursive_inner(&src, &dst2, true).unwrap();
        assert!(!dst2.join("manifest.json").exists());
        assert!(dst2.join("zz-late.js").is_file());
    }

    #[test]
    fn corrupt_canonical_manifest_triggers_resync() {
        let tmp = tempfile::tempdir().unwrap();
        let bundled = tmp.path().join("bundle");
        let canonical = tmp.path().join("extension");
        write_ext(&bundled, "1.2.0", &[]);
        write_ext(&canonical, "1.2.0", &[]);
        fs::write(canonical.join("manifest.json"), "{ not json").unwrap();

        let synced = sync_dirs(&bundled, &canonical).unwrap();
        assert_eq!(synced.as_deref(), Some("1.2.0"));
        assert_eq!(staged_version(&canonical).as_deref(), Some("1.2.0"));
    }
}
