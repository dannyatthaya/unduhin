//! Commands behind the details panel's "This file only" settings and file
//! actions, the New download link check, and Settings → General → Storage.
//!
//! Thin wrappers, like `commands.rs`: each one hands straight to
//! [`unduhin_core::Core`].

use std::path::PathBuf;

use tauri::State;

use unduhin_core::{folder_is_usable, Core, DownloadId, DownloadRecord, LinkProbe};

use crate::error::CommandResult;

/// Set one download's own speed cap in bytes per second. `None` or `0`
/// removes it.
#[tauri::command]
pub async fn set_download_speed_limit(
    core: State<'_, Core>,
    id: DownloadId,
    bps: Option<u64>,
) -> CommandResult<DownloadRecord> {
    Ok(core.set_speed_limit(id, bps).await?)
}

#[tauri::command]
pub async fn set_help_slow_parts(
    core: State<'_, Core>,
    id: DownloadId,
    enabled: bool,
) -> CommandResult<DownloadRecord> {
    Ok(core.set_help_slow_parts(id, enabled).await?)
}

/// SHA-256 of a finished download's file, as lower-case hex.
#[tauri::command]
pub async fn compute_checksum(core: State<'_, Core>, id: DownloadId) -> CommandResult<String> {
    Ok(core.compute_checksum(id).await?)
}

#[tauri::command]
pub async fn rename_download(
    core: State<'_, Core>,
    id: DownloadId,
    name: String,
) -> CommandResult<DownloadRecord> {
    Ok(core.rename_download(id, &name).await?)
}

#[tauri::command]
pub async fn move_download(
    core: State<'_, Core>,
    id: DownloadId,
    folder: String,
) -> CommandResult<DownloadRecord> {
    Ok(core.move_download(id, &PathBuf::from(folder)).await?)
}

/// Ask the server for a link's name, size and range support. Fails when the
/// server cannot be reached, so the dialog can say so.
#[tauri::command]
pub async fn probe_link(core: State<'_, Core>, url: String) -> CommandResult<LinkProbe> {
    Ok(core.probe_link(&url).await?)
}

/// Write the download list to `path` (`.csv` or `.json`); returns the row
/// count. `path` is meant to come from the system save dialog; the core
/// checks it all the same and refuses anything but a full path to an
/// ordinary `.csv` or `.json` file.
#[tauri::command]
pub async fn export_history(core: State<'_, Core>, path: String) -> CommandResult<usize> {
    Ok(core.export_history(&PathBuf::from(path)).await?)
}

/// Forget every finished download, leaving files on disk; returns how many.
#[tauri::command]
pub async fn clear_history(core: State<'_, Core>) -> CommandResult<usize> {
    Ok(core.clear_history().await?)
}

/// Show the system "Open with" chooser for a download's file. Windows
/// only: the other platforms have no chooser a program can summon, so the
/// details panel does not offer the action there.
#[tauri::command]
pub async fn open_with(core: State<'_, Core>, id: DownloadId) -> CommandResult<()> {
    let record = core.get_download(id).await?;
    if !record.output_path.is_file() {
        return Err(format!("{} is not on disk", record.output_path.display()).into());
    }
    show_open_with(&record.output_path)
}

#[cfg(windows)]
fn show_open_with(path: &std::path::Path) -> CommandResult<()> {
    // The path goes in as its own argument, never through a shell.
    std::process::Command::new("rundll32.exe")
        .arg("shell32.dll,OpenAs_RunDLL")
        .arg(path)
        .spawn()?;
    Ok(())
}

#[cfg(not(windows))]
fn show_open_with(_path: &std::path::Path) -> CommandResult<()> {
    Err("Open with is only available on Windows".into())
}

/// Whether `path` can be used as a download folder: it exists, or the
/// nearest folder above it does (so it can be created on first use). A path
/// on a drive that is not connected has no existing ancestor and fails.
/// The same rule [`Core::move_download`] applies before it moves a file.
#[tauri::command]
pub async fn check_folder(path: String) -> CommandResult<bool> {
    Ok(folder_is_usable(std::path::Path::new(path.trim())))
}

#[cfg(test)]
mod tests {
    use super::folder_is_usable;

    #[test]
    fn accepts_an_existing_folder_and_a_new_one_inside_it() {
        let dir = std::env::temp_dir();
        assert!(folder_is_usable(&dir));
        assert!(folder_is_usable(
            &dir.join("unduhin-not-created-yet").join("deeper")
        ));
    }

    #[test]
    fn rejects_relative_paths() {
        assert!(!folder_is_usable(std::path::Path::new("Downloads")));
        assert!(!folder_is_usable(std::path::Path::new("")));
    }

    #[test]
    fn rejects_a_path_under_a_file() {
        let file =
            std::env::temp_dir().join(format!("unduhin-check-folder-{}", std::process::id()));
        std::fs::write(&file, b"x").unwrap();
        let usable = folder_is_usable(&file.join("inside"));
        std::fs::remove_file(&file).unwrap();
        assert!(!usable);
    }

    #[cfg(windows)]
    #[test]
    fn rejects_a_drive_that_is_not_connected() {
        // No machine this runs on maps every letter; find one that is free.
        let free = ('D'..='Z')
            .map(|letter| format!("{letter}:\\"))
            .find(|root| !std::path::Path::new(root).exists());
        if let Some(root) = free {
            assert!(!folder_is_usable(
                &std::path::Path::new(&root).join("Media")
            ));
        }
    }
}
