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

/// Which backend runs a download. Persisted to the `downloads.kind`
/// column (migration `20260905000001_downloads_torrent`) as the explicit
/// discriminator the queue worker branches on, replacing the older
/// implicit `media_info.is_some()` check. Mirrors [`DownloadSource`] in
/// shape (derives, snake_case serde, `as_str`/`Display`/`FromStr`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DownloadKind {
    /// Multi-segment HTTP/HTTPS via the [`engine`] crate.
    #[default]
    Http,
    /// yt-dlp subprocess flow (`crate::ytdlp`).
    Media,
    /// BitTorrent via the `crates/torrent` facade over librqbit.
    Torrent,
}

impl DownloadKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DownloadKind::Http => "http",
            DownloadKind::Media => "media",
            DownloadKind::Torrent => "torrent",
        }
    }
}

impl std::fmt::Display for DownloadKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DownloadKind {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "http" => DownloadKind::Http,
            "media" => DownloadKind::Media,
            "torrent" => DownloadKind::Torrent,
            other => {
                return Err(CoreError::InvalidArgument(format!(
                    "unknown download kind: {other}"
                )))
            }
        })
    }
}

/// Persisted torrent state for a [`DownloadKind::Torrent`] row. Stored as
/// one nullable JSON column on `downloads.torrent`, exactly like
/// `media_info` / `headers`. The DB blob holds only logical state; the
/// librqbit piece bitfield / fastresume lives in a managed dir keyed by
/// `info_hash`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TorrentMeta {
    /// Lowercase hex info-hash — the stable de-dup key (from the magnet's
    /// `xt=urn:btih:` or hashed from `.torrent` bytes).
    pub info_hash: String,
    pub source: TorrentSource,
    /// `None` = download all files; otherwise the librqbit `only_files`
    /// selection (file indices into `files`).
    pub selected_files: Option<Vec<usize>>,
    /// Filled once librqbit resolves metadata.
    pub files: Option<Vec<TorrentFile>>,
    /// Last swarm snapshot; survives relaunch so the UI can render
    /// peers/seeds before the session re-attaches.
    pub swarm: Option<SwarmStats>,
}

/// Where a torrent came from. The `.torrent` bytes are copied into the
/// managed dir at add time and referenced by path (not stored inline);
/// magnets store the URI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TorrentSource {
    Magnet { uri: String },
    File { path: PathBuf },
    InfoHash { hash: String },
}

/// One file inside a torrent, as exposed to the add-time picker and the
/// detail-pane per-file progress list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TorrentFile {
    pub index: usize,
    pub path: String,
    pub length: u64,
    pub selected: bool,
}

/// Last swarm snapshot persisted into the row's `torrent` JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwarmStats {
    pub peers: u32,
    pub seeds: u32,
    pub up_bps: u64,
    pub down_bps: u64,
    pub ratio_milli: u32,
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
    /// Which backend runs this download. The queue worker branches on it
    /// (`http` → engine, `media` → yt-dlp, `torrent` → librqbit facade).
    /// Older rows land as `Http` per the NOT NULL DEFAULT applied by
    /// `20260905000001_downloads_torrent.sql` (the backfill upgrades
    /// yt-dlp rows to `Media`).
    #[serde(default)]
    pub kind: DownloadKind,
    /// Persisted torrent state when `kind == Torrent`. `None` for HTTP /
    /// media rows and rows predating the torrent migration.
    #[serde(default)]
    pub torrent: Option<TorrentMeta>,
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
    /// Which backend runs this download. HTTP/media callers pass `Http` /
    /// `Media`; torrent callers pass `Torrent` and populate `torrent`.
    pub kind: DownloadKind,
    /// Torrent state when `kind == Torrent`. `None` otherwise.
    pub torrent: Option<TorrentMeta>,
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
        kind,
        torrent,
    } = input;

    // Normalize the discriminator: a row carrying yt-dlp `media_info` is
    // always `Media`, and one carrying `torrent` state is always
    // `Torrent`, regardless of what the caller passed. This keeps the
    // explicit `kind` column in lock-step with the JSON-blob columns the
    // worker branches on, so the two can never disagree.
    let kind = if media_info.is_some() {
        DownloadKind::Media
    } else if torrent.is_some() {
        DownloadKind::Torrent
    } else {
        kind
    };

    // Normalize the torrent meta before anything else: ensure the
    // `info_hash` is lowercase hex (deriving it from a magnet's
    // `xt=urn:btih:` when the caller left it blank), because Q7 de-dup and
    // the managed-state-dir key both rely on a canonical hash.
    let torrent = match torrent {
        Some(mut meta) => {
            normalize_torrent_meta(&mut meta, &url);
            Some(meta)
        }
        None => None,
    };

    // Q7 front-door de-dup: a second add of the same swarm (matched on the
    // canonical `info_hash`) is a NO-OP that hands back the existing row,
    // never a new one — so the UI never shows two rows for one swarm. Only
    // checked against rows that are still meaningful (not removed/cancelled
    // — a cancelled row can be retried, so it counts; a separate explicit
    // re-add of a removed torrent legitimately makes a fresh row because
    // the old row is gone). See design §5.7.
    if let Some(meta) = torrent.as_ref() {
        if !meta.info_hash.is_empty() {
            if let Some(existing) = find_active_torrent_by_hash(pool, &meta.info_hash).await? {
                tracing::info!(
                    info_hash = %meta.info_hash,
                    existing_id = existing.id,
                    "add_download: duplicate torrent — returning existing row"
                );
                return Ok(existing);
            }
        }
    }

    // Provisional name. Torrents skip the HEAD probe entirely (there is no
    // HTTP resource to probe) and take a provisional name now — magnet
    // `dn=` → `.torrent` stem → `"torrent"` — reconciled to the real
    // torrent name once librqbit resolves metadata (the facade emits
    // `FilenameLearned`, mirroring `finalize_ytdlp_completion`). yt-dlp rows
    // bring their own title and also skip the probe; plain HTTP rows pre-probe
    // the URL so randomized URLs like `/d/abc123xyz` don't save as
    // extension-less garbage.
    let filename = match (filename, kind, media_info.as_ref(), torrent.as_ref()) {
        (Some(f), ..) => f,
        (None, DownloadKind::Torrent, _, torrent) => provisional_torrent_name(torrent, &url),
        (None, _, Some(info), _) => sanitize_filename(&info.title),
        (None, ..) => probe_filename(pool, &url)
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

    // HTTP / media rows resolve to a single FILE path. Torrents resolve to
    // a content-root DIRECTORY: librqbit writes the torrent's file(s)
    // directly under the `output_folder` we hand it, so the row's
    // `output_path` must be that directory — keyed per-infohash so two
    // torrents never collide and Q4's directory-aware `remove_dir_all`
    // (download::remove) can delete the whole tree.
    let resolved_output = match kind {
        DownloadKind::Torrent => {
            resolve_torrent_output_dir(pool, &filename, &output_path, torrent.as_ref()).await?
        }
        _ => resolve_output_path(pool, &filename, &output_path, category_id).await?,
    };
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
    // Torrent state rides one nullable JSON column, exactly like
    // `media_info` / `headers`.
    let torrent_json = match torrent.as_ref() {
        Some(meta) => Some(serde_json::to_string(meta)?),
        None => None,
    };
    let row = sqlx::query(
        "INSERT INTO downloads (\
            url, filename, output_path, total_bytes, downloaded_bytes, status, \
            category_id, priority, segments, created_at, media_info, headers, source, \
            kind, torrent) \
         VALUES (?, ?, ?, NULL, 0, 'queued', ?, ?, ?, ?, ?, ?, ?, ?, ?) \
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
    .bind(kind.as_str())
    .bind(torrent_json)
    .fetch_one(pool)
    .await?;

    let id: i64 = row.get("id");
    get(pool, id).await
}

/// Lowercase the canonical `info_hash` and, when the caller left it blank,
/// derive it from a magnet `xt=urn:btih:` (no metadata fetch needed — Q7).
/// Bare-infohash sources copy their hash in. Leaves it empty only when no
/// hash can be recovered (e.g. a `.torrent` file whose bytes haven't been
/// hashed yet — the add-dialog path computes that hash before calling in).
fn normalize_torrent_meta(meta: &mut TorrentMeta, url: &url::Url) {
    if meta.info_hash.is_empty() {
        if let TorrentSource::Magnet { uri } = &meta.source {
            if let Some(h) = info_hash_from_magnet(uri) {
                meta.info_hash = h;
            }
        } else if let TorrentSource::InfoHash { hash } = &meta.source {
            meta.info_hash = hash.clone();
        }
    }
    // The `url` column for a torrent row carries the magnet URI; try it too
    // as a last resort so a caller that only set `url` still de-dups.
    if meta.info_hash.is_empty() {
        if let Some(h) = info_hash_from_magnet(url.as_str()) {
            meta.info_hash = h;
        }
    }
    meta.info_hash = meta.info_hash.trim().to_ascii_lowercase();
}

/// Extract the BitTorrent v1 info-hash from a magnet URI's
/// `xt=urn:btih:<hash>` parameter, normalized to lowercase hex. Accepts the
/// 40-char hex form; returns `None` for the (rarer) base32 form or when the
/// parameter is absent — callers fall back to other hash sources. No
/// network / metadata fetch is involved (design §5.7).
fn info_hash_from_magnet(uri: &str) -> Option<String> {
    // Magnets are not always valid `Url`s for `url::Url`, but the query is a
    // simple `&`-joined list of `key=value`s after the first `?`.
    let query = uri.split_once('?').map(|(_, q)| q).unwrap_or(uri);
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=')?;
        if !k.eq_ignore_ascii_case("xt") {
            continue;
        }
        // urn:btih:<hash> (case-insensitive scheme).
        let lower = v.to_ascii_lowercase();
        if let Some(hash) = lower.strip_prefix("urn:btih:") {
            if hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Some(hash.to_string());
            }
        }
    }
    None
}

/// Provisional display name for a torrent before librqbit resolves
/// metadata: magnet `dn=` → `.torrent` file stem → `"torrent"` (design
/// §3.B). Reconciled to the real torrent name on the first
/// `FilenameLearned` event (see [`reconcile_torrent_filename`]).
fn provisional_torrent_name(torrent: Option<&TorrentMeta>, url: &url::Url) -> String {
    if let Some(meta) = torrent {
        match &meta.source {
            TorrentSource::Magnet { uri } => {
                if let Some(name) = magnet_display_name(uri) {
                    return name;
                }
            }
            TorrentSource::File { path } => {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if !stem.is_empty() {
                        return stem.to_string();
                    }
                }
            }
            TorrentSource::InfoHash { .. } => {}
        }
    }
    // Fall back to the `url` column (the magnet URI for magnet rows).
    if let Some(name) = magnet_display_name(url.as_str()) {
        return name;
    }
    "torrent".to_string()
}

/// Pull the `dn=` (display name) parameter out of a magnet URI, percent-decoded.
fn magnet_display_name(uri: &str) -> Option<String> {
    let query = uri.split_once('?').map(|(_, q)| q).unwrap_or(uri);
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=')?;
        if k.eq_ignore_ascii_case("dn") {
            // Magnet `dn=` uses `+` for spaces in addition to percent-encoding.
            let decoded = urlencoding_decode(&v.replace('+', " "));
            let trimmed = decoded.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Find a non-terminal download row carrying the given torrent `info_hash`,
/// for Q7 front-door de-dup. A `Removed` row is gone from the table, so it
/// never matches; a `Cancelled`/`Failed` row still matches (the user can
/// retry it, and a re-add should resume that row, not spawn a twin).
async fn find_active_torrent_by_hash(
    pool: &SqlitePool,
    info_hash: &str,
) -> Result<Option<DownloadRecord>> {
    // The `info_hash` lives inside the `torrent` JSON blob, so there is no
    // indexable column to query directly. Torrent rows are rare relative to
    // the table, so scan the kind='torrent' rows and compare in Rust. The
    // JSON `"info_hash":"<hash>"` substring is a cheap pre-filter.
    let needle = format!("\"info_hash\":\"{info_hash}\"");
    let rows = sqlx::query("SELECT * FROM downloads WHERE kind = 'torrent' AND torrent LIKE ?")
        .bind(format!("%{needle}%"))
        .fetch_all(pool)
        .await?;
    for row in &rows {
        let record = record_from_row(row)?;
        if record
            .torrent
            .as_ref()
            .is_some_and(|t| t.info_hash == info_hash)
        {
            return Ok(Some(record));
        }
    }
    Ok(None)
}

/// Resolve the content-root DIRECTORY a torrent row downloads into. Unlike
/// HTTP/media rows (a single file), librqbit writes the torrent's file(s)
/// directly under one `output_folder`, so the row's `output_path` is that
/// folder. We key it per-infohash under the configured base so two torrents
/// never share a directory and Q4's `remove_dir_all` keys cleanly off it.
async fn resolve_torrent_output_dir(
    pool: &SqlitePool,
    filename: &str,
    explicit: &Option<PathBuf>,
    torrent: Option<&TorrentMeta>,
) -> Result<PathBuf> {
    // An explicit path wins verbatim (the add-dialog / extension may target a
    // chosen folder); make it absolute the same way `resolve_output_path` does.
    if let Some(path) = explicit {
        return Ok(if path.is_absolute() || path.has_root() {
            path.clone()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        });
    }

    // Base folder: the torrent-specific download dir, else the global default,
    // else the current dir — mirroring `category_target_folder`'s fallback
    // chain but rooted at the `torrent_download_dir` setting (design §3.G).
    let base = torrent_base_dir(pool).await?;
    // Per-download subdir name: the sanitized provisional/real name, suffixed
    // with the infohash so distinct torrents that happen to share a name stay
    // separate. Fall back to just the infohash when there's no usable name.
    let hash = torrent.map(|t| t.info_hash.as_str()).unwrap_or("");
    let subdir = match (filename.is_empty(), hash.is_empty()) {
        (false, false) => format!("{filename}.{}", &hash[..hash.len().min(12)]),
        (false, true) => filename.to_string(),
        (true, false) => hash.to_string(),
        (true, true) => "torrent".to_string(),
    };
    safe_join(&base, &sanitize_filename(&subdir))
}

/// Base folder torrents download into: `torrent_download_dir` setting, else
/// the global `default_output_path`, else the current working directory.
async fn torrent_base_dir(pool: &SqlitePool) -> Result<PathBuf> {
    let torrent_dir = crate::settings::get(pool, "torrent_download_dir")
        .await?
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());
    if let Some(d) = torrent_dir {
        return Ok(PathBuf::from(d));
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

/// Reconcile a torrent row's DISPLAY metadata once librqbit resolves the real
/// torrent name (delivered via `engine::ProgressEvent::FilenameLearned`,
/// mirroring `finalize_ytdlp_completion`). Unlike the HTTP/yt-dlp paths this
/// must NOT move anything on disk: the content root is a directory librqbit is
/// actively writing into, and renaming it mid-flight would break the live
/// handle. We update only the row's `filename` + (re-routed) `category_id`.
///
/// Re-categorization preserves a deliberate user choice exactly like
/// `finalize_ytdlp_completion`: only re-route when the stored category is the
/// one auto-routing for the *old* name would have picked. Returns the new
/// (filename, category) when anything changed, else `None` (idempotent on
/// event re-emission).
pub(crate) async fn reconcile_torrent_filename(
    pool: &SqlitePool,
    id: DownloadId,
    hint: &str,
) -> Result<Option<(String, Option<crate::category::CategoryId>, bool)>> {
    let new_name = sanitize_filename(hint);
    if new_name.is_empty() {
        return Ok(None);
    }
    let row = sqlx::query("SELECT filename, category_id FROM downloads WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::DownloadNotFound(id))?;
    let current_name: String = row.get("filename");
    let current_category: Option<i64> = row.get("category_id");

    if current_name == new_name {
        // Display already reflects the resolved name (event re-emitted).
        return Ok(None);
    }

    let auto_for_current = auto_categorize_for_filename(pool, &current_name).await?;
    let new_category = if current_category == auto_for_current {
        auto_categorize_for_filename(pool, &new_name).await?
    } else {
        current_category
    };
    let category_changed = new_category != current_category;

    sqlx::query("UPDATE downloads SET filename = ?, category_id = ? WHERE id = ?")
        .bind(&new_name)
        .bind(new_category)
        .bind(id)
        .execute(pool)
        .await?;

    Ok(Some((new_name, new_category, category_changed)))
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
        // Multi-file torrents create a *content folder* (the row's
        // `output_path` is the content root), so they need a recursive
        // delete; HTTP / media rows are a single file. Best-effort either
        // way: report failure on the row, but don't roll back the DB
        // delete — the user asked to remove the row.
        let delete_result = if record.kind == DownloadKind::Torrent {
            tokio::fs::remove_dir_all(&record.output_path).await
        } else {
            tokio::fs::remove_file(&record.output_path).await
        };
        match delete_result {
            Ok(()) => outcome.data_deleted = true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                outcome.data_deleted = true;
            }
            Err(e) => outcome.data_error = Some(e.to_string()),
        }
        if record.kind == DownloadKind::Torrent {
            // librqbit's JSON persistence store keeps fastresume state as flat
            // files in the shared state root (`<app_data>/torrents/`):
            // `<info_hash>.bitv` (piece bitfield) + `<info_hash>.torrent`
            // (metainfo), next to a shared `session.json`. Remove only THIS
            // torrent's two files so its bitfield doesn't leak — never the
            // whole dir, which would drop every other torrent's state and the
            // session list. Best-effort: ignore errors and a not-yet-resolved
            // (empty) info_hash (skipping it also avoids `join("")` resolving
            // back to the state root).
            if let Some(info_hash) = record
                .torrent
                .as_ref()
                .map(|t| t.info_hash.as_str())
                .filter(|h| !h.is_empty())
            {
                if let Some(root) = torrent_state_root() {
                    let _ = tokio::fs::remove_file(root.join(format!("{info_hash}.bitv"))).await;
                    let _ =
                        tokio::fs::remove_file(root.join(format!("{info_hash}.torrent"))).await;
                }
            }
        } else {
            // Sidecar is engine-only state; cleaning it up here keeps stale
            // files from accumulating, but we ignore errors.
            let sidecar = engine::Meta::sidecar_path(&record.output_path);
            let _ = tokio::fs::remove_file(&sidecar).await;
        }
    }
    Ok(outcome)
}

/// Root dir where librqbit persists per-torrent fastresume state — the
/// `state_dir` handed to the facade in `queue::build_torrent_config`:
/// `<app_data>/torrents`. librqbit's `JsonSessionPersistenceStore` writes flat
/// files here (`<info_hash>.bitv`, `<info_hash>.torrent`) plus a shared
/// `session.json`; it does NOT create a per-infohash subdir. `None` when no
/// app-data root is resolvable.
pub fn torrent_state_root() -> Option<PathBuf> {
    crate::directories_root().map(|d| d.join("torrents"))
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

/// Outcome of [`apply_engine_filename`] when a rename/relocate happened.
pub(crate) struct RenamedDownload {
    pub filename: String,
    pub path: PathBuf,
    pub category_id: Option<crate::category::CategoryId>,
    pub category_changed: bool,
}

/// Outcome of [`mark_learned_filename`] when a row's *display* metadata
/// changed mid-download. The on-disk file is left untouched (still at
/// `output_path`); the physical move is deferred to completion.
pub(crate) struct LearnedFilename {
    pub filename: String,
    pub output_path: PathBuf,
    pub category_id: Option<crate::category::CategoryId>,
    pub category_changed: bool,
}

/// What a learned engine filename would assign to a row, after the
/// "is this our slug?" guard. Shared by the mid-flight display update
/// ([`mark_learned_filename`]) and the completion move
/// ([`apply_engine_filename`]) so their guards can never drift apart.
struct LearnedDecision {
    /// Sanitized learned name.
    new_name: String,
    /// The row's current display filename (may already be the learned name
    /// when this is the completion call following a mid-flight update).
    current_display_name: String,
    current_path: PathBuf,
    current_category: Option<i64>,
    new_category: Option<i64>,
}

/// Resolve what a learned `hint` implies for download `id`, or `None` when
/// the row must be left alone.
///
/// One-click / single-use-token hosts (fuckingfast.co, pixeldrain) often
/// reveal the real name only on the GET that fetches the bytes — too late
/// for the add-time HEAD probe, which had to fall back to the random URL-path
/// slug. The engine surfaces that name (on `ProgressEvent::FilenameLearned`
/// mid-flight and `DownloadSummary::filename_hint` at completion); this
/// computes the resulting name + category once for both.
///
/// Conservative by construction: it ignores hints that are themselves random
/// slugs (no improvement), and only acts when the file's *on-disk* name is
/// exactly the sanitized URL-path tail we ourselves saved — never a
/// `Content-Disposition` name found at add time or a name the user typed. The
/// on-disk name (not the display name) is the stable key, because the display
/// name may already have been swapped to the learned name by a mid-flight
/// update while the file still sits at its slug path.
async fn decide_learned(
    pool: &SqlitePool,
    id: DownloadId,
    url: &url::Url,
    hint: &str,
) -> Result<Option<LearnedDecision>> {
    let new_name = sanitize_filename(hint);
    if engine::filename::is_random_slug(&new_name) {
        return Ok(None);
    }

    let row = sqlx::query("SELECT filename, output_path, category_id FROM downloads WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(CoreError::DownloadNotFound(id))?;
    let current_display_name: String = row.get("filename");
    let current_path = PathBuf::from(row.get::<String, _>("output_path"));
    let current_category: Option<i64> = row.get("category_id");

    let physical_name = current_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    // The on-disk file already carries the learned name — no rename, and no
    // relocation either (the file may sit at a caller-chosen explicit path
    // that we must not move when there's nothing to improve).
    if new_name == physical_name {
        return Ok(None);
    }
    let url_fallback = filename_from_url(url).map(|t| sanitize_filename(&t));
    if url_fallback.as_deref() != Some(physical_name.as_str()) {
        return Ok(None);
    }

    // Re-categorize only when the stored category is the one auto-routing for
    // the slug would have picked — i.e. it wasn't a manual choice (nor an
    // already-applied mid-flight update, which we treat the same: leave it).
    let auto_for_slug = auto_categorize_for_filename(pool, &physical_name).await?;
    let new_category = if current_category == auto_for_slug {
        auto_categorize_for_filename(pool, &new_name).await?
    } else {
        current_category
    };

    Ok(Some(LearnedDecision {
        new_name,
        current_display_name,
        current_path,
        current_category,
        new_category,
    }))
}

/// Apply a filename the engine learned from the *response headers* to a row
/// that is still downloading: update the row's display name and category so
/// the UI shows the real name (and correct group) immediately, instead of the
/// random URL slug it was saved under.
///
/// The file on disk is **not** moved — the engine still owns and is writing
/// it; the physical rename/relocate happens at completion via
/// [`apply_engine_filename`], which keys off the unchanged on-disk path.
/// Returns `Ok(None)` when nothing changed (slug hint, a name we shouldn't
/// override, or the display name already reflects the hint).
pub(crate) async fn mark_learned_filename(
    pool: &SqlitePool,
    id: DownloadId,
    url: &url::Url,
    hint: &str,
) -> Result<Option<LearnedFilename>> {
    let Some(decision) = decide_learned(pool, id, url, hint).await? else {
        return Ok(None);
    };
    // Idempotent: the display name already reflects the hint (event re-emitted
    // on resume / restart). Nothing to update or announce.
    if decision.current_display_name == decision.new_name {
        return Ok(None);
    }

    // Display metadata only — `output_path` is deliberately left pointing at
    // the working (slug) file the engine is still writing.
    sqlx::query("UPDATE downloads SET filename = ?, category_id = ? WHERE id = ?")
        .bind(&decision.new_name)
        .bind(decision.new_category)
        .bind(id)
        .execute(pool)
        .await?;

    Ok(Some(LearnedFilename {
        filename: decision.new_name,
        output_path: decision.current_path,
        category_id: decision.new_category,
        category_changed: decision.new_category != decision.current_category,
    }))
}

/// Apply a learned filename to a freshly-completed engine download: rename the
/// on-disk file and reconcile its category, relocating it into the category's
/// folder so the on-disk location agrees with the UI grouping.
///
/// Runs whether or not [`mark_learned_filename`] already updated the row's
/// display metadata mid-flight — the file itself is still at its slug path, so
/// this is what actually moves the bytes. See [`decide_learned`] for the
/// guard. Returns `Ok(None)` when nothing changed.
pub(crate) async fn apply_engine_filename(
    pool: &SqlitePool,
    id: DownloadId,
    url: &url::Url,
    hint: &str,
) -> Result<Option<RenamedDownload>> {
    let Some(decision) = decide_learned(pool, id, url, hint).await? else {
        return Ok(None);
    };
    let LearnedDecision {
        new_name,
        current_path,
        current_category,
        new_category,
        ..
    } = decision;

    // Relocate the working (slug-named) file to its final home: the category's
    // folder + the learned name. `category_target_folder` mirrors the add-time
    // `resolve_output_path`, so when the category is unchanged it resolves to
    // the folder the file already lives in and the move degrades to an
    // in-folder rename. Best-effort: on any IO failure `move_renamed` returns
    // `current_path` unchanged, so we leave the DB pointing at the real file
    // rather than a path that was never produced.
    let dest_folder = category_target_folder(pool, new_category).await?;
    let dest = move_renamed(&current_path, &dest_folder, std::ffi::OsStr::new(&new_name)).await;
    if dest == current_path {
        return Ok(None);
    }
    let final_name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&new_name)
        .to_string();

    sqlx::query("UPDATE downloads SET filename = ?, output_path = ?, category_id = ? WHERE id = ?")
        .bind(&final_name)
        .bind(dest.to_string_lossy().as_ref())
        .bind(new_category)
        .bind(id)
        .execute(pool)
        .await?;

    Ok(Some(RenamedDownload {
        filename: final_name,
        path: dest,
        category_id: new_category,
        category_changed: new_category != current_category,
    }))
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
    match from.file_name() {
        Some(name) => move_renamed(from, dest_dir, name).await,
        None => from.to_path_buf(),
    }
}

/// Move `from` into `dest_dir` under `new_name`, deduping on collision
/// (`stem (n).ext`) and falling back to copy + delete for cross-volume moves.
/// Returns the path the file ended up at, or `from` unchanged on any IO
/// failure (so the DB keeps pointing at the real file rather than a path that
/// doesn't exist). [`move_into_folder`] is the keep-the-current-name case.
async fn move_renamed(from: &Path, dest_dir: &Path, new_name: &std::ffi::OsStr) -> PathBuf {
    if tokio::fs::create_dir_all(dest_dir).await.is_err() {
        return from.to_path_buf();
    }
    let mut dest = dest_dir.join(new_name);
    if dest == from {
        return from.to_path_buf();
    }
    if tokio::fs::metadata(&dest).await.is_ok() {
        dest = dedupe_path(dest_dir, new_name).await;
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
                    "failed to move completed download to learned path");
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

/// Persist the latest swarm snapshot into a torrent row's `torrent` JSON
/// blob (the `swarm` field of [`TorrentMeta`]). Mirrors how
/// [`persist_progress`] keeps the byte counters fresh, but for the
/// torrent-only swarm state so peers/seeds/ratio survive a relaunch and the
/// UI can render them before the session re-attaches (design §3.C).
///
/// Idempotent on a missing/non-torrent row: if the row has no `torrent`
/// blob (a data bug or an HTTP/media row) we leave it untouched rather than
/// fabricate one. Read-modify-write: the rest of the `TorrentMeta` (source,
/// selected files, file list) is preserved.
pub(crate) async fn persist_swarm(
    pool: &SqlitePool,
    id: DownloadId,
    swarm: &SwarmStats,
) -> Result<()> {
    let existing: Option<String> = sqlx::query_scalar("SELECT torrent FROM downloads WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .flatten();
    let Some(json) = existing.filter(|s| !s.is_empty()) else {
        // No torrent blob to merge into — nothing to persist.
        return Ok(());
    };
    let mut meta: TorrentMeta = match serde_json::from_str(&json) {
        Ok(m) => m,
        // A corrupt blob shouldn't take down the swarm pump; degrade.
        Err(_) => return Ok(()),
    };
    meta.swarm = Some(swarm.clone());
    let updated = serde_json::to_string(&meta)?;
    sqlx::query("UPDATE downloads SET torrent = ? WHERE id = ?")
        .bind(updated)
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
    // Defensive `try_get` for `kind` (migration 20260905000001): absent
    // column / NULL / unknown value all degrade to `Http`, matching the
    // NOT NULL DEFAULT 'http' the migration applies.
    let kind = row
        .try_get::<String, _>("kind")
        .ok()
        .and_then(|s| s.parse::<DownloadKind>().ok())
        .unwrap_or(DownloadKind::Http);
    // Defensive `try_get` for `torrent` (same migration): absent column /
    // NULL / unparseable JSON all degrade to `None`.
    let torrent = row
        .try_get::<Option<String>, _>("torrent")
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str::<TorrentMeta>(&s).ok());
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
        kind,
        torrent,
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
            kind: DownloadKind::Http,
            torrent: None,
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
            kind: DownloadKind::Http,
            torrent: None,
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

    #[test]
    fn download_kind_round_trip() {
        for kind in [
            DownloadKind::Http,
            DownloadKind::Media,
            DownloadKind::Torrent,
        ] {
            // string form round-trips through FromStr
            assert_eq!(kind.as_str().parse::<DownloadKind>().unwrap(), kind);
            // Display agrees with as_str
            assert_eq!(kind.to_string(), kind.as_str());
            // serde snake_case round-trips and matches as_str
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.as_str()));
            assert_eq!(serde_json::from_str::<DownloadKind>(&json).unwrap(), kind);
        }
        // Default is Http (mirrors the migration's NOT NULL DEFAULT).
        assert_eq!(DownloadKind::default(), DownloadKind::Http);
        // Unknown values are rejected, not silently defaulted, at the
        // FromStr boundary (record_from_row degrades separately).
        assert!("ftp".parse::<DownloadKind>().is_err());
    }

    #[test]
    fn torrent_meta_json_round_trip() {
        let meta = TorrentMeta {
            info_hash: "abcdef0123456789".to_string(),
            source: TorrentSource::Magnet {
                uri: "magnet:?xt=urn:btih:abcdef0123456789".to_string(),
            },
            selected_files: Some(vec![0, 2]),
            files: Some(vec![TorrentFile {
                index: 0,
                path: "dir/file.bin".to_string(),
                length: 1234,
                selected: true,
            }]),
            swarm: Some(SwarmStats {
                peers: 7,
                seeds: 3,
                up_bps: 100,
                down_bps: 2000,
                ratio_milli: 1500,
            }),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: TorrentMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back, meta);
        // TorrentSource uses an internally-tagged snake_case enum.
        assert!(json.contains("\"kind\":\"magnet\""));
    }

    #[tokio::test]
    async fn migration_backfills_media_kind() {
        // Reproduce the pre-torrent on-disk shape: a yt-dlp row carrying a
        // non-empty `media_info` blob but with `kind` still at the column's
        // NOT NULL DEFAULT 'http' (the value an unmigrated row would have
        // had before the backfill UPDATE ran). Then run the migration's
        // exact backfill statement and assert it upgrades only that row.
        let pool = fresh_pool().await;

        // A media-style row left at the default 'http' kind. Use a
        // complete `MediaInfo` JSON so the (non-defensive) media_info read
        // in `record_from_row` doesn't fail when we read the row back.
        let media_info_json =
            serde_json::to_string(&crate::ytdlp::MediaInfo {
                extractor: "test".to_string(),
                format_selector: "best".to_string(),
                title: "v".to_string(),
                original_url: "https://x/v".to_string(),
                needs_ffmpeg: false,
            })
            .unwrap();
        let media_id: i64 = sqlx::query(
            "INSERT INTO downloads (url, filename, output_path, downloaded_bytes, status, \
                                    priority, segments, created_at, media_info, kind) \
             VALUES ('https://x/v', 'v', '/tmp/v', 0, 'completed', 0, 1, \
                     '2026-01-01T00:00:00Z', ?, 'http') \
             RETURNING id",
        )
        .bind(&media_info_json)
        .fetch_one(&pool)
        .await
        .unwrap()
        .get("id");

        // A plain HTTP row with no media_info — must stay 'http'.
        let http_id: i64 = sqlx::query(
            "INSERT INTO downloads (url, filename, output_path, downloaded_bytes, status, \
                                    priority, segments, created_at, kind) \
             VALUES ('https://x/f', 'f', '/tmp/f', 0, 'completed', 0, 1, \
                     '2026-01-01T00:00:00Z', 'http') \
             RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .get("id");

        // The migration's backfill statement, verbatim.
        sqlx::query("UPDATE downloads SET kind = 'media' WHERE media_info IS NOT NULL AND media_info <> ''")
            .execute(&pool)
            .await
            .unwrap();

        let media_rec = get(&pool, media_id).await.unwrap();
        let http_rec = get(&pool, http_id).await.unwrap();
        assert_eq!(media_rec.kind, DownloadKind::Media, "media_info row upgraded");
        assert_eq!(http_rec.kind, DownloadKind::Http, "plain http row untouched");
    }

    #[tokio::test]
    async fn insert_normalizes_kind_from_media_info() {
        // A caller that passes media_info but forgets to set kind=Media
        // still lands as Media — the explicit column can't drift from the
        // JSON blob the worker branches on.
        let pool = fresh_pool().await;
        let input = AddDownload {
            url: "https://example.com/v".parse().unwrap(),
            filename: Some("v".to_string()),
            output_path: Some(std::path::PathBuf::from("/tmp/v")),
            category: None,
            priority: 0,
            segments: Some(1),
            media_info: Some(crate::ytdlp::MediaInfo {
                extractor: "test".to_string(),
                format_selector: "best".to_string(),
                title: "v".to_string(),
                original_url: "https://example.com/v".to_string(),
                needs_ffmpeg: false,
            }),
            headers: None,
            source: DownloadSource::Manual,
            kind: DownloadKind::Http, // deliberately wrong; insert must fix it
            torrent: None,
        };
        let rec = insert(&pool, input).await.unwrap();
        assert_eq!(rec.kind, DownloadKind::Media);
    }

    #[tokio::test]
    async fn insert_round_trips_torrent_meta() {
        let pool = fresh_pool().await;
        let meta = TorrentMeta {
            info_hash: "deadbeef".to_string(),
            source: TorrentSource::Magnet {
                uri: "magnet:?xt=urn:btih:deadbeef".to_string(),
            },
            selected_files: None,
            files: None,
            swarm: None,
        };
        let input = AddDownload {
            url: "magnet:?xt=urn:btih:deadbeef".parse().unwrap(),
            filename: Some("torrent".to_string()),
            output_path: Some(std::path::PathBuf::from("/tmp/torrent")),
            category: None,
            priority: 0,
            segments: Some(1),
            media_info: None,
            headers: None,
            source: DownloadSource::Manual,
            kind: DownloadKind::Torrent,
            torrent: Some(meta.clone()),
        };
        let rec = insert(&pool, input).await.unwrap();
        assert_eq!(rec.kind, DownloadKind::Torrent);
        assert_eq!(rec.torrent.as_ref(), Some(&meta));
        // Reads back identically through get/record_from_row.
        let again = get(&pool, rec.id).await.unwrap();
        assert_eq!(again.torrent.as_ref(), Some(&meta));
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

    /// Insert a row with an explicit url + on-disk path so the
    /// `apply_engine_filename` URL-tail guard can be exercised.
    async fn seed_row_with_url(
        pool: &SqlitePool,
        url: &str,
        filename: &str,
        output_path: &str,
        category_id: Option<i64>,
    ) -> i64 {
        let row = sqlx::query(
            "INSERT INTO downloads (url, filename, output_path, total_bytes, downloaded_bytes, \
                                    status, category_id, priority, segments, created_at) \
             VALUES (?, ?, ?, NULL, 0, 'active', ?, 0, 1, '2026-01-01T00:00:00Z') \
             RETURNING id",
        )
        .bind(url)
        .bind(filename)
        .bind(output_path)
        .bind(category_id)
        .fetch_one(pool)
        .await
        .unwrap();
        row.get("id")
    }

    #[tokio::test]
    async fn apply_engine_filename_renames_slug_and_recategorizes() {
        // The reported DDL symptom: the row was named after the random URL
        // slug because the add-time probe couldn't see the real name. The
        // download GET learned `clip.mp4` via Content-Disposition. With the
        // Video category pointed at its own folder, the file is both renamed
        // *and relocated* there so the on-disk location matches the UI.
        let pool = fresh_pool().await;
        let other = crate::category::find_by_name(&pool, "Other")
            .await
            .unwrap()
            .unwrap();
        let video = crate::category::find_by_name(&pool, "Video")
            .await
            .unwrap()
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let other_dir = tmp.path().join("Other");
        let video_dir = tmp.path().join("Video");
        tokio::fs::create_dir_all(&other_dir).await.unwrap();
        // Configure the Video category's folder (a user who set per-category
        // download folders); the recategorized file should land here.
        sqlx::query("UPDATE categories SET default_output_path = ? WHERE id = ?")
            .bind(video_dir.to_string_lossy().as_ref())
            .bind(video.id)
            .execute(&pool)
            .await
            .unwrap();

        let slug = "BWImVeeBXzQpnkCSnOk7PLUjH";
        let on_disk = other_dir.join(slug);
        tokio::fs::write(&on_disk, b"video bytes").await.unwrap();
        let url = format!("https://dl.fuckingfast.co/dl/{slug}");
        let id = seed_row_with_url(&pool, &url, slug, on_disk.to_str().unwrap(), Some(other.id))
            .await;

        let renamed = apply_engine_filename(&pool, id, &url.parse().unwrap(), "clip.mp4")
            .await
            .unwrap()
            .expect("a better name should rename");

        let moved = video_dir.join("clip.mp4");
        assert_eq!(renamed.filename, "clip.mp4");
        assert!(renamed.category_changed);
        assert_eq!(renamed.category_id, Some(video.id));
        assert_eq!(renamed.path, moved);
        // File physically relocated into the Video folder under the new name;
        // the slug in the Other folder is gone.
        assert!(tokio::fs::metadata(&moved).await.is_ok());
        assert!(tokio::fs::metadata(&on_disk).await.is_err());

        let row = sqlx::query("SELECT filename, output_path, category_id FROM downloads WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("filename"), "clip.mp4");
        assert_eq!(
            row.get::<String, _>("output_path"),
            moved.to_string_lossy()
        );
        assert_eq!(row.get::<i64, _>("category_id"), video.id);
    }

    #[tokio::test]
    async fn apply_engine_filename_renames_in_place_when_category_unchanged() {
        // The learned name improves on the slug but its extension routes to
        // the same category (`.dat` is in no category's rules → Other), so
        // the file is renamed inside Other's folder, not relocated elsewhere.
        let pool = fresh_pool().await;
        let other = crate::category::find_by_name(&pool, "Other")
            .await
            .unwrap()
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("downloads");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        // Point the Other category at this folder so it's where the file both
        // started and stays (the realistic shape: a row lives in the folder
        // its category resolves to).
        sqlx::query("UPDATE categories SET default_output_path = ? WHERE id = ?")
            .bind(dir.to_string_lossy().as_ref())
            .bind(other.id)
            .execute(&pool)
            .await
            .unwrap();
        let slug = "abc123def456";
        let on_disk = dir.join(slug);
        tokio::fs::write(&on_disk, b"x").await.unwrap();
        let url = format!("https://host.example/d/{slug}");
        let id = seed_row_with_url(&pool, &url, slug, on_disk.to_str().unwrap(), Some(other.id))
            .await;

        let renamed = apply_engine_filename(&pool, id, &url.parse().unwrap(), "data.dat")
            .await
            .unwrap()
            .expect("a better name should still rename in place");

        assert_eq!(renamed.filename, "data.dat");
        assert!(!renamed.category_changed);
        assert_eq!(renamed.category_id, Some(other.id));
        // Renamed in the SAME folder, not relocated.
        assert_eq!(renamed.path, dir.join("data.dat"));
        assert!(tokio::fs::metadata(dir.join("data.dat")).await.is_ok());
        assert!(tokio::fs::metadata(&on_disk).await.is_err());
    }

    #[tokio::test]
    async fn apply_engine_filename_respects_non_slug_name() {
        // The stored name doesn't match the URL tail, so it came from a
        // real source (a user choice or an add-time Content-Disposition).
        // The engine hint must NOT clobber it.
        let pool = fresh_pool().await;
        let tmp = tempfile::tempdir().unwrap();
        let on_disk = tmp.path().join("My Report.pdf");
        tokio::fs::write(&on_disk, b"pdf").await.unwrap();
        let url = "https://host.example.com/d/abc123xyz";
        let id = seed_row_with_url(&pool, url, "My Report.pdf", on_disk.to_str().unwrap(), None)
            .await;

        let got = apply_engine_filename(&pool, id, &url.parse().unwrap(), "something-else.bin")
            .await
            .unwrap();
        assert!(got.is_none(), "a deliberately-chosen name is never overridden");
        assert!(tokio::fs::metadata(&on_disk).await.is_ok(), "file untouched");
    }

    #[tokio::test]
    async fn apply_engine_filename_ignores_slug_hint() {
        // Even when the stored name IS the URL slug, a hint that is itself a
        // slug is no improvement and must be ignored (no needless rename).
        let pool = fresh_pool().await;
        let tmp = tempfile::tempdir().unwrap();
        let slug = "LdFGH2vN9xKq";
        let on_disk = tmp.path().join(slug);
        tokio::fs::write(&on_disk, b"x").await.unwrap();
        let url = format!("https://pixeldrain.com/api/file/{slug}");
        let id = seed_row_with_url(&pool, &url, slug, on_disk.to_str().unwrap(), None).await;

        let got = apply_engine_filename(&pool, id, &url.parse().unwrap(), "anotherSlug12345")
            .await
            .unwrap();
        assert!(got.is_none(), "a sluggy hint is no better than the slug");
    }

    #[tokio::test]
    async fn mark_learned_filename_updates_display_without_moving_file() {
        // Mid-download: the engine learns the real name from the response
        // headers. The row's display name + category update immediately so the
        // user isn't staring at a slug, but the file on disk stays put — the
        // engine is still writing it.
        let pool = fresh_pool().await;
        let other = crate::category::find_by_name(&pool, "Other")
            .await
            .unwrap()
            .unwrap();
        let video = crate::category::find_by_name(&pool, "Video")
            .await
            .unwrap()
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let slug = "BWImVeeBXzQpnkCSnOk7PLUjH";
        let on_disk = tmp.path().join(slug);
        tokio::fs::write(&on_disk, b"partial").await.unwrap();
        let url = format!("https://dl.fuckingfast.co/dl/{slug}");
        let id = seed_row_with_url(&pool, &url, slug, on_disk.to_str().unwrap(), Some(other.id))
            .await;

        let learned = mark_learned_filename(&pool, id, &url.parse().unwrap(), "clip.mp4")
            .await
            .unwrap()
            .expect("the real name should be applied to the display row");

        assert_eq!(learned.filename, "clip.mp4");
        assert!(learned.category_changed);
        assert_eq!(learned.category_id, Some(video.id));
        // The file has NOT moved: still at the slug path, no clip.mp4 yet.
        assert_eq!(learned.output_path, on_disk);
        assert!(tokio::fs::metadata(&on_disk).await.is_ok());
        assert!(tokio::fs::metadata(tmp.path().join("clip.mp4")).await.is_err());

        let row = sqlx::query("SELECT filename, output_path, category_id FROM downloads WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("filename"), "clip.mp4");
        // output_path still points at the working (slug) file.
        assert_eq!(
            row.get::<String, _>("output_path"),
            on_disk.to_string_lossy()
        );
        assert_eq!(row.get::<i64, _>("category_id"), video.id);

        // Idempotent: re-emitting the same hint (resume/restart) is a no-op.
        let again = mark_learned_filename(&pool, id, &url.parse().unwrap(), "clip.mp4")
            .await
            .unwrap();
        assert!(again.is_none(), "re-applying the same name changes nothing");
    }

    #[tokio::test]
    async fn apply_engine_filename_relocates_after_early_display_update() {
        // After a mid-flight display update swapped the slug for the real
        // name, completion must still relocate the physical file (still at the
        // slug path) into the category folder — keyed off the on-disk name,
        // not the already-updated display name.
        let pool = fresh_pool().await;
        let other = crate::category::find_by_name(&pool, "Other")
            .await
            .unwrap()
            .unwrap();
        let video = crate::category::find_by_name(&pool, "Video")
            .await
            .unwrap()
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let other_dir = tmp.path().join("Other");
        let video_dir = tmp.path().join("Video");
        tokio::fs::create_dir_all(&other_dir).await.unwrap();
        sqlx::query("UPDATE categories SET default_output_path = ? WHERE id = ?")
            .bind(video_dir.to_string_lossy().as_ref())
            .bind(video.id)
            .execute(&pool)
            .await
            .unwrap();

        let slug = "BWImVeeBXzQpnkCSnOk7PLUjH";
        let on_disk = other_dir.join(slug);
        tokio::fs::write(&on_disk, b"video bytes").await.unwrap();
        let url = format!("https://dl.fuckingfast.co/dl/{slug}");
        let id = seed_row_with_url(&pool, &url, slug, on_disk.to_str().unwrap(), Some(other.id))
            .await;

        // Mid-flight: display name + category already updated, file unmoved.
        mark_learned_filename(&pool, id, &url.parse().unwrap(), "clip.mp4")
            .await
            .unwrap()
            .expect("display update");

        // Completion: physical relocate into the Video folder.
        let renamed = apply_engine_filename(&pool, id, &url.parse().unwrap(), "clip.mp4")
            .await
            .unwrap()
            .expect("the file should still be relocated at completion");

        let moved = video_dir.join("clip.mp4");
        assert_eq!(renamed.filename, "clip.mp4");
        // Category was already Video from the mid-flight update, so completion
        // reports no further change (the queue won't re-emit CategoryChanged).
        assert!(!renamed.category_changed);
        assert_eq!(renamed.category_id, Some(video.id));
        assert_eq!(renamed.path, moved);
        assert!(tokio::fs::metadata(&moved).await.is_ok());
        assert!(tokio::fs::metadata(&on_disk).await.is_err());
    }

    // --- Phase 2b: torrent insert / de-dup / reconcile -----------------------

    #[test]
    fn info_hash_from_magnet_extracts_lowercase_hex() {
        let m = "magnet:?xt=urn:btih:6F84758B0DDD8DC05840BF932A77935D8B5B8B93&dn=debian.iso";
        assert_eq!(
            info_hash_from_magnet(m).as_deref(),
            Some("6f84758b0ddd8dc05840bf932a77935d8b5b8b93")
        );
        // Case-insensitive scheme + extra params in any order.
        let m2 = "magnet:?tr=udp%3A%2F%2Ftracker&XT=URN:BTIH:abcdef0123456789abcdef0123456789abcdef01";
        assert_eq!(
            info_hash_from_magnet(m2).as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef01")
        );
        // No xt param, or non-40-hex (base32) form → None (caller falls back).
        assert!(info_hash_from_magnet("magnet:?dn=no-hash").is_none());
        assert!(info_hash_from_magnet("magnet:?xt=urn:btih:TOOSHORT").is_none());
    }

    #[test]
    fn provisional_torrent_name_prefers_dn_then_stem_then_default() {
        let url: url::Url = "magnet:?xt=urn:btih:deadbeef".parse().unwrap();
        // magnet dn= (with + and percent-encoding) wins.
        let magnet = TorrentMeta {
            info_hash: "deadbeef".into(),
            source: TorrentSource::Magnet {
                uri: "magnet:?xt=urn:btih:deadbeef&dn=My%20Linux+ISO".into(),
            },
            selected_files: None,
            files: None,
            swarm: None,
        };
        assert_eq!(provisional_torrent_name(Some(&magnet), &url), "My Linux ISO");

        // .torrent file → stem.
        let file = TorrentMeta {
            info_hash: "deadbeef".into(),
            source: TorrentSource::File {
                path: PathBuf::from("/downloads/ubuntu-24.04.torrent"),
            },
            selected_files: None,
            files: None,
            swarm: None,
        };
        assert_eq!(provisional_torrent_name(Some(&file), &url), "ubuntu-24.04");

        // Bare info-hash with no dn → "torrent" fallback.
        let ih = TorrentMeta {
            info_hash: "deadbeef".into(),
            source: TorrentSource::InfoHash {
                hash: "deadbeef".into(),
            },
            selected_files: None,
            files: None,
            swarm: None,
        };
        let plain: url::Url = "magnet:?xt=urn:btih:deadbeef".parse().unwrap();
        assert_eq!(provisional_torrent_name(Some(&ih), &plain), "torrent");
    }

    #[tokio::test]
    async fn insert_derives_provisional_name_from_magnet_dn() {
        // No caller filename: a magnet row takes its dn= as the provisional
        // name (no HEAD probe), sanitized.
        let pool = fresh_pool().await;
        let uri = "magnet:?xt=urn:btih:6f84758b0ddd8dc05840bf932a77935d8b5b8b93&dn=Debian+ISO";
        let input = AddDownload {
            url: uri.parse().unwrap(),
            filename: None,
            output_path: Some(PathBuf::from("/tmp/torrents/debian")),
            category: None,
            priority: 0,
            segments: None,
            media_info: None,
            headers: None,
            source: DownloadSource::Manual,
            kind: DownloadKind::Torrent,
            torrent: Some(TorrentMeta {
                info_hash: String::new(), // forces derivation from the magnet
                source: TorrentSource::Magnet { uri: uri.into() },
                selected_files: None,
                files: None,
                swarm: None,
            }),
        };
        let rec = insert(&pool, input).await.unwrap();
        assert_eq!(rec.kind, DownloadKind::Torrent);
        assert_eq!(rec.filename, "Debian ISO");
        // info_hash was derived from xt= and lowercased.
        assert_eq!(
            rec.torrent.as_ref().unwrap().info_hash,
            "6f84758b0ddd8dc05840bf932a77935d8b5b8b93"
        );
    }

    #[tokio::test]
    async fn insert_dedups_duplicate_magnet_add() {
        // Q7 front-door de-dup: a second add of the SAME info-hash hands back
        // the existing row rather than inserting a twin.
        let pool = fresh_pool().await;
        let uri = "magnet:?xt=urn:btih:1111111111111111111111111111111111111111&dn=Thing";
        let make = || AddDownload {
            url: uri.parse().unwrap(),
            filename: None,
            output_path: Some(PathBuf::from("/tmp/torrents/thing")),
            category: None,
            priority: 0,
            segments: None,
            media_info: None,
            headers: None,
            source: DownloadSource::Manual,
            kind: DownloadKind::Torrent,
            torrent: Some(TorrentMeta {
                info_hash: String::new(),
                source: TorrentSource::Magnet { uri: uri.into() },
                selected_files: None,
                files: None,
                swarm: None,
            }),
        };
        let first = insert(&pool, make()).await.unwrap();
        let second = insert(&pool, make()).await.unwrap();
        assert_eq!(first.id, second.id, "duplicate add must be a no-op");

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM downloads WHERE kind = 'torrent'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "only one row for the swarm");
    }

    #[tokio::test]
    async fn insert_torrent_skips_dedup_for_distinct_hashes() {
        // Two genuinely different torrents both insert — de-dup keys on the
        // hash, not on kind.
        let pool = fresh_pool().await;
        let mk = |hash40: &str| {
            let uri = format!("magnet:?xt=urn:btih:{hash40}&dn=t");
            AddDownload {
                url: uri.parse().unwrap(),
                filename: None,
                output_path: Some(PathBuf::from(format!("/tmp/torrents/{hash40}"))),
                category: None,
                priority: 0,
                segments: None,
                media_info: None,
                headers: None,
                source: DownloadSource::Manual,
                kind: DownloadKind::Torrent,
                torrent: Some(TorrentMeta {
                    info_hash: String::new(),
                    source: TorrentSource::Magnet { uri },
                    selected_files: None,
                    files: None,
                    swarm: None,
                }),
            }
        };
        let a = insert(&pool, mk("2222222222222222222222222222222222222222"))
            .await
            .unwrap();
        let b = insert(&pool, mk("3333333333333333333333333333333333333333"))
            .await
            .unwrap();
        assert_ne!(a.id, b.id);
    }

    #[tokio::test]
    async fn reconcile_torrent_filename_updates_display_and_category() {
        // The facade learned the real torrent name post-metadata. The display
        // name + (auto) category update; nothing on disk is touched.
        let pool = fresh_pool().await;
        let other = crate::category::find_by_name(&pool, "Other")
            .await
            .unwrap()
            .unwrap();
        let video = crate::category::find_by_name(&pool, "Video")
            .await
            .unwrap()
            .unwrap();
        // Provisional torrent row: a name with no extension auto-routes to Other.
        let id = seed_row(&pool, "magnet-provisional", Some(other.id), None).await;

        let (name, cat, changed) =
            reconcile_torrent_filename(&pool, id, "Big Buck Bunny.mp4")
                .await
                .unwrap()
                .expect("a real name reconciles");
        assert_eq!(name, "Big Buck Bunny.mp4");
        assert!(changed);
        assert_eq!(cat, Some(video.id));

        let row = sqlx::query("SELECT filename, category_id FROM downloads WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("filename"), "Big Buck Bunny.mp4");
        assert_eq!(row.get::<i64, _>("category_id"), video.id);

        // Idempotent: re-emitting the resolved name changes nothing.
        let again = reconcile_torrent_filename(&pool, id, "Big Buck Bunny.mp4")
            .await
            .unwrap();
        assert!(again.is_none());
    }

    #[tokio::test]
    async fn reconcile_torrent_filename_preserves_user_category() {
        // A deliberate user category is never re-routed by the learned name.
        let pool = fresh_pool().await;
        let docs = crate::category::find_by_name(&pool, "Documents")
            .await
            .unwrap()
            .unwrap();
        // "weird name" doesn't auto-route to Documents, so it's a user choice.
        let id = seed_row(&pool, "weird name", Some(docs.id), None).await;

        let (name, cat, changed) = reconcile_torrent_filename(&pool, id, "movie.mp4")
            .await
            .unwrap()
            .expect("name still updates");
        assert_eq!(name, "movie.mp4");
        assert!(!changed, "user category preserved");
        assert_eq!(cat, Some(docs.id));
    }

    /// Seed a torrent row whose `torrent` column holds a serialized
    /// [`TorrentMeta`] (no swarm snapshot yet), returning its id and the
    /// stored meta. Used to exercise [`persist_swarm`]'s read-modify-write.
    async fn seed_torrent_row(pool: &SqlitePool) -> (i64, TorrentMeta) {
        let meta = TorrentMeta {
            info_hash: "aabbccddeeff00112233445566778899aabbccdd".into(),
            source: TorrentSource::Magnet {
                uri: "magnet:?xt=urn:btih:aabbccddeeff00112233445566778899aabbccdd&dn=Thing".into(),
            },
            selected_files: Some(vec![0, 2]),
            files: Some(vec![TorrentFile {
                index: 0,
                path: "a.bin".into(),
                length: 100,
                selected: true,
            }]),
            swarm: None,
        };
        let json = serde_json::to_string(&meta).unwrap();
        let row = sqlx::query(
            "INSERT INTO downloads (url, filename, output_path, downloaded_bytes, status, \
                                    priority, segments, created_at, kind, torrent) \
             VALUES ('magnet:?xt=urn:btih:aabbccddeeff00112233445566778899aabbccdd', \
                     'Thing', '/tmp/torrents/thing', 0, 'active', 0, 1, \
                     '2026-01-01T00:00:00Z', 'torrent', ?) \
             RETURNING id",
        )
        .bind(&json)
        .fetch_one(pool)
        .await
        .unwrap();
        (row.get("id"), meta)
    }

    #[tokio::test]
    async fn persist_swarm_round_trips_into_torrent_json() {
        // The pump persists each swarm snapshot into the row's `torrent` blob
        // (design §3.C) so peers/seeds survive a relaunch. The snapshot lands
        // in `swarm` and the rest of the meta is preserved untouched.
        let pool = fresh_pool().await;
        let (id, original) = seed_torrent_row(&pool).await;

        let snap = SwarmStats {
            peers: 12,
            seeds: 30,
            up_bps: 4_096,
            down_bps: 1_048_576,
            ratio_milli: 1500,
        };
        persist_swarm(&pool, id, &snap).await.unwrap();

        // Read back through the same defensive path the worker uses.
        let rec = get(&pool, id).await.unwrap();
        let meta = rec.torrent.expect("torrent blob preserved");
        assert_eq!(meta.swarm.as_ref(), Some(&snap), "snapshot persisted");
        // Everything else is untouched by the read-modify-write.
        assert_eq!(meta.info_hash, original.info_hash);
        assert_eq!(meta.source, original.source);
        assert_eq!(meta.selected_files, original.selected_files);
        assert_eq!(meta.files, original.files);

        // A later snapshot overwrites the previous one in place.
        let snap2 = SwarmStats {
            peers: 8,
            seeds: 25,
            up_bps: 0,
            down_bps: 2_000,
            ratio_milli: 1750,
        };
        persist_swarm(&pool, id, &snap2).await.unwrap();
        let meta2 = get(&pool, id).await.unwrap().torrent.unwrap();
        assert_eq!(meta2.swarm.as_ref(), Some(&snap2));
    }

    #[tokio::test]
    async fn persist_swarm_is_noop_without_torrent_blob() {
        // An HTTP/media row (no `torrent` blob) must not gain a fabricated one.
        let pool = fresh_pool().await;
        let id = seed_row(&pool, "plain.bin", None, None).await;
        let snap = SwarmStats {
            peers: 1,
            seeds: 1,
            up_bps: 0,
            down_bps: 0,
            ratio_milli: 0,
        };
        // Does not error and leaves the column NULL.
        persist_swarm(&pool, id, &snap).await.unwrap();
        let torrent: Option<String> = sqlx::query_scalar("SELECT torrent FROM downloads WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(torrent.is_none(), "no torrent blob fabricated");
    }
}
