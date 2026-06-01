//! `downloads` table: typed record, repository functions, and the
//! enum of statuses.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use engine::SegmentState;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::category::auto_categorize_for_filename;
use crate::error::{CoreError, Result};
use crate::ytdlp::MediaInfo;

/// Database id for a download row.
pub type DownloadId = i64;

/// Which surface created the download row. Persisted to the
/// `downloads.source` column (migration `20260902000001_downloads_source`)
/// so the Settings → Browser status card can light up the "downloads
/// captured this week" counter without a sessions-table JOIN.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DownloadSource {
    /// Add URL dialog, drag-drop, or any other in-app surface.
    #[default]
    Manual,
    /// Native-messaging host hand-off via `\\.\pipe\unduhin`.
    ExtensionPipe,
    /// `unduhin add` from the CLI.
    Cli,
}

impl DownloadSource {
    pub fn as_str(self) -> &'static str {
        match self {
            DownloadSource::Manual => "manual",
            DownloadSource::ExtensionPipe => "extension_pipe",
            DownloadSource::Cli => "cli",
        }
    }
}

impl std::fmt::Display for DownloadSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DownloadSource {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "manual" => DownloadSource::Manual,
            "extension_pipe" => DownloadSource::ExtensionPipe,
            "cli" => DownloadSource::Cli,
            other => {
                return Err(CoreError::InvalidArgument(format!(
                    "unknown download source: {other}"
                )))
            }
        })
    }
}

/// Lifecycle status. Stored as the lowercase string in the DB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Queued,
    Active,
    /// yt-dlp finished one stream and is downloading the next (or running
    /// ffmpeg to merge audio+video). The progress bar restarts from 0%
    /// during this phase, which is why we surface a distinct status
    /// instead of leaving the row on `Active`.
    Muxing,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

/// All statuses, useful for iteration in the CLI and UI.
pub const ALL_STATUSES: &[Status] = &[
    Status::Queued,
    Status::Active,
    Status::Muxing,
    Status::Paused,
    Status::Completed,
    Status::Failed,
    Status::Cancelled,
];

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Queued => "queued",
            Status::Active => "active",
            Status::Muxing => "muxing",
            Status::Paused => "paused",
            Status::Completed => "completed",
            Status::Failed => "failed",
            Status::Cancelled => "cancelled",
        }
    }

    /// True if the download is terminal — no more transitions expected
    /// without explicit user action.
    pub fn is_terminal(self) -> bool {
        matches!(self, Status::Completed | Status::Failed | Status::Cancelled)
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Status {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "queued" => Status::Queued,
            "active" => Status::Active,
            "muxing" => Status::Muxing,
            "paused" => Status::Paused,
            "completed" => Status::Completed,
            "failed" => Status::Failed,
            "cancelled" => Status::Cancelled,
            other => return Err(CoreError::InvalidStatus(other.to_string())),
        })
    }
}

/// Persisted view of one download.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRecord {
    pub id: DownloadId,
    pub url: String,
    pub filename: String,
    pub output_path: PathBuf,
    pub total_bytes: Option<u64>,
    pub downloaded_bytes: u64,
    pub status: Status,
    pub error: Option<String>,
    pub category_id: Option<i64>,
    pub priority: i64,
    pub segments: u32,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// Cached engine sidecar state, JSON-encoded. `None` until the first
    /// transfer attempt produces one.
    pub segments_meta: Option<Vec<SegmentState>>,
    /// Present when this row was created from a yt-dlp probe. The queue
    /// worker branches on this: if `Some`, the download is delegated to
    /// the [`crate::ytdlp`] subprocess flow instead of the engine.
    pub media_info: Option<MediaInfo>,
    /// Captured browser request headers (Cookie, Referer, User-Agent,
    /// observed `webRequest` headers) replayed on every engine request
    /// and forwarded to yt-dlp via `--add-header`. `None` when the row
    /// was added without browser capture context (CLI, Add URL dialog).
    pub headers: Option<Vec<(String, String)>>,
    /// Which surface added this row. Used by the Settings → Browser
    /// status card to count extension hand-offs. Older rows
    /// land as `Manual` per the NOT NULL
    /// DEFAULT applied by `20260902000001_downloads_source.sql`.
    #[serde(default)]
    pub source: DownloadSource,
    /// Downsampled bytes-per-second series, persisted once on completion so
    /// the detail-pane sparkline renders for downloads that finished before
    /// this session. `None` while in flight or for rows predating the
    /// `20260904000001_downloads_speed_samples.sql` column.
    #[serde(default)]
    pub speed_samples: Option<Vec<u32>>,
}

/// Inputs to [`crate::Core::add_download`].
#[derive(Debug, Clone)]
pub struct AddDownload {
    pub url: url::Url,
    /// Final filename. If `None`, derived from the URL path; the engine's
    /// HEAD-probe filename hint is applied at transfer time when it
    /// supplies a better value (Content-Disposition).
    pub filename: Option<String>,
    /// Final output path. If `None`, joined from the category's
    /// `default_output_path` (or the global `default_output_path`).
    pub output_path: Option<PathBuf>,
    pub category: Option<CategorySelector>,
    pub priority: i64,
    pub segments: Option<u32>,
    /// When set, the queue worker delegates to yt-dlp instead of the
    /// engine. Filename derivation skips the HEAD probe because yt-dlp
    /// already supplied the metadata.
    pub media_info: Option<MediaInfo>,
    /// Captured browser request headers — typically populated by the
    /// native messaging host when the extension forwards a
    /// cancelled browser download. `None` / empty for direct user-paste.
    pub headers: Option<Vec<(String, String)>>,
    /// Provenance — which surface initiated this download. Manual UI
    /// paths pass `Manual`; the pipe sets `ExtensionPipe`; the
    /// CLI sets `Cli`. Persisted to `downloads.source`.
    pub source: DownloadSource,
}

/// Either a database id or a name when calling [`AddDownload`]. The CLI
/// passes a name; the UI will mostly pass an id from the sidebar.
#[derive(Debug, Clone)]
pub enum CategorySelector {
    Id(i64),
    Name(String),
}

/// Filter applied to [`crate::Core::list_downloads`].
#[derive(Debug, Default, Clone)]
pub struct DownloadFilter {
    pub status: Option<Status>,
    pub category_id: Option<i64>,
}

pub(crate) async fn insert(pool: &SqlitePool, input: AddDownload) -> Result<DownloadRecord> {
    let AddDownload {
        url,
        filename,
        output_path,
        category,
        priority,
        segments,
        media_info,
        headers,
        source,
    } = input;

    // If the caller supplied a filename, respect it. yt-dlp-driven rows
    // bring their own title (we use it as the filename stem) and skip
    // the HEAD probe entirely. Otherwise pre-probe the URL so we capture
    // Content-Disposition / final-redirect / MIME signals that path-tail
    // alone can't see — without this, randomized URLs like `/d/abc123xyz`
    // get saved as extension-less garbage.
    let filename = match (filename, media_info.as_ref()) {
        (Some(f), _) => f,
        (None, Some(info)) => sanitize_filename(&info.title),
        (None, None) => probe_filename(pool, &url)
            .await
            .or_else(|| filename_from_url(&url))
            .unwrap_or_else(|| "download.bin".to_string()),
    };

    // Path-traversal guard. Every filename source converges here: an
    // explicit caller-supplied name, the yt-dlp title, the HEAD-probe
    // hint, and the URL-path tail — including the browser bridge, which
    // forwards `job.filename` verbatim through the named pipe. Only some
    // of those sources sanitized before; running the result through
    // `sanitize_filename` unconditionally (it is idempotent) strips path
    // separators, drive colons, and `..` so the `resolve_output_path`
    // join below can never escape the target folder.
    let filename = sanitize_filename(&filename);

    // Resolve category: explicit selector wins; otherwise auto-detect by
    // filename extension; otherwise fall back to "Other".
    let category_id = match category {
        Some(CategorySelector::Id(id)) => Some(id),
        Some(CategorySelector::Name(name)) => Some(
            crate::category::find_by_name(pool, &name)
                .await?
                .ok_or(CoreError::CategoryNameNotFound(name))?
                .id,
        ),
        None => auto_categorize_for_filename(pool, &filename).await?,
    };

    let resolved_output = resolve_output_path(pool, &filename, &output_path, category_id).await?;
    let segments = segments.unwrap_or(default_segments(pool).await?);

    let created_at = crate::now_iso();
    let media_info_json = match media_info.as_ref() {
        Some(info) => Some(serde_json::to_string(info)?),
        None => None,
    };
    // Headers are stored as JSON `[[name, value], ...]` so the column is
    // self-describing and survives schema growth without another migration.
    // The JSON can carry `Cookie` / `Authorization` (incl. HttpOnly cookies)
    // captured from the browser, so it is encrypted at rest via DPAPI before
    // it touches the (unencrypted) SQLite file. See `crate::secret`.
    let headers_json = match headers.as_ref() {
        Some(pairs) if !pairs.is_empty() => {
            Some(crate::secret::protect(&serde_json::to_string(pairs)?))
        }
        _ => None,
    };
    let row = sqlx::query(
        "INSERT INTO downloads (\
            url, filename, output_path, total_bytes, downloaded_bytes, status, \
            category_id, priority, segments, created_at, media_info, headers, source) \
         VALUES (?, ?, ?, NULL, 0, 'queued', ?, ?, ?, ?, ?, ?, ?) \
         RETURNING id",
    )
    .bind(url.as_str())
    .bind(&filename)
    .bind(resolved_output.to_string_lossy().as_ref())
    .bind(category_id)
    .bind(priority)
    .bind(segments as i64)
    .bind(&created_at)
    .bind(media_info_json)
    .bind(headers_json)
    .bind(source.as_str())
    .fetch_one(pool)
    .await?;

    let id: i64 = row.get("id");
    get(pool, id).await
}

/// Make an arbitrary string safe to use as a single Windows filename.
/// Strips path separators (`/` `\`), the drive colon, and other reserved
/// characters, drops control characters, trims trailing dots/whitespace,
/// and caps the length. This is the one sanitizer applied to *every*
/// download filename in [`insert`] (see the path-traversal guard there),
/// so it must never emit a separator, a `..`, or an empty string. yt-dlp's
/// own `--restrict-filenames` does similar work but we want consistency
/// with the rest of our filename derivation.
fn sanitize_filename(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    let trimmed = out.trim().trim_end_matches('.').to_string();
    out = if trimmed.is_empty() {
        "download".to_string()
    } else {
        trimmed
    };
    if out.len() > 200 {
        out.truncate(200);
    }
    out
}

pub(crate) async fn get(pool: &SqlitePool, id: DownloadId) -> Result<DownloadRecord> {
    let row = sqlx::query("SELECT * FROM downloads WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let row = row.ok_or(CoreError::DownloadNotFound(id))?;
    record_from_row(&row)
}

pub(crate) async fn list(pool: &SqlitePool, filter: DownloadFilter) -> Result<Vec<DownloadRecord>> {
    let mut sql = String::from("SELECT * FROM downloads WHERE 1 = 1");
    if filter.status.is_some() {
        sql.push_str(" AND status = ?");
    }
    if filter.category_id.is_some() {
        sql.push_str(" AND category_id = ?");
    }
    sql.push_str(" ORDER BY priority DESC, created_at ASC");

    let mut q = sqlx::query(&sql);
    if let Some(s) = filter.status {
        q = q.bind(s.as_str());
    }
    if let Some(cid) = filter.category_id {
        q = q.bind(cid);
    }
    let rows = q.fetch_all(pool).await?;
    rows.iter().map(record_from_row).collect()
}

/// Result of [`remove`] — communicates which on-disk artefacts the
/// caller decided to delete, so the queue layer can surface a sensible
/// message when the file was missing or refused to delete.
#[derive(Debug, Default, Clone)]
pub(crate) struct RemoveOutcome {
    /// `true` when the caller asked to delete the file too AND we
    /// either succeeded or the file was already gone.
    pub data_deleted: bool,
    /// `Some` when caller asked to delete data but the OS refused
    /// (permission denied, file locked, etc). The DB row is still gone.
    pub data_error: Option<String>,
}

pub(crate) async fn remove(
    pool: &SqlitePool,
    id: DownloadId,
    delete_data: bool,
) -> Result<RemoveOutcome> {
    // Snapshot the path before deleting so we can clean up on disk.
    let record = get(pool, id).await?;
    let res = sqlx::query("DELETE FROM downloads WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(CoreError::DownloadNotFound(id));
    }

    let mut outcome = RemoveOutcome::default();
    if delete_data {
        // Best-effort delete: report failure on the row, but don't
        // roll back the DB delete — the user asked to remove the row.
        match tokio::fs::remove_file(&record.output_path).await {
            Ok(()) => outcome.data_deleted = true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                outcome.data_deleted = true;
            }
            Err(e) => outcome.data_error = Some(e.to_string()),
        }
        // Sidecar is engine-only state; cleaning it up here keeps stale
        // files from accumulating, but we ignore errors.
        let sidecar = engine::Meta::sidecar_path(&record.output_path);
        let _ = tokio::fs::remove_file(&sidecar).await;
    }
    Ok(outcome)
}

pub(crate) async fn set_priority(pool: &SqlitePool, id: DownloadId, priority: i64) -> Result<()> {
    let res = sqlx::query("UPDATE downloads SET priority = ? WHERE id = ?")
        .bind(priority)
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(CoreError::DownloadNotFound(id));
    }
    Ok(())
}

pub(crate) async fn update_segments(
    pool: &SqlitePool,
    id: DownloadId,
    segments: u32,
) -> Result<()> {
    let res = sqlx::query("UPDATE downloads SET segments = ? WHERE id = ?")
        .bind(segments as i64)
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(CoreError::DownloadNotFound(id));
    }
    Ok(())
}

/// Atomically move a download to `to` provided its current status is in
/// `allowed_from`. Returns the previous status on success.
///
/// We read first (no lock), then issue a single conditional UPDATE that
/// is exact-match on the status we read. If a concurrent writer changed
/// the row in between, the UPDATE affects zero rows and we re-read to
/// produce a precise error. Crucially, we do NOT open a multi-statement
/// transaction — `BEGIN DEFERRED` + later UPDATE is the classic
/// "database is locked" trap under any concurrent writer in SQLite.
pub(crate) async fn transition_status(
    pool: &SqlitePool,
    id: DownloadId,
    allowed_from: &[Status],
    to: Status,
) -> Result<Status> {
    let row = sqlx::query("SELECT status FROM downloads WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let row = row.ok_or(CoreError::DownloadNotFound(id))?;
    let current_str: String = row.get("status");
    let current: Status = current_str.parse()?;

    if current == to {
        return Ok(current);
    }
    if !allowed_from.contains(&current) {
        return Err(CoreError::InvalidTransition {
            id,
            from: current.to_string(),
            to: to.to_string(),
        });
    }

    let res = sqlx::query(
        "UPDATE downloads SET status = ?, error = NULL \
         WHERE id = ? AND status = ?",
    )
    .bind(to.as_str())
    .bind(id)
    .bind(current.as_str())
    .execute(pool)
    .await?;

    if res.rows_affected() == 1 {
        Ok(current)
    } else {
        // Status moved out from under us; the caller's mental model is
        // stale. Surface the actual current value.
        let now_row = sqlx::query("SELECT status FROM downloads WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
        let now: Status = now_row
            .ok_or(CoreError::DownloadNotFound(id))?
            .get::<String, _>("status")
            .parse()?;
        Err(CoreError::InvalidTransition {
            id,
            from: now.to_string(),
            to: to.to_string(),
        })
    }
}

/// Set a download's `category_id`. `None` clears the assignment.
/// Validates the row exists; the caller is responsible for checking the
/// category id (if `Some`) maps to a real row before calling.
pub(crate) async fn set_category(
    pool: &SqlitePool,
    id: DownloadId,
    category_id: Option<crate::category::CategoryId>,
) -> Result<()> {
    let res = sqlx::query("UPDATE downloads SET category_id = ? WHERE id = ?")
        .bind(category_id)
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(CoreError::DownloadNotFound(id));
    }
    Ok(())
}

/// Reconcile a yt-dlp row once the download finishes and the true
/// filename, on-disk size, and extension are known. At insert time the
/// title-derived filename has no extension, so all three fields are
/// based on a stale view of the file. This rewrites all four columns in
/// a single UPDATE.
///
/// Category handling preserves an explicit user choice: if the row's
/// current `category_id` matches what `auto_categorize_for_filename`
/// would have returned for the *original* (extensionless) filename,
/// the row was auto-categorized — re-evaluate against the new filename.
/// Otherwise the user picked something deliberately; leave it alone.
///
/// When the category changes, the file was written into the *old*
/// category's folder (the row was auto-categorized to "Other" at insert
/// because the title had no extension). This moves the file into the new
/// category's folder so the on-disk location matches the UI, then records
/// the final path in the DB.
///
/// Returns the final on-disk path (after any move/de-dup) and
/// `Some(new_category_id)` when the category actually changed, so the
/// caller can emit `PathsChanged` / `CategoryChanged` events.
pub(crate) async fn finalize_ytdlp_completion(
    pool: &SqlitePool,
    id: DownloadId,
    filename: &str,
    output_path: &std::path::Path,
    total_bytes: u64,
) -> Result<(PathBuf, Option<crate::category::CategoryId>)> {
    let row = sqlx::query("SELECT filename, category_id FROM downloads WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let row = row.ok_or(CoreError::DownloadNotFound(id))?;
    let original_filename: String = row.get("filename");
    let current_category: Option<i64> = row.get("category_id");

    let auto_for_original = auto_categorize_for_filename(pool, &original_filename).await?;
    let new_category = if current_category == auto_for_original {
        auto_categorize_for_filename(pool, filename).await?
    } else {
        current_category
    };

    // Move the finished file into the new category's folder when the
    // category changed. Best-effort: on any IO failure the file stays put
    // and the DB points at its real location (never a phantom path).
    let category_changed = new_category != current_category;
    let final_on_disk = if category_changed && tokio::fs::metadata(output_path).await.is_ok() {
        let target_folder = category_target_folder(pool, new_category).await?;
        if output_path.parent() == Some(target_folder.as_path()) {
            output_path.to_path_buf()
        } else {
            move_into_folder(output_path, &target_folder).await
        }
    } else {
        output_path.to_path_buf()
    };
    // A move may de-duplicate the name (e.g. `clip (1).mp4`); keep the
    // stored filename and path consistent with what's on disk.
    let final_name = final_on_disk
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(filename)
        .to_string();

    sqlx::query(
        "UPDATE downloads SET filename = ?, output_path = ?, \
                              total_bytes = ?, category_id = ? \
         WHERE id = ?",
    )
    .bind(&final_name)
    .bind(final_on_disk.to_string_lossy().as_ref())
    .bind(total_bytes as i64)
    .bind(new_category)
    .bind(id)
    .execute(pool)
    .await?;

    Ok((
        final_on_disk,
        if category_changed { new_category } else { None },
    ))
}

/// Resolve the folder a download in `category_id` should live in: the
/// category's configured folder, else the global default, else the current
/// directory. Mirrors the folder selection in [`resolve_output_path`].
async fn category_target_folder(pool: &SqlitePool, category_id: Option<i64>) -> Result<PathBuf> {
    if let Some(id) = category_id {
        if let Ok(cat) = crate::category::get(pool, id).await {
            if let Some(folder) = cat.default_output_path {
                if !folder.as_os_str().is_empty() {
                    return Ok(folder);
                }
            }
        }
    }
    let global = crate::settings::get(pool, "default_output_path")
        .await?
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());
    if let Some(g) = global {
        return Ok(PathBuf::from(g));
    }
    Ok(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Move `from` into `dest_dir`, keeping the file name. Returns the path the
/// file ended up at (which may carry a ` (n)` suffix if the destination
/// already existed). Falls back to copy + delete for cross-volume moves.
/// Best-effort: on any IO failure returns `from` unchanged so the DB keeps
/// pointing at the real file rather than a path that doesn't exist.
async fn move_into_folder(from: &Path, dest_dir: &Path) -> PathBuf {
    let file_name = match from.file_name() {
        Some(n) => n.to_owned(),
        None => return from.to_path_buf(),
    };
    if tokio::fs::create_dir_all(dest_dir).await.is_err() {
        return from.to_path_buf();
    }
    let mut dest = dest_dir.join(&file_name);
    if dest == from {
        return from.to_path_buf();
    }
    if tokio::fs::metadata(&dest).await.is_ok() {
        dest = dedupe_path(dest_dir, &file_name).await;
    }
    match tokio::fs::rename(from, &dest).await {
        Ok(()) => dest,
        // Cross-volume rename fails on Windows; fall back to copy + delete.
        Err(_) => match tokio::fs::copy(from, &dest).await {
            Ok(_) => {
                let _ = tokio::fs::remove_file(from).await;
                dest
            }
            Err(e) => {
                tracing::warn!(error = %e, from = %from.display(), to = %dest.display(),
                    "failed to move completed download into category folder");
                from.to_path_buf()
            }
        },
    }
}

/// Find the first non-colliding `stem (n).ext` path in `dir`.
async fn dedupe_path(dir: &Path, file_name: &std::ffi::OsStr) -> PathBuf {
    let name = Path::new(file_name);
    let stem = name
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = name.extension().map(|e| e.to_string_lossy().into_owned());
    for i in 1..1000 {
        let candidate_name = match &ext {
            Some(e) => format!("{stem} ({i}).{e}"),
            None => format!("{stem} ({i})"),
        };
        let candidate = dir.join(candidate_name);
        if tokio::fs::metadata(&candidate).await.is_err() {
            return candidate;
        }
    }
    dir.join(file_name)
}

/// Mark a download `completed`. Called by the queue manager.
///
/// `bytes` is the authoritative on-disk size, so it always overwrites
/// `total_bytes`. Earlier progress ticks may have written a stale value
/// — for yt-dlp DASH streams, the first stream's size (often a small
/// audio-only m4a) — and COALESCE-ing here would lock that wrong size
/// in forever.
pub(crate) async fn mark_completed(pool: &SqlitePool, id: DownloadId, bytes: u64) -> Result<()> {
    let completed_at = crate::now_iso();
    sqlx::query(
        "UPDATE downloads SET status = 'completed', downloaded_bytes = ?, \
                              total_bytes = ?, \
                              completed_at = ?, error = NULL WHERE id = ?",
    )
    .bind(bytes as i64)
    .bind(bytes as i64)
    .bind(completed_at)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark a download `failed` with an error message.
pub(crate) async fn mark_failed(pool: &SqlitePool, id: DownloadId, err: &str) -> Result<()> {
    sqlx::query("UPDATE downloads SET status = 'failed', error = ? WHERE id = ?")
        .bind(err)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Persist a fresh snapshot of progress + sidecar state. Called from the
/// queue manager on every progress tick.
pub(crate) async fn persist_progress(
    pool: &SqlitePool,
    id: DownloadId,
    downloaded: u64,
    total: Option<u64>,
    etag: Option<&str>,
    last_modified: Option<&str>,
    segments_meta: Option<&[SegmentState]>,
) -> Result<()> {
    let segments_json = match segments_meta {
        Some(s) => Some(serde_json::to_string(s)?),
        None => None,
    };
    sqlx::query(
        "UPDATE downloads SET downloaded_bytes = ?, \
                              total_bytes = COALESCE(?, total_bytes), \
                              etag = COALESCE(?, etag), \
                              last_modified = COALESCE(?, last_modified), \
                              segments_meta = COALESCE(?, segments_meta) \
         WHERE id = ?",
    )
    .bind(downloaded as i64)
    .bind(total.map(|t| t as i64))
    .bind(etag)
    .bind(last_modified)
    .bind(segments_json)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Count downloads of a given `source`, optionally restricted to rows
/// created at or after `since`. Powers the Settings → Browser status
/// card's "downloads captured this week" counter and the lifetime
/// total. Cheap — the `source` column is indexable; for the volumes
/// the app deals with no index is needed today.
pub async fn count_by_source(
    pool: &SqlitePool,
    source: DownloadSource,
    since: Option<DateTime<Utc>>,
) -> Result<u64> {
    let count: i64 = match since {
        None => {
            sqlx::query_scalar("SELECT COUNT(*) FROM downloads WHERE source = ?")
                .bind(source.as_str())
                .fetch_one(pool)
                .await?
        }
        Some(cutoff) => {
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM downloads WHERE source = ? AND created_at >= ?",
            )
            .bind(source.as_str())
            .bind(cutoff.to_rfc3339())
            .fetch_one(pool)
            .await?
        }
    };
    Ok(count.max(0) as u64)
}

/// Most recent `created_at` for a row with the given `source`, or
/// `None` when no such row exists. The Settings → Browser status card
/// renders this as a "last handoff: 4 min ago" relative timestamp.
pub async fn last_by_source(
    pool: &SqlitePool,
    source: DownloadSource,
) -> Result<Option<DateTime<Utc>>> {
    let row: Option<String> = sqlx::query_scalar(
        "SELECT created_at FROM downloads WHERE source = ? \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(source.as_str())
    .fetch_optional(pool)
    .await?;
    row.as_deref().map(parse_dt).transpose()
}

/// First queued download in priority + creation order; returns `None` if
/// none are queued.
#[allow(dead_code)]
pub(crate) async fn next_queued(pool: &SqlitePool) -> Result<Option<DownloadRecord>> {
    let row = sqlx::query(
        "SELECT * FROM downloads WHERE status = 'queued' \
         ORDER BY priority DESC, created_at ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(record_from_row).transpose()
}

/// All queued downloads in priority + creation order. Used by
/// the queue manager so the claim loop can consult the
/// `SchedulesCache` per-row without round-tripping to SQL.
pub(crate) async fn list_queued(pool: &SqlitePool) -> Result<Vec<DownloadRecord>> {
    let rows = sqlx::query(
        "SELECT * FROM downloads WHERE status = 'queued' \
         ORDER BY priority DESC, created_at ASC",
    )
    .fetch_all(pool)
    .await?;
    rows.iter().map(record_from_row).collect()
}

/// Read all ids currently in an in-flight status. The queue manager uses
/// this during a poll to reconcile against its own handle map — any
/// worker whose row is *not* in this set is presumed cancelled (by the
/// user or by an external write) and is killed.
///
/// `Muxing` is included alongside `Active` because the pump transitions
/// the row to `Muxing` mid-run when yt-dlp moves to the second stream
/// (or ffmpeg merge). Leaving it out would cause `reconcile_active` to
/// cancel the worker as soon as the row flipped status, killing yt-dlp
/// mid-mux and leaving the row stuck on Muxing with partial progress.
pub(crate) async fn active_ids(pool: &SqlitePool) -> Result<Vec<DownloadId>> {
    let rows = sqlx::query("SELECT id FROM downloads WHERE status IN ('active', 'muxing')")
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(|r| r.get("id")).collect())
}

/// Move a row from `queued` to `active` atomically. Used by the queue
/// manager when it's about to spawn a worker.
pub(crate) async fn claim(pool: &SqlitePool, id: DownloadId) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE downloads SET status = 'active', error = NULL \
         WHERE id = ? AND status = 'queued'",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}

fn record_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<DownloadRecord> {
    let status_str: String = row.get("status");
    let created_at: String = row.get("created_at");
    let completed_at: Option<String> = row.get("completed_at");
    let segments_meta_json: Option<String> = row.get("segments_meta");
    let segments_meta = match segments_meta_json {
        Some(s) => Some(serde_json::from_str(&s)?),
        None => None,
    };
    let media_info_json: Option<String> = row.try_get("media_info").ok();
    let media_info = match media_info_json {
        Some(s) if !s.is_empty() => Some(serde_json::from_str(&s)?),
        _ => None,
    };
    // `try_get` rather than `get` because `headers` was added in
    // migration 20260901000001 — older databases predating it won't have
    // the column at all (sqlx surfaces that as a column-not-found error
    // until the migration runs; treat it as `None` defensively).
    let headers_json: Option<String> = row.try_get("headers").ok().flatten();
    let headers = match headers_json {
        // Decrypt the DPAPI-protected column (legacy plaintext rows pass
        // through `unprotect` unchanged) before parsing the JSON.
        Some(s) if !s.is_empty() => Some(serde_json::from_str::<Vec<(String, String)>>(
            &crate::secret::unprotect(&s),
        )?),
        _ => None,
    };
    // Same defensive `try_get` for `source` (migration 20260902000001).
    // Pre-9c rows fall back to Manual via the NOT NULL DEFAULT 'manual'
    // applied by the migration; if the column truly doesn't exist
    // (e.g. tests pointed at an unmigrated schema) we mirror the same
    // default here so the read path never explodes.
    let source = row
        .try_get::<String, _>("source")
        .ok()
        .and_then(|s| s.parse::<DownloadSource>().ok())
        .unwrap_or(DownloadSource::Manual);
    // Defensive `try_get` for `speed_samples` (migration 20260904000001):
    // absent column / NULL / unparseable JSON all degrade to `None`.
    let speed_samples = row
        .try_get::<Option<String>, _>("speed_samples")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<Vec<u32>>(&s).ok());
    Ok(DownloadRecord {
        id: row.get("id"),
        url: row.get("url"),
        filename: row.get("filename"),
        output_path: PathBuf::from(row.get::<String, _>("output_path")),
        total_bytes: row.get::<Option<i64>, _>("total_bytes").map(|n| n as u64),
        downloaded_bytes: row.get::<i64, _>("downloaded_bytes") as u64,
        status: status_str.parse()?,
        error: row.get("error"),
        category_id: row.get("category_id"),
        priority: row.get("priority"),
        segments: row.get::<i64, _>("segments") as u32,
        created_at: parse_dt(&created_at)?,
        completed_at: completed_at.as_deref().map(parse_dt).transpose()?,
        etag: row.get("etag"),
        last_modified: row.get("last_modified"),
        segments_meta,
        media_info,
        headers,
        source,
        speed_samples,
    })
}

fn parse_dt(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| CoreError::InvalidArgument(format!("bad timestamp {s:?}: {e}")))
}

fn filename_from_url(url: &url::Url) -> Option<String> {
    let last = url.path_segments()?.next_back()?.to_string();
    if last.is_empty() {
        None
    } else {
        Some(urlencoding_decode(&last))
    }
}

/// Fast HEAD probe used at add-download time to pull a filename from
/// `Content-Disposition` / final-redirect URL / `Content-Type`. Returns
/// `None` if the probe fails, times out, or yields nothing better than
/// the URL path tail. Times out fast (5s caps) so a slow or unreachable
/// host doesn't make Add URL hang.
async fn probe_filename(pool: &SqlitePool, url: &url::Url) -> Option<String> {
    const PROBE_TIMEOUT_SECS: u64 = 5;

    let connect = crate::settings::get(pool, crate::settings::settings_keys::CONNECT_TIMEOUT_SECS)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_u64())
        .unwrap_or(15)
        .min(PROBE_TIMEOUT_SECS);
    let read = crate::settings::get(pool, crate::settings::settings_keys::READ_TIMEOUT_SECS)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_u64())
        .unwrap_or(60)
        .min(PROBE_TIMEOUT_SECS);
    let user_agent = crate::settings::get(pool, crate::settings::settings_keys::USER_AGENT)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());

    let client = engine::http::build_client(
        std::time::Duration::from_secs(connect),
        std::time::Duration::from_secs(read),
        user_agent.as_deref(),
        &[],
    )
    .ok()?;

    match engine::probe(&client, url).await {
        Ok(info) => info.filename_hint,
        Err(e) => {
            tracing::debug!(error = %e, "add_download probe failed; falling back to URL");
            None
        }
    }
}

fn urlencoding_decode(s: &str) -> String {
    // Cheap RFC 3986 percent-decode for filenames; full RFC handling lives
    // in `engine::http`. If decoding fails, return the input as-is.
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

async fn default_segments(pool: &SqlitePool) -> Result<u32> {
    let v = crate::settings::get(pool, "default_segments").await?;
    Ok(v.and_then(|v| v.as_u64()).unwrap_or(8) as u32)
}

/// Combine the category's default folder (if any) with the filename to
/// produce an absolute path. If neither category nor global default
/// supplies one, we fall back to the current working directory.
async fn resolve_output_path(
    pool: &SqlitePool,
    filename: &str,
    explicit: &Option<PathBuf>,
    category_id: Option<i64>,
) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return if path.is_absolute() || path.has_root() {
            Ok(path.clone())
        } else {
            Ok(std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path))
        };
    }

    if let Some(id) = category_id {
        if let Ok(cat) = crate::category::get(pool, id).await {
            if let Some(folder) = cat.default_output_path {
                if !folder.as_os_str().is_empty() {
                    return safe_join(&folder, filename);
                }
            }
        }
    }

    let global = crate::settings::get(pool, "default_output_path")
        .await?
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());
    if let Some(g) = global {
        return safe_join(Path::new(&g), filename);
    }

    safe_join(
        &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        filename,
    )
}

/// Join a download `filename` onto a base `folder`, guaranteeing the
/// result stays directly inside `folder`. Callers should already have run
/// `filename` through [`sanitize_filename`]; this is a defense-in-depth
/// check that rejects anything that is not a single plain file name (an
/// embedded separator, a `..` component, a drive-rooted or absolute path),
/// so a future caller that forgets to sanitize cannot reintroduce the
/// path-traversal hole.
fn safe_join(folder: &Path, filename: &str) -> Result<PathBuf> {
    let name = Path::new(filename);
    let is_plain = !filename.is_empty() && name.file_name() == Some(name.as_os_str());
    if !is_plain {
        return Err(CoreError::InvalidArgument(format!(
            "unsafe download filename {filename:?}"
        )));
    }
    Ok(folder.join(filename))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    #[test]
    fn status_round_trip() {
        for s in ALL_STATUSES {
            assert_eq!(s.as_str().parse::<Status>().unwrap(), *s);
        }
    }

    #[test]
    fn filename_from_url_basic() {
        let u: url::Url = "https://example.com/path/to/foo.zip".parse().unwrap();
        assert_eq!(filename_from_url(&u).as_deref(), Some("foo.zip"));
    }

    #[test]
    fn filename_from_url_percent_decoded() {
        let u: url::Url = "https://example.com/My%20File.zip".parse().unwrap();
        assert_eq!(filename_from_url(&u).as_deref(), Some("My File.zip"));
    }

    #[test]
    fn filename_from_root_url_is_none() {
        let u: url::Url = "https://example.com/".parse().unwrap();
        assert!(filename_from_url(&u).is_none());
    }

    #[test]
    fn sanitize_filename_strips_path_separators() {
        // Traversal attempts collapse to a single, separator-free name.
        assert_eq!(sanitize_filename("../../etc/passwd"), ".._.._etc_passwd");
        assert_eq!(sanitize_filename("/etc/passwd"), "_etc_passwd");
        assert_eq!(sanitize_filename("subdir/file.txt"), "subdir_file.txt");
        assert_eq!(
            sanitize_filename(r"C:\Windows\System32\cmd.exe"),
            "C__Windows_System32_cmd.exe"
        );
        // Pure-dot names and empties degrade to a safe fallback.
        assert_eq!(sanitize_filename(".."), "download");
        assert_eq!(sanitize_filename("   "), "download");
        // None of the outputs contain a separator or are absolute.
        for input in ["../x", "/x", r"\\server\share\x", "a/b/c"] {
            let out = sanitize_filename(input);
            assert!(!out.contains('/') && !out.contains('\\'), "{out:?}");
            assert_eq!(
                Path::new(&out).file_name(),
                Some(std::ffi::OsStr::new(&out))
            );
        }
    }

    #[test]
    fn safe_join_rejects_escaping_filenames() {
        let base = Path::new("/downloads");
        // A plain sanitized name joins cleanly inside the base folder.
        assert_eq!(
            safe_join(base, "movie.mp4").unwrap(),
            base.join("movie.mp4")
        );
        // Anything with a separator, parent ref, or root is rejected.
        for bad in ["../escape", "sub/dir", "/etc/passwd", ""] {
            assert!(
                safe_join(base, bad).is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }

    async fn fresh_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    /// Insert a download row directly, bypassing the higher-level
    /// `insert()` path so the test can control the initial filename and
    /// category exactly (mirroring what the yt-dlp insert path produces:
    /// an extensionless stem categorized as "Other").
    async fn seed_row(
        pool: &SqlitePool,
        filename: &str,
        category_id: Option<i64>,
        total_bytes: Option<i64>,
    ) -> i64 {
        let row = sqlx::query(
            "INSERT INTO downloads (url, filename, output_path, total_bytes, downloaded_bytes, \
                                    status, category_id, priority, segments, created_at) \
             VALUES ('https://example.com/x', ?, ?, ?, 0, 'active', ?, 0, 1, '2026-01-01T00:00:00Z') \
             RETURNING id",
        )
        .bind(filename)
        .bind(filename)
        .bind(total_bytes)
        .bind(category_id)
        .fetch_one(pool)
        .await
        .unwrap();
        row.get("id")
    }

    #[tokio::test]
    async fn finalize_recategorizes_auto_assigned_row() {
        let pool = fresh_pool().await;
        let other = crate::category::find_by_name(&pool, "Other")
            .await
            .unwrap()
            .unwrap();
        let video = crate::category::find_by_name(&pool, "Video")
            .await
            .unwrap()
            .unwrap();

        let id = seed_row(&pool, "SECSUN Boom Lift", Some(other.id), Some(189_600)).await;

        let new_path = std::path::PathBuf::from("/tmp/SECSUN Boom Lift.mp4");
        // The file doesn't physically exist here, so the move is skipped and
        // the path is unchanged; we only assert the recategorization.
        let (_final_path, changed) =
            finalize_ytdlp_completion(&pool, id, "SECSUN Boom Lift.mp4", &new_path, 16_900_000)
                .await
                .unwrap();

        assert_eq!(changed, Some(video.id));

        let row = sqlx::query(
            "SELECT filename, output_path, total_bytes, category_id FROM downloads WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let filename: String = row.get("filename");
        let total_bytes: i64 = row.get("total_bytes");
        let category_id: i64 = row.get("category_id");
        assert_eq!(filename, "SECSUN Boom Lift.mp4");
        assert_eq!(total_bytes, 16_900_000);
        assert_eq!(category_id, video.id);
    }

    #[tokio::test]
    async fn finalize_preserves_user_selected_category() {
        let pool = fresh_pool().await;
        let docs = crate::category::find_by_name(&pool, "Documents")
            .await
            .unwrap()
            .unwrap();

        // User explicitly picked "Documents" — neither original nor new
        // filename would auto-route there, so the row must stay put.
        let id = seed_row(&pool, "weird name", Some(docs.id), None).await;

        let new_path = std::path::PathBuf::from("/tmp/weird name.mp4");
        let (_final_path, changed) =
            finalize_ytdlp_completion(&pool, id, "weird name.mp4", &new_path, 12_345)
                .await
                .unwrap();

        assert_eq!(changed, None);
        let category_id: i64 = sqlx::query_scalar("SELECT category_id FROM downloads WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(category_id, docs.id);
    }

    #[tokio::test]
    async fn move_into_folder_relocates_and_dedupes() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("Other");
        let dst_dir = tmp.path().join("Video");
        tokio::fs::create_dir_all(&src_dir).await.unwrap();

        let src = src_dir.join("clip.mp4");
        tokio::fs::write(&src, b"hello").await.unwrap();
        let moved = move_into_folder(&src, &dst_dir).await;
        assert_eq!(moved, dst_dir.join("clip.mp4"));
        assert!(tokio::fs::metadata(&moved).await.is_ok());
        assert!(tokio::fs::metadata(&src).await.is_err(), "original removed");

        // A second file of the same name de-duplicates rather than clobbering.
        let src2 = src_dir.join("clip.mp4");
        tokio::fs::write(&src2, b"world").await.unwrap();
        let moved2 = move_into_folder(&src2, &dst_dir).await;
        assert_eq!(moved2, dst_dir.join("clip (1).mp4"));
        assert!(tokio::fs::metadata(&moved2).await.is_ok());
    }

    #[tokio::test]
    async fn add_download_round_trips_headers() {
        // A native-host hand-off arrives with captured
        // Cookie + Referer. They must persist on the row and come back
        // out when the queue worker reads it.
        let pool = fresh_pool().await;
        let headers = vec![
            ("Cookie".to_string(), "session=abc".to_string()),
            ("Referer".to_string(), "https://example.com/".to_string()),
            ("X-Custom".to_string(), "captured".to_string()),
        ];
        let input = AddDownload {
            url: "https://example.com/file.zip".parse().unwrap(),
            filename: Some("file.zip".to_string()),
            output_path: Some(std::path::PathBuf::from("/tmp/file.zip")),
            category: None,
            priority: 0,
            segments: Some(4),
            media_info: None,
            headers: Some(headers.clone()),
            source: DownloadSource::ExtensionPipe,
        };
        let rec = insert(&pool, input).await.unwrap();
        let again = get(&pool, rec.id).await.unwrap();
        assert_eq!(again.headers.as_deref(), Some(headers.as_slice()));
        assert_eq!(again.source, DownloadSource::ExtensionPipe);
    }

    #[tokio::test]
    async fn add_download_without_headers_yields_none() {
        let pool = fresh_pool().await;
        let input = AddDownload {
            url: "https://example.com/file.zip".parse().unwrap(),
            filename: Some("file.zip".to_string()),
            output_path: Some(std::path::PathBuf::from("/tmp/x.zip")),
            category: None,
            priority: 0,
            segments: Some(1),
            media_info: None,
            headers: None,
            source: DownloadSource::Manual,
        };
        let rec = insert(&pool, input).await.unwrap();
        assert!(rec.headers.is_none());
        assert_eq!(rec.source, DownloadSource::Manual);
    }

    #[tokio::test]
    async fn count_by_source_filters_by_provenance_and_window() {
        // Seed three rows — one ExtensionPipe today, one
        // ExtensionPipe 30 days ago, one Manual today. The weekly
        // counter must report 1 hit (the recent ExtensionPipe row); the
        // lifetime total must be 2; the Manual provenance must be
        // independent.
        let pool = fresh_pool().await;

        // Recent ExtensionPipe row.
        sqlx::query(
            "INSERT INTO downloads (url, filename, output_path, total_bytes, downloaded_bytes, \
                                    status, category_id, priority, segments, created_at, source) \
             VALUES ('https://x/recent', 'recent.zip', '/tmp/recent.zip', NULL, 0, 'completed', \
                     NULL, 0, 1, ?, 'extension_pipe')",
        )
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();

        // Old ExtensionPipe row — 30 days back.
        let thirty_days_ago = chrono::Utc::now() - chrono::Duration::days(30);
        sqlx::query(
            "INSERT INTO downloads (url, filename, output_path, total_bytes, downloaded_bytes, \
                                    status, category_id, priority, segments, created_at, source) \
             VALUES ('https://x/old', 'old.zip', '/tmp/old.zip', NULL, 0, 'completed', \
                     NULL, 0, 1, ?, 'extension_pipe')",
        )
        .bind(thirty_days_ago.to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();

        // Manual row today; must NOT contribute to ExtensionPipe counts.
        sqlx::query(
            "INSERT INTO downloads (url, filename, output_path, total_bytes, downloaded_bytes, \
                                    status, category_id, priority, segments, created_at, source) \
             VALUES ('https://x/manual', 'manual.zip', '/tmp/manual.zip', NULL, 0, 'completed', \
                     NULL, 0, 1, ?, 'manual')",
        )
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();

        let week_cutoff = chrono::Utc::now() - chrono::Duration::days(7);
        let weekly = count_by_source(&pool, DownloadSource::ExtensionPipe, Some(week_cutoff))
            .await
            .unwrap();
        let total = count_by_source(&pool, DownloadSource::ExtensionPipe, None)
            .await
            .unwrap();
        let manual_total = count_by_source(&pool, DownloadSource::Manual, None)
            .await
            .unwrap();
        assert_eq!(weekly, 1, "only the recent ExtensionPipe row is inside 7d");
        assert_eq!(total, 2, "both ExtensionPipe rows count toward lifetime");
        assert_eq!(manual_total, 1, "Manual provenance is independent");
    }

    #[tokio::test]
    async fn last_by_source_returns_most_recent_only() {
        let pool = fresh_pool().await;
        // Two ExtensionPipe rows; assert the newer timestamp wins.
        let older = chrono::Utc::now() - chrono::Duration::hours(2);
        let newer = chrono::Utc::now() - chrono::Duration::minutes(5);
        for ts in [older, newer] {
            sqlx::query(
                "INSERT INTO downloads (url, filename, output_path, total_bytes, downloaded_bytes, \
                                        status, category_id, priority, segments, created_at, source) \
                 VALUES ('https://x/y', 'y.zip', '/tmp/y.zip', NULL, 0, 'completed', \
                         NULL, 0, 1, ?, 'extension_pipe')",
            )
            .bind(ts.to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        }

        let got = last_by_source(&pool, DownloadSource::ExtensionPipe)
            .await
            .unwrap()
            .expect("two rows seeded");
        // Tolerate sub-second formatting noise — equality at second
        // resolution is enough for the assertion.
        assert_eq!(got.timestamp(), newer.timestamp());

        // No CLI rows seeded → None.
        let cli = last_by_source(&pool, DownloadSource::Cli).await.unwrap();
        assert!(cli.is_none());
    }

    #[tokio::test]
    async fn fresh_db_migration_yields_zero_handoffs() {
        // Migration applies cleanly on a fresh database (no rows) and
        // the new column is queryable from byte one.
        let pool = fresh_pool().await;
        let total = count_by_source(&pool, DownloadSource::ExtensionPipe, None)
            .await
            .unwrap();
        assert_eq!(total, 0);
        let last = last_by_source(&pool, DownloadSource::ExtensionPipe)
            .await
            .unwrap();
        assert!(last.is_none());
    }

    #[test]
    fn download_source_round_trip() {
        for src in [
            DownloadSource::Manual,
            DownloadSource::ExtensionPipe,
            DownloadSource::Cli,
        ] {
            assert_eq!(src.as_str().parse::<DownloadSource>().unwrap(), src);
        }
    }

    #[tokio::test]
    async fn mark_completed_overwrites_stale_total_bytes() {
        // Bug 2: a stream-1 total (e.g. audio-only m4a at 189 KB) gets
        // written first; the actually-completed `bytes` (16.9 MB) must
        // overwrite it, not be discarded by COALESCE.
        let pool = fresh_pool().await;
        let id = seed_row(&pool, "x.mp4", None, Some(189_600)).await;

        mark_completed(&pool, id, 16_900_000).await.unwrap();

        let total: i64 = sqlx::query_scalar("SELECT total_bytes FROM downloads WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(total, 16_900_000);
    }
}
