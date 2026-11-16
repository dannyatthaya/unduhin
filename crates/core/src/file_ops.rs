//! Operations on one download's file and on its own settings, plus the
//! history export / clear that back Settings → General → Storage.
//!
//! Split out of `lib.rs` to keep the facade readable; everything here is
//! still a method on [`Core`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::download::{self, DownloadKind, DownloadRecord, Status};
use crate::error::{CoreError, Result};
use crate::event::CoreEvent;
use crate::{queue, Core, DownloadFilter, DownloadId};

/// Longest a link check may spend connecting or waiting for headers. The
/// dialog shows "Checking the link…" for this long at most.
const LINK_PROBE_TIMEOUT_SECS: u64 = 10;

/// Read buffer for hashing a finished file.
const HASH_BUFFER_BYTES: usize = 1024 * 1024;

/// What the server said about a link, before anything is downloaded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkProbe {
    /// Best file name from `Content-Disposition` or the final URL.
    pub filename: Option<String>,
    /// Body size, when the server reports one.
    pub total_bytes: Option<u64>,
    /// Whether the server accepts byte ranges: the download can resume and
    /// can be split into parts.
    pub resumable: bool,
    /// Host of the final URL, after redirects.
    pub host: Option<String>,
}

/// File formats [`Core::export_history`] can write, picked from the target
/// file's extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportFormat {
    Json,
    Csv,
}

impl ExportFormat {
    /// `None` for any extension other than `.csv` or `.json`.
    fn for_path(path: &Path) -> Option<Self> {
        let ext = path.extension().and_then(|e| e.to_str())?;
        if ext.eq_ignore_ascii_case("csv") {
            Some(ExportFormat::Csv)
        } else if ext.eq_ignore_ascii_case("json") {
            Some(ExportFormat::Json)
        } else {
            None
        }
    }
}

/// Whether `path` can be used as a download folder: it exists, or the
/// nearest folder above it does (so it can be created on first use). A path
/// on a drive that is not connected has no existing ancestor and fails.
pub fn folder_is_usable(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    match path.ancestors().find(|ancestor| ancestor.exists()) {
        Some(existing) => existing.is_dir(),
        None => false,
    }
}

/// Check where an export is about to be written and pick its format.
///
/// The path is meant to come from the system save dialog, but the command
/// takes it as plain text, so it is checked here: a full path, a `.csv` or
/// `.json` name, and nothing but an ordinary file (or nothing at all) there
/// already — never a folder, a link or a device.
async fn export_target(path: &Path) -> Result<ExportFormat> {
    if !path.is_absolute() {
        return Err(CoreError::InvalidArgument(
            "pick a full file path to export the list to".to_string(),
        ));
    }
    let format = ExportFormat::for_path(path).ok_or_else(|| {
        CoreError::InvalidArgument("the list can only be exported as .csv or .json".to_string())
    })?;
    match tokio::fs::symlink_metadata(path).await {
        Ok(existing) if !existing.file_type().is_file() => Err(CoreError::InvalidArgument(
            format!("{} is not a file that can be replaced", path.display()),
        )),
        // An ordinary file (the save dialog already asked about replacing
        // it) or nothing there yet. Any other error surfaces on the write.
        _ => Ok(format),
    }
}

impl Core {
    /// Set or clear one download's own speed cap. `None` (or `0`) removes it.
    ///
    /// Applies at once to a running HTTP download. A media download reads it
    /// when yt-dlp next starts; a torrent has no cap of its own.
    pub async fn set_speed_limit(
        &self,
        id: DownloadId,
        bps: Option<u64>,
    ) -> Result<DownloadRecord> {
        let bps = bps.filter(|n| *n > 0);
        // The column is a signed 64-bit integer; a larger value would wrap
        // to a negative one and read back as "no cap".
        let stored = bps
            .map(i64::try_from)
            .transpose()
            .map_err(|_| CoreError::InvalidArgument("that speed limit is too large".to_string()))?;
        let record = download::get(&self.inner.pool, id).await?;
        if record.kind == DownloadKind::Torrent {
            return Err(CoreError::InvalidArgument(
                "a torrent has no speed limit of its own".to_string(),
            ));
        }
        sqlx::query("UPDATE downloads SET speed_limit_bps = ? WHERE id = ?")
            .bind(stored)
            .bind(id)
            .execute(&self.inner.pool)
            .await?;
        if let Some(limiter) = self.tuning_for(id).await.and_then(|t| t.speed_limiter) {
            limiter.set_rate(bps.unwrap_or(0)).await;
        }
        download::get(&self.inner.pool, id).await
    }

    /// Turn "help slow parts" on or off for one download. Takes effect the
    /// next time one of its connections finishes a part.
    pub async fn set_help_slow_parts(&self, id: DownloadId, on: bool) -> Result<DownloadRecord> {
        // Validates the row exists before writing.
        download::get(&self.inner.pool, id).await?;
        sqlx::query("UPDATE downloads SET help_slow_parts = ? WHERE id = ?")
            .bind(on)
            .bind(id)
            .execute(&self.inner.pool)
            .await?;
        if let Some(flag) = self.tuning_for(id).await.and_then(|t| t.help_slow_parts) {
            flag.store(on, std::sync::atomic::Ordering::Relaxed);
        }
        download::get(&self.inner.pool, id).await
    }

    async fn tuning_for(&self, id: DownloadId) -> Option<engine::Tuning> {
        let queue = self.inner.queue.lock().await.clone();
        match queue {
            Some(q) => q.tuning_for(id).await,
            None => None,
        }
    }

    /// Hash a finished download's file with SHA-256, remember the result on
    /// the row, and return it as lower-case hex.
    pub async fn compute_checksum(&self, id: DownloadId) -> Result<String> {
        let record = download::get(&self.inner.pool, id).await?;
        if record.status != Status::Completed {
            return Err(CoreError::InvalidTransition {
                id,
                from: record.status.to_string(),
                to: "checksum".to_string(),
            });
        }
        if record.kind == DownloadKind::Torrent {
            return Err(CoreError::InvalidArgument(
                "a torrent is a folder of files; it has no single checksum".to_string(),
            ));
        }
        let path = record.output_path.clone();
        let digest = tokio::task::spawn_blocking(move || sha256_of_file(&path))
            .await
            .map_err(|e| CoreError::Io(std::io::Error::other(e.to_string())))??;
        sqlx::query("UPDATE downloads SET sha256 = ? WHERE id = ?")
            .bind(&digest)
            .bind(id)
            .execute(&self.inner.pool)
            .await?;
        Ok(digest)
    }

    /// Give a download a new file name, in the folder it is already in.
    ///
    /// A name typed without an extension keeps the old one, so the file
    /// still opens. Emits [`CoreEvent::PathsChanged`].
    pub async fn rename_download(&self, id: DownloadId, new_name: &str) -> Result<DownloadRecord> {
        let record = download::get(&self.inner.pool, id).await?;
        let name = renamed_file_name(&record.filename, new_name)?;
        let folder = parent_folder(&record)?;
        self.relocate(record, &folder, &name).await
    }

    /// Move a download into another folder, keeping its file name.
    /// Emits [`CoreEvent::PathsChanged`].
    pub async fn move_download(&self, id: DownloadId, folder: &Path) -> Result<DownloadRecord> {
        if folder.as_os_str().is_empty() || !folder.is_absolute() {
            return Err(CoreError::InvalidArgument(
                "pick a full folder path to move the download to".to_string(),
            ));
        }
        if !folder_is_usable(folder) {
            return Err(CoreError::InvalidArgument(format!(
                "{} cannot be used as a folder",
                folder.display()
            )));
        }
        let record = download::get(&self.inner.pool, id).await?;
        let name = record.filename.clone();
        self.relocate(record, folder, &name).await
    }

    /// Shared tail of rename and move: put the file (and its resume sidecar,
    /// if any) at `folder/name`, then point the row at where it landed.
    ///
    /// Never overwrites: a taken name gets ` (n)` appended, and the row
    /// records the name that was actually used.
    async fn relocate(
        &self,
        record: DownloadRecord,
        folder: &Path,
        name: &str,
    ) -> Result<DownloadRecord> {
        let id = record.id;
        if record.kind == DownloadKind::Torrent {
            return Err(CoreError::InvalidArgument(
                "a torrent's folder is managed by the torrent session and cannot be moved here"
                    .to_string(),
            ));
        }
        // A running worker holds the file open and keeps writing to the old
        // path; moving under it would split the download across two files.
        if matches!(record.status, Status::Active | Status::Muxing) {
            return Err(CoreError::InvalidTransition {
                id,
                from: record.status.to_string(),
                to: "moved".to_string(),
            });
        }

        let current = record.output_path.clone();
        let on_disk = tokio::fs::metadata(&current).await.is_ok();
        let new_path = if on_disk {
            let landed = download::move_renamed(&current, folder, std::ffi::OsStr::new(name)).await;
            if landed == current && folder.join(name) != current {
                return Err(CoreError::Io(std::io::Error::other(format!(
                    "could not move the file to {}",
                    folder.display()
                ))));
            }
            landed
        } else {
            // Nothing written yet (a queued row): only the plan changes.
            download::resolve_unique_output_path(&self.inner.pool, folder, name, record.kind)
                .await?
        };
        if new_path == current {
            return Ok(record);
        }

        if on_disk {
            carry_sidecar(&current, &new_path).await;
        }
        let final_name = new_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.to_string());
        sqlx::query("UPDATE downloads SET output_path = ?, filename = ? WHERE id = ?")
            .bind(new_path.to_string_lossy().as_ref())
            .bind(&final_name)
            .bind(id)
            .execute(&self.inner.pool)
            .await?;
        let _ = self.inner.events.send(CoreEvent::PathsChanged {
            id,
            filename: final_name,
            output_path: new_path.to_string_lossy().into_owned(),
        });
        download::get(&self.inner.pool, id).await
    }

    /// Ask the server about a link without downloading it: name, size, and
    /// whether it can resume. Backs the link card in New download.
    ///
    /// Unlike [`Core::preview_filename`] a failure is an error, so the
    /// dialog can say the server could not be reached.
    pub async fn probe_link(&self, url: &str) -> Result<LinkProbe> {
        let parsed: url::Url = url
            .parse()
            .map_err(|e: url::ParseError| CoreError::InvalidArgument(e.to_string()))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(CoreError::InvalidArgument(format!(
                "only http and https links can be checked, got {}",
                parsed.scheme()
            )));
        }
        let (connect, read) = queue::timeouts(&self.inner.pool).await;
        let cap = std::time::Duration::from_secs(LINK_PROBE_TIMEOUT_SECS);
        let user_agent = queue::user_agent_setting(&self.inner.pool).await;
        let client = engine::http::build_client(
            connect.min(cap),
            read.min(cap),
            user_agent.as_deref(),
            &[],
        )?;
        let info = engine::probe(&client, &parsed).await?;
        Ok(LinkProbe {
            filename: info.filename_hint,
            total_bytes: info.content_length,
            resumable: info.accept_ranges,
            host: info.url.host_str().map(str::to_string),
        })
    }

    /// Write the whole download list to `path` and return how many rows went
    /// out. A `.csv` target gets a spreadsheet-friendly table; a `.json`
    /// target gets every field. Any other target is refused, and so is one
    /// that is not an ordinary file.
    ///
    /// Captured request headers are never included (they are not serialized
    /// at all), but each link is written in full, so a token inside a link
    /// ends up in the file.
    pub async fn export_history(&self, path: &Path) -> Result<usize> {
        let format = export_target(path).await?;
        let records = download::list(&self.inner.pool, DownloadFilter::default()).await?;
        let body = match format {
            ExportFormat::Csv => history_csv(&records),
            ExportFormat::Json => serde_json::to_string_pretty(&records)?,
        };
        tokio::fs::write(path, body).await.map_err(CoreError::io)?;
        Ok(records.len())
    }

    /// Forget every finished download. Files on disk are not touched.
    /// Returns how many rows were removed.
    ///
    /// Only `completed` rows count as history: a paused or failed row still
    /// has a partial file the user may want to resume.
    pub async fn clear_history(&self) -> Result<usize> {
        let finished = download::list(
            &self.inner.pool,
            DownloadFilter {
                status: Some(Status::Completed),
                category_id: None,
            },
        )
        .await?;
        let mut removed = 0usize;
        for record in finished {
            // `remove` reports a file it could not delete as an error; with
            // `delete_data = false` it never touches the file, so an error
            // here means the row itself is still there.
            match self.remove(record.id, false).await {
                Ok(()) => removed += 1,
                Err(e) => {
                    tracing::warn!(id = record.id, error = %e, "clear_history: could not remove row");
                }
            }
        }
        Ok(removed)
    }
}

/// Folder a download's file lives in.
fn parent_folder(record: &DownloadRecord) -> Result<PathBuf> {
    record
        .output_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            CoreError::InvalidArgument("this download has no folder to rename it in".to_string())
        })
}

/// The file name to rename to: `requested` made safe for the file system,
/// with the old extension kept when the new name has none.
fn renamed_file_name(current: &str, requested: &str) -> Result<String> {
    let trimmed = requested.trim();
    if trimmed.is_empty() {
        return Err(CoreError::InvalidArgument(
            "a file name cannot be empty".to_string(),
        ));
    }
    let clean = download::sanitize_filename(trimmed);
    let has_extension = Path::new(&clean).extension().is_some();
    let old_extension = Path::new(current).extension().and_then(|e| e.to_str());
    Ok(match (has_extension, old_extension) {
        (false, Some(ext)) => format!("{clean}.{ext}"),
        _ => clean,
    })
}

/// Move a partial download's resume sidecar next to the file's new home and
/// point it at the new path. The worker resumes against the path stored in
/// the sidecar, not the row, so renaming the sidecar alone would send the
/// rest of the download back to the old location.
///
/// Best-effort: a finished file has no sidecar, and losing one only costs a
/// restart of that download.
async fn carry_sidecar(old_output: &Path, new_output: &Path) {
    let old_sidecar = engine::Meta::sidecar_path(old_output);
    let Ok(mut meta) = engine::Meta::load(&old_sidecar).await else {
        return;
    };
    meta.output_path = new_output.to_path_buf();
    let new_sidecar = engine::Meta::sidecar_path(new_output);
    match meta.save(&new_sidecar).await {
        Ok(()) => {
            let _ = tokio::fs::remove_file(&old_sidecar).await;
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not carry the resume sidecar to the new location");
        }
    }
}

fn sha256_of_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    // Hash the download itself, never whatever a link left in its place
    // points at.
    if !std::fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(std::io::Error::other(format!(
            "{} is not an ordinary file",
            path.display()
        )));
    }
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// The download list as CSV: one header row, then one row per download.
fn history_csv(records: &[DownloadRecord]) -> String {
    let mut out = String::from("name,link,status,size_bytes,added,finished,saved_to\r\n");
    for r in records {
        let fields = [
            r.filename.clone(),
            r.url.clone(),
            r.status.to_string(),
            r.total_bytes.map(|n| n.to_string()).unwrap_or_default(),
            r.created_at.to_rfc3339(),
            r.completed_at.map(|t| t.to_rfc3339()).unwrap_or_default(),
            r.output_path.to_string_lossy().into_owned(),
        ];
        let line: Vec<String> = fields.iter().map(|f| csv_field(f)).collect();
        out.push_str(&line.join(","));
        out.push_str("\r\n");
    }
    out
}

/// Quote a CSV field when it holds a comma, a quote or a line break, and
/// neutralise a leading `=`, `+`, `-`, `@`, tab or carriage return so a
/// spreadsheet does not run a file name or link as a formula.
fn csv_field(value: &str) -> String {
    let guarded = match value.chars().next() {
        Some('=' | '+' | '-' | '@' | '\t' | '\r') => format!("'{value}"),
        _ => value.to_string(),
    };
    if guarded.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AddDownload, Core};

    fn add_input(url: &str, output_path: PathBuf) -> AddDownload {
        AddDownload {
            url: url.parse().unwrap(),
            filename: None,
            output_path: Some(output_path),
            output_dir: None,
            category: None,
            priority: 0,
            segments: None,
            media_info: None,
            headers: None,
            source: crate::DownloadSource::Manual,
            kind: DownloadKind::Http,
            torrent: None,
        }
    }

    async fn core_with_completed_file(dir: &Path, name: &str, body: &[u8]) -> (Core, DownloadId) {
        let core = Core::open_in_memory().await.unwrap();
        let path = dir.join(name);
        tokio::fs::write(&path, body).await.unwrap();
        let id = core
            .add_download(add_input(&format!("https://example.com/{name}"), path))
            .await
            .unwrap();
        sqlx::query("UPDATE downloads SET status = 'completed' WHERE id = ?")
            .bind(id)
            .execute(core.pool())
            .await
            .unwrap();
        (core, id)
    }

    #[test]
    fn rename_keeps_the_old_extension_when_none_is_typed() {
        assert_eq!(
            renamed_file_name("video-file-09.mp4", "lecture-09").unwrap(),
            "lecture-09.mp4"
        );
    }

    #[test]
    fn rename_honours_a_typed_extension() {
        assert_eq!(
            renamed_file_name("clip.mp4", "clip.mkv").unwrap(),
            "clip.mkv"
        );
    }

    #[test]
    fn rename_rejects_an_empty_name() {
        assert!(matches!(
            renamed_file_name("clip.mp4", "   "),
            Err(CoreError::InvalidArgument(_))
        ));
    }

    #[test]
    fn rename_replaces_path_separators() {
        let name = renamed_file_name("clip.mp4", "..\\evil/name").unwrap();
        assert!(!name.contains('/') && !name.contains('\\'), "{name}");
    }

    #[test]
    fn csv_field_quotes_commas_and_quotes() {
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("plain"), "plain");
    }

    #[test]
    fn csv_field_defuses_a_leading_formula_character() {
        assert_eq!(csv_field("=SUM(A1)"), "'=SUM(A1)");
        assert_eq!(csv_field("@cmd"), "'@cmd");
    }

    #[test]
    fn export_format_follows_the_extension() {
        assert_eq!(
            ExportFormat::for_path(Path::new("list.CSV")),
            Some(ExportFormat::Csv)
        );
        assert_eq!(
            ExportFormat::for_path(Path::new("list.json")),
            Some(ExportFormat::Json)
        );
        assert_eq!(ExportFormat::for_path(Path::new("list")), None);
        assert_eq!(ExportFormat::for_path(Path::new("list.txt")), None);
    }

    #[tokio::test]
    async fn checksum_hashes_the_file_and_is_remembered() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, id) = core_with_completed_file(tmp.path(), "a.bin", b"abc").await;

        let digest = core.compute_checksum(id).await.unwrap();

        assert_eq!(
            digest,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let record = core.get_download(id).await.unwrap();
        assert_eq!(record.sha256.as_deref(), Some(digest.as_str()));
    }

    #[tokio::test]
    async fn checksum_refuses_an_unfinished_download() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, id) = core_with_completed_file(tmp.path(), "a.bin", b"abc").await;
        sqlx::query("UPDATE downloads SET status = 'paused' WHERE id = ?")
            .bind(id)
            .execute(core.pool())
            .await
            .unwrap();

        let result = core.compute_checksum(id).await;

        assert!(matches!(result, Err(CoreError::InvalidTransition { .. })));
    }

    #[tokio::test]
    async fn rename_moves_the_file_and_updates_the_row() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, id) = core_with_completed_file(tmp.path(), "old.bin", b"data").await;

        let record = core.rename_download(id, "new").await.unwrap();

        assert_eq!(record.filename, "new.bin");
        assert_eq!(record.output_path, tmp.path().join("new.bin"));
        assert!(tmp.path().join("new.bin").exists());
        assert!(!tmp.path().join("old.bin").exists());
    }

    #[tokio::test]
    async fn rename_never_overwrites_an_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, id) = core_with_completed_file(tmp.path(), "old.bin", b"mine").await;
        tokio::fs::write(tmp.path().join("taken.bin"), b"theirs")
            .await
            .unwrap();

        let record = core.rename_download(id, "taken").await.unwrap();

        assert_eq!(record.filename, "taken (1).bin");
        let untouched = tokio::fs::read(tmp.path().join("taken.bin")).await.unwrap();
        assert_eq!(untouched, b"theirs");
    }

    #[tokio::test]
    async fn move_puts_the_file_in_the_new_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("elsewhere");
        let (core, id) = core_with_completed_file(tmp.path(), "a.bin", b"data").await;

        let record = core.move_download(id, &dest).await.unwrap();

        assert_eq!(record.output_path, dest.join("a.bin"));
        assert!(dest.join("a.bin").exists());
    }

    #[tokio::test]
    async fn move_refuses_a_relative_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, id) = core_with_completed_file(tmp.path(), "a.bin", b"data").await;

        let result = core.move_download(id, Path::new("relative/dir")).await;

        assert!(matches!(result, Err(CoreError::InvalidArgument(_))));
    }

    #[tokio::test]
    async fn move_refuses_a_running_download() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, id) = core_with_completed_file(tmp.path(), "a.bin", b"data").await;
        sqlx::query("UPDATE downloads SET status = 'active' WHERE id = ?")
            .bind(id)
            .execute(core.pool())
            .await
            .unwrap();

        let result = core.move_download(id, &tmp.path().join("x")).await;

        assert!(matches!(result, Err(CoreError::InvalidTransition { .. })));
    }

    #[tokio::test]
    async fn speed_limit_round_trips_and_zero_clears_it() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, id) = core_with_completed_file(tmp.path(), "a.bin", b"data").await;

        let limited = core.set_speed_limit(id, Some(1_048_576)).await.unwrap();
        assert_eq!(limited.speed_limit_bps, Some(1_048_576));

        let cleared = core.set_speed_limit(id, Some(0)).await.unwrap();
        assert_eq!(cleared.speed_limit_bps, None);
    }

    #[tokio::test]
    async fn help_slow_parts_defaults_on_and_can_be_switched_off() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, id) = core_with_completed_file(tmp.path(), "a.bin", b"data").await;
        assert!(core.get_download(id).await.unwrap().help_slow_parts);

        let record = core.set_help_slow_parts(id, false).await.unwrap();

        assert!(!record.help_slow_parts);
    }

    #[tokio::test]
    async fn clear_history_removes_finished_rows_and_keeps_files() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, done) = core_with_completed_file(tmp.path(), "done.bin", b"data").await;
        let paused = core
            .add_download(add_input(
                "https://example.com/paused.bin",
                tmp.path().join("paused.bin"),
            ))
            .await
            .unwrap();
        sqlx::query("UPDATE downloads SET status = 'paused' WHERE id = ?")
            .bind(paused)
            .execute(core.pool())
            .await
            .unwrap();

        let removed = core.clear_history().await.unwrap();

        assert_eq!(removed, 1);
        assert!(core.get_download(done).await.is_err());
        assert!(core.get_download(paused).await.is_ok());
        assert!(tmp.path().join("done.bin").exists(), "the file must stay");
    }

    #[tokio::test]
    async fn export_history_writes_csv_with_a_header_row() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, _id) = core_with_completed_file(tmp.path(), "a.bin", b"data").await;
        let target = tmp.path().join("history.csv");

        let count = core.export_history(&target).await.unwrap();

        assert_eq!(count, 1);
        let text = tokio::fs::read_to_string(&target).await.unwrap();
        assert!(text.starts_with("name,link,status,"));
        assert!(text.contains("a.bin"));
    }

    #[tokio::test]
    async fn export_history_writes_json_for_a_json_target() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, _id) = core_with_completed_file(tmp.path(), "a.bin", b"data").await;
        let target = tmp.path().join("history.JSON");

        let count = core.export_history(&target).await.unwrap();

        assert_eq!(count, 1);
        let text = tokio::fs::read_to_string(&target).await.unwrap();
        let rows: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn export_history_refuses_another_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, _id) = core_with_completed_file(tmp.path(), "a.bin", b"data").await;
        let target = tmp.path().join("history.bat");

        let result = core.export_history(&target).await;

        assert!(matches!(result, Err(CoreError::InvalidArgument(_))));
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn export_history_refuses_a_relative_path() {
        let core = Core::open_in_memory().await.unwrap();

        let result = core.export_history(Path::new("history.csv")).await;

        assert!(matches!(result, Err(CoreError::InvalidArgument(_))));
    }

    #[tokio::test]
    async fn export_history_refuses_a_folder_as_the_target() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, _id) = core_with_completed_file(tmp.path(), "a.bin", b"data").await;
        let target = tmp.path().join("folder.csv");
        tokio::fs::create_dir(&target).await.unwrap();

        let result = core.export_history(&target).await;

        assert!(matches!(result, Err(CoreError::InvalidArgument(_))));
    }

    #[tokio::test]
    async fn move_refuses_a_folder_under_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, id) = core_with_completed_file(tmp.path(), "a.bin", b"data").await;

        let result = core
            .move_download(id, &tmp.path().join("a.bin").join("inside"))
            .await;

        assert!(matches!(result, Err(CoreError::InvalidArgument(_))));
        assert!(tmp.path().join("a.bin").exists(), "the file must stay put");
    }

    #[tokio::test]
    async fn speed_limit_refuses_a_value_the_database_cannot_hold() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, id) = core_with_completed_file(tmp.path(), "a.bin", b"data").await;

        let result = core.set_speed_limit(id, Some(u64::MAX)).await;

        assert!(matches!(result, Err(CoreError::InvalidArgument(_))));
        assert_eq!(core.get_download(id).await.unwrap().speed_limit_bps, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn checksum_refuses_a_link_left_in_place_of_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, id) = core_with_completed_file(tmp.path(), "a.bin", b"data").await;
        let elsewhere = tmp.path().join("elsewhere.bin");
        tokio::fs::write(&elsewhere, b"other").await.unwrap();
        tokio::fs::remove_file(tmp.path().join("a.bin"))
            .await
            .unwrap();
        std::os::unix::fs::symlink(&elsewhere, tmp.path().join("a.bin")).unwrap();

        let result = core.compute_checksum(id).await;

        assert!(result.is_err());
    }

    #[test]
    fn csv_field_guards_formula_starters() {
        for starter in ["=1+1", "+1", "-1", "@SUM(A1)", "\tcmd", "\rcmd"] {
            let field = csv_field(starter);
            assert!(
                field.trim_start_matches('"').starts_with('\''),
                "{starter:?} → {field:?}"
            );
        }
        assert_eq!(csv_field("plain.bin"), "plain.bin");
    }

    #[tokio::test]
    async fn probe_link_rejects_a_non_http_scheme() {
        let core = Core::open_in_memory().await.unwrap();

        let result = core.probe_link("ftp://example.com/file.bin").await;

        assert!(matches!(result, Err(CoreError::InvalidArgument(_))));
    }
}
