//! Thin `#[tauri::command]` wrappers around [`unduhin_core::Core`].
//!
//! Each command translates serializable input into the typed `Core` API
//! and serializes the result back. No download or queue logic lives
//! here — that's all in `unduhin-core`.

use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use tauri::State;

use unduhin_core::{
    tooling::{Tool, ToolStatus},
    wire::{ExtensionSettings, HandoffDecision, RuleMetric, SettingsPatch},
    ytdlp::{MediaInfo, ProbeResult},
    AddDownload, Category, CategoryId, CategorySelector, Core, DownloadFilter, DownloadId,
    DownloadKind, DownloadRecord, DownloadSource, NewCategory, NewSchedule, QuietHoursState,
    Schedule, ScheduleId, SettingValue, Status, TorrentMeta,
};

use crate::error::{CommandError, CommandResult};
use crate::window::ConfirmOnQuitBridge;

#[derive(Debug, Deserialize)]
pub struct AddDownloadInput {
    pub url: String,
    pub filename: Option<String>,
    pub output_path: Option<String>,
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
    pub priority: Option<i64>,
    pub segments: Option<u32>,
    /// Present when the frontend probed the URL with yt-dlp first and
    /// the user picked a format in the media dialog.
    pub media_info: Option<MediaInfo>,
    /// Captured browser request headers — populated by native
    /// host hand-offs. Frontend Add URL paths leave this `None`.
    pub headers: Option<Vec<(String, String)>>,
    /// Which backend should run the download. Omitted (or `Http`) for the
    /// direct-file / media paths; the torrent path (P4) sends `Torrent`
    /// together with `torrent`. Defaults to `Http`.
    #[serde(default)]
    pub kind: DownloadKind,
    /// Torrent state when `kind == Torrent` — the resolved `info_hash`,
    /// source, and the user's file selection from the add-time picker.
    #[serde(default)]
    pub torrent: Option<TorrentMeta>,
}

#[derive(Debug, Deserialize, Default)]
pub struct DownloadFilterInput {
    pub status: Option<String>,
    pub category_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct AppInfo {
    pub version: &'static str,
    pub name: &'static str,
    pub git_sha: &'static str,
    pub build_timestamp: &'static str,
    /// `"stable"` or `"beta"` — read from the persisted setting at call time
    /// rather than baked in, so the About page reflects user choices live.
    pub channel: String,
    /// Result of OS detection — e.g. `"Windows 11 · x64 · 22631.3593"`.
    pub os: String,
}

#[tauri::command]
pub async fn app_info(core: State<'_, Core>) -> CommandResult<AppInfo> {
    let info = unduhin_core::build_info::build_info();
    let channel = core
        .get_setting(unduhin_core::settings_keys::UPDATE_CHANNEL)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "stable".into());
    Ok(AppInfo {
        version: info.version,
        name: "Unduhin",
        git_sha: info.git_sha,
        build_timestamp: info.build_timestamp,
        channel,
        os: crate::os_summary(),
    })
}

#[derive(Debug, Serialize)]
pub struct UpdateCheckResult {
    pub status: &'static str, // "up_to_date" | "update_available" | "error"
    pub available_version: Option<String>,
    pub notes: Option<String>,
    pub checked_at: String,
}

/// Records the timestamp + outcome of an update check the frontend just
/// performed. The actual HTTP fetch happens on the JS side via the Tauri
/// updater plugin so we don't duplicate retry/signature logic — this
/// command just persists the result so the About page can show
/// "last checked N minutes ago" after a reload.
#[tauri::command]
pub async fn record_update_check(
    core: State<'_, Core>,
    status: String,
    available_version: Option<String>,
    notes: Option<String>,
) -> CommandResult<()> {
    use unduhin_core::{settings_keys, SettingValue};
    let normalized = match status.as_str() {
        "up_to_date" | "update_available" | "error" => status,
        other => {
            return Err(CommandError {
                message: format!("invalid update-check status: {other}"),
            })
        }
    };
    core.set_setting(
        settings_keys::LAST_UPDATE_CHECK_RESULT,
        SettingValue::from_string(normalized.clone()),
    )
    .await?;
    core.set_setting(
        settings_keys::LAST_UPDATE_CHECK_AT,
        SettingValue::from_string(chrono::Utc::now().to_rfc3339()),
    )
    .await?;
    let _ = (available_version, notes); // reserved for future schema extension
    Ok(())
}

/// Frontend → Rust answer for an active confirm-on-quit prompt. The
/// `request_id` identifies which pending close decision this answer
/// resolves; unknown ids are tolerated (a late click after the timeout
/// shouldn't error in the UI).
#[tauri::command]
pub fn confirm_quit_response(
    bridge: State<'_, ConfirmOnQuitBridge>,
    request_id: u32,
    allow: bool,
) -> CommandResult<()> {
    bridge.respond(request_id, allow);
    Ok(())
}

/// Trigger a graceful app exit. Used by the tray menu's *Quit* item and
/// any future "force quit" affordance. Bypasses the close-behavior
/// policy — when the user explicitly asks to quit, they mean it.
#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) -> CommandResult<()> {
    app.exit(0);
    Ok(())
}

/// Path to the rotating log directory, or `null` when logging hasn't been
/// initialized (tests, headless smoke).
#[tauri::command]
pub fn get_logs_dir() -> Option<String> {
    unduhin_core::logging::logs_dir().map(|p| p.to_string_lossy().into_owned())
}

#[derive(Debug, Serialize)]
pub struct DiskInfo {
    pub drive: String,
    pub free_bytes: u64,
    pub total_bytes: u64,
}

/// Free / total bytes for the drive that hosts the configured
/// `default_output_path` (or `%USERPROFILE%` as a fallback).
#[tauri::command]
pub async fn get_disk_info(core: State<'_, Core>) -> CommandResult<DiskInfo> {
    let configured = core
        .get_setting("default_output_path")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());

    let target = configured
        .or_else(|| std::env::var("USERPROFILE").ok())
        .unwrap_or_else(|| "C:\\".to_string());

    let drive_letter = target
        .chars()
        .next()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_uppercase())
        .unwrap_or('C');

    let disks = sysinfo::Disks::new_with_refreshed_list();
    let prefix = format!("{}:", drive_letter);
    for disk in disks.iter() {
        let mount = disk.mount_point().to_string_lossy();
        if mount.to_ascii_uppercase().starts_with(&prefix) {
            return Ok(DiskInfo {
                drive: format!("{}:\\", drive_letter),
                free_bytes: disk.available_space(),
                total_bytes: disk.total_space(),
            });
        }
    }

    // Fallback: report the first disk we can see (e.g. on non-Windows)
    if let Some(disk) = disks.iter().next() {
        return Ok(DiskInfo {
            drive: disk.mount_point().to_string_lossy().into_owned(),
            free_bytes: disk.available_space(),
            total_bytes: disk.total_space(),
        });
    }

    Err(CommandError {
        message: format!("could not resolve disk for {target}"),
    })
}

#[tauri::command]
pub async fn add_download(
    core: State<'_, Core>,
    input: AddDownloadInput,
) -> CommandResult<DownloadId> {
    // The `kind` column and the `torrent` blob must agree; trust either
    // signal so a caller that set only one still lands a torrent row.
    let is_torrent = input.kind == DownloadKind::Torrent || input.torrent.is_some();

    // For HTTP / media rows `url` must be a real URL. Torrent rows carry the
    // magnet URI (or a `file:`-wrapped `.torrent` path) in this field; magnets
    // parse fine, but a `.torrent` source may not, so for torrents we fall
    // back to synthesizing a magnet URL from the resolved info-hash. The
    // backend keys de-dup and the provisional name off `torrent.source`, not
    // this column, so the fallback is only a parseable placeholder.
    let url = match url::Url::parse(&input.url) {
        Ok(u) => u,
        Err(e) if is_torrent => {
            let hash = input
                .torrent
                .as_ref()
                .map(|t| t.info_hash.as_str())
                .filter(|h| !h.is_empty());
            match hash {
                Some(h) => url::Url::parse(&format!("magnet:?xt=urn:btih:{h}")).map_err(|e| {
                    CommandError {
                        message: format!("invalid torrent magnet placeholder: {e}"),
                    }
                })?,
                None => {
                    return Err(CommandError {
                        message: format!("invalid torrent input: {e}"),
                    })
                }
            }
        }
        Err(e) => {
            return Err(CommandError {
                message: format!("invalid URL: {e}"),
            })
        }
    };

    let category = match (input.category_id, input.category_name) {
        (Some(id), _) => Some(CategorySelector::Id(id)),
        (None, Some(name)) if !name.is_empty() => Some(CategorySelector::Name(name)),
        _ => None,
    };

    // `insert` re-normalizes the discriminator against the JSON blobs, but we
    // pass the explicit `kind` through so a torrent without a resolved blob
    // (shouldn't happen from the UI) still lands as `Torrent`.
    let kind = if is_torrent {
        DownloadKind::Torrent
    } else {
        input.kind
    };

    let id = core
        .add_download(AddDownload {
            url,
            filename: input.filename,
            output_path: input.output_path.map(PathBuf::from),
            category,
            priority: input.priority.unwrap_or(0),
            segments: input.segments,
            media_info: input.media_info,
            headers: input.headers,
            source: DownloadSource::Manual,
            kind,
            torrent: input.torrent,
        })
        .await?;
    Ok(id)
}

#[tauri::command]
pub async fn list_downloads(
    core: State<'_, Core>,
    filter: Option<DownloadFilterInput>,
) -> CommandResult<Vec<DownloadRecord>> {
    let filter = filter.unwrap_or_default();
    let status = filter
        .status
        .as_deref()
        .map(Status::from_str)
        .transpose()
        .map_err(CommandError::from)?;
    let rows = core
        .list_downloads(DownloadFilter {
            status,
            category_id: filter.category_id,
        })
        .await?;
    Ok(rows)
}

#[tauri::command]
pub async fn get_download(core: State<'_, Core>, id: DownloadId) -> CommandResult<DownloadRecord> {
    Ok(core.get_download(id).await?)
}

#[tauri::command]
pub async fn pause_download(core: State<'_, Core>, id: DownloadId) -> CommandResult<()> {
    Ok(core.pause(id).await?)
}

#[tauri::command]
pub async fn resume_download(core: State<'_, Core>, id: DownloadId) -> CommandResult<()> {
    Ok(core.resume(id).await?)
}

#[tauri::command]
pub async fn cancel_download(core: State<'_, Core>, id: DownloadId) -> CommandResult<()> {
    Ok(core.cancel(id).await?)
}

#[tauri::command]
pub async fn retry_download(core: State<'_, Core>, id: DownloadId) -> CommandResult<()> {
    Ok(core.retry(id).await?)
}

#[tauri::command]
pub async fn remove_download(
    core: State<'_, Core>,
    id: DownloadId,
    delete_data: Option<bool>,
) -> CommandResult<()> {
    Ok(core.remove(id, delete_data.unwrap_or(false)).await?)
}

#[tauri::command]
pub async fn set_priority(
    core: State<'_, Core>,
    id: DownloadId,
    priority: i64,
) -> CommandResult<()> {
    Ok(core.set_priority(id, priority).await?)
}

#[tauri::command]
pub async fn set_segments(core: State<'_, Core>, id: DownloadId, n: u32) -> CommandResult<()> {
    Ok(core.set_segments(id, n as usize).await?)
}

#[tauri::command]
pub async fn set_category(
    core: State<'_, Core>,
    id: DownloadId,
    category_id: Option<CategoryId>,
) -> CommandResult<()> {
    Ok(core.set_category(id, category_id).await?)
}

/// HEAD-probe `url` and return the engine's best-effort derived filename.
/// Returns `Ok(None)` when nothing usable could be derived. Used by the
/// Add download dialog to prefill the override field.
#[tauri::command]
pub async fn preview_filename(core: State<'_, Core>, url: String) -> CommandResult<Option<String>> {
    Ok(core.preview_filename(&url).await?)
}

// Pause / resume all

#[tauri::command]
pub async fn pause_all(core: State<'_, Core>) -> CommandResult<u32> {
    let rows = core.list_downloads(DownloadFilter::default()).await?;
    let mut n = 0u32;
    for r in rows {
        if matches!(r.status, Status::Queued | Status::Active) && core.pause(r.id).await.is_ok() {
            n += 1;
        }
    }
    Ok(n)
}

#[tauri::command]
pub async fn resume_all(core: State<'_, Core>) -> CommandResult<u32> {
    let rows = core
        .list_downloads(DownloadFilter {
            status: Some(Status::Paused),
            category_id: None,
        })
        .await?;
    let mut n = 0u32;
    for r in rows {
        if core.resume(r.id).await.is_ok() {
            n += 1;
        }
    }
    Ok(n)
}

// Categories

#[tauri::command]
pub async fn list_categories(core: State<'_, Core>) -> CommandResult<Vec<Category>> {
    Ok(core.list_categories().await?)
}

#[derive(Debug, Deserialize)]
pub struct NewCategoryInput {
    pub name: String,
    pub icon: Option<String>,
    pub default_output_path: Option<String>,
    pub extension_rules: Option<Vec<String>>,
}

#[tauri::command]
pub async fn add_category(
    core: State<'_, Core>,
    input: NewCategoryInput,
) -> CommandResult<CategoryId> {
    Ok(core
        .add_category(NewCategory {
            name: input.name,
            icon: input.icon,
            default_output_path: input.default_output_path.map(PathBuf::from),
            extension_rules: input.extension_rules.unwrap_or_default(),
        })
        .await?)
}

#[tauri::command]
pub async fn update_category(
    core: State<'_, Core>,
    id: CategoryId,
    input: NewCategoryInput,
) -> CommandResult<()> {
    Ok(core
        .update_category(
            id,
            NewCategory {
                name: input.name,
                icon: input.icon,
                default_output_path: input.default_output_path.map(PathBuf::from),
                extension_rules: input.extension_rules.unwrap_or_default(),
            },
        )
        .await?)
}

#[tauri::command]
pub async fn remove_category(core: State<'_, Core>, id: CategoryId) -> CommandResult<()> {
    Ok(core.remove_category(id).await?)
}

#[tauri::command]
pub async fn set_category_order(core: State<'_, Core>, ids: Vec<CategoryId>) -> CommandResult<()> {
    Ok(core.set_category_order(ids).await?)
}

// Settings

#[tauri::command]
pub async fn get_settings(
    core: State<'_, Core>,
) -> CommandResult<std::collections::HashMap<String, SettingValue>> {
    Ok(core.all_settings().await?)
}

#[tauri::command]
pub async fn get_setting(
    core: State<'_, Core>,
    key: String,
) -> CommandResult<Option<SettingValue>> {
    Ok(core.get_setting(&key).await?)
}

#[derive(Debug, Deserialize)]
pub struct SetSettingInput {
    pub key: String,
    pub value: serde_json::Value,
}

#[tauri::command]
pub async fn set_setting(core: State<'_, Core>, input: SetSettingInput) -> CommandResult<()> {
    core.set_setting(&input.key, SettingValue(input.value))
        .await?;
    Ok(())
}

// Media (yt-dlp / ffmpeg)

/// Probe a URL with the installed yt-dlp. Returns `Ok(Some(_))` when yt-dlp
/// recognizes the URL as media, and `Ok(None)` when it can't be handled as
/// media — in which case the AddUrlDialog falls through to the direct-file
/// engine path. `Ok(None)` covers: unsupported URLs, probe timeouts, and an
/// opaque non-zero yt-dlp exit (`Process`) such as a Google Drive
/// quota/permission page — those are ordinary files that should just
/// download. Genuinely actionable problems (`NotInstalled`, `FfmpegMissing`,
/// `Drm`, `BotChallenge`, sign-in `AuthRequired`, parse/IO) still propagate
/// so the frontend can show a guided banner — and so a recognized-media URL
/// is never silently downloaded as its HTML page.
#[tauri::command]
pub async fn probe_media_url(
    core: State<'_, Core>,
    url: String,
) -> CommandResult<Option<ProbeResult>> {
    use unduhin_core::ytdlp::YtdlpError;
    match core.probe_media_url(&url).await {
        Ok(result) => Ok(Some(result)),
        Err(e @ (YtdlpError::Unsupported | YtdlpError::Timeout(_) | YtdlpError::Process { .. })) => {
            tracing::debug!(%url, error = %e, "probe_media_url: not media, falling back to HTTP engine");
            Ok(None)
        }
        Err(e) => Err(CommandError {
            message: e.to_string(),
        }),
    }
}

// Torrents (add-time metadata probe)

/// Input to [`fetch_torrent_metadata`] — mirrors the frontend `TorrentSource`
/// discriminant (`crates/core/src/download.rs::TorrentSource`, serde-tagged
/// `kind` snake_case). The dialog builds this from the user's magnet paste or
/// `.torrent` drop and asks for the file list before adding anything.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TorrentMetadataInput {
    Magnet { uri: String },
    File { path: String },
    InfoHash { hash: String },
}

impl TorrentMetadataInput {
    fn into_torrent_input(self) -> torrent::TorrentInput {
        match self {
            TorrentMetadataInput::Magnet { uri } => torrent::TorrentInput::Magnet(uri),
            TorrentMetadataInput::File { path } => {
                torrent::TorrentInput::TorrentFile(PathBuf::from(path))
            }
            TorrentMetadataInput::InfoHash { hash } => torrent::TorrentInput::InfoHash(hash),
        }
    }
}

/// One file in a probed torrent. `index` is its position in the torrent's
/// file list — the value the add-time picker feeds back as a
/// `TorrentMeta.selected_files` entry.
#[derive(Debug, Serialize)]
pub struct TorrentMetadataFile {
    pub index: usize,
    pub path: String,
    pub length: u64,
}

/// Result of [`fetch_torrent_metadata`] — the resolved `info_hash`, display
/// name, and every file in torrent order. Mirrors the frontend's
/// `TorrentMetadataResult`.
#[derive(Debug, Serialize)]
pub struct TorrentMetadataResult {
    pub info_hash: String,
    pub name: String,
    pub files: Vec<TorrentMetadataFile>,
}

/// Probe a magnet / `.torrent` / infohash for its file list WITHOUT
/// downloading (librqbit `list_only`) — backs the add-time file picker.
///
/// This spins up a SHORT-LIVED librqbit session distinct from the queue's
/// process-wide download session (which lives in `unduhin-core` and is not
/// exposed here): a `list_only` probe binds an OS-assigned ephemeral port,
/// fetches metadata, and is dropped when the command returns. DHT is read
/// from the `torrent_enable_dht` setting (default on) so trackerless magnets
/// can resolve; UPnP is left off for the probe since no inbound peers are
/// needed. The facade bounds the probe with its own timeout
/// ([`torrent::DEFAULT_METADATA_TIMEOUT`]); on elapse this returns an error
/// the dialog surfaces as "couldn't fetch torrent metadata".
#[tauri::command]
pub async fn fetch_torrent_metadata(
    core: State<'_, Core>,
    input: TorrentMetadataInput,
) -> CommandResult<TorrentMetadataResult> {
    // Probe through the queue's SHARED process-wide session (in unduhin-core),
    // NOT a second librqbit session here: a second session starts a second
    // PersistentDht on the same persisted UDP port, which fails to bind
    // ("error initializing persistent DHT") and would starve the real download
    // of peers. `list_only` only probes — it doesn't disturb active downloads.
    let ti = input.into_torrent_input();
    let cancel = tokio_util::sync::CancellationToken::new();
    let meta = core
        .fetch_torrent_metadata(ti, cancel)
        .await
        .map_err(|e| CommandError {
            message: e.to_string(),
        })?;

    Ok(TorrentMetadataResult {
        info_hash: meta.info_hash,
        name: meta.name,
        files: meta
            .files
            .into_iter()
            .enumerate()
            .map(|(index, f)| TorrentMetadataFile {
                index,
                path: f.path,
                length: f.length,
            })
            .collect(),
    })
}

#[tauri::command]
pub async fn tool_status(core: State<'_, Core>, tool: Tool) -> CommandResult<ToolStatus> {
    Ok(core.tool_status(tool).await)
}

/// Kick off an install or update of the given tool. Progress and the
/// final outcome arrive on the `unduhin:event` channel as
/// `tool_install_progress` / `tool_install_completed` /
/// `tool_install_failed`. The command's return value is the post-install
/// status (also emitted as `tool_install_completed`).
#[tauri::command]
pub async fn install_tool(core: State<'_, Core>, tool: Tool) -> CommandResult<ToolStatus> {
    core.install_tool(tool).await.map_err(|e| CommandError {
        message: e.to_string(),
    })
}

// Schedules

#[tauri::command]
pub async fn list_schedules(core: State<'_, Core>) -> CommandResult<Vec<Schedule>> {
    Ok(core.list_schedules().await?)
}

#[tauri::command]
pub async fn add_schedule(core: State<'_, Core>, input: NewSchedule) -> CommandResult<ScheduleId> {
    Ok(core.add_schedule(input).await?)
}

#[tauri::command]
pub async fn update_schedule(
    core: State<'_, Core>,
    id: ScheduleId,
    input: NewSchedule,
) -> CommandResult<()> {
    Ok(core.update_schedule(id, input).await?)
}

#[tauri::command]
pub async fn remove_schedule(core: State<'_, Core>, id: ScheduleId) -> CommandResult<()> {
    Ok(core.remove_schedule(id).await?)
}

/// Snapshot of the global quiet-hours window. Returns `{ active: false,
/// until: null }` when no `quiet_hours` schedule currently covers the
/// caller's local clock. The frontend `useNotifications` composable uses
/// this as the suppression gate.
#[tauri::command]
pub async fn get_quiet_hours_state(core: State<'_, Core>) -> CommandResult<QuietHoursState> {
    Ok(core.quiet_hours_state().await)
}

// Extension settings round-trip

/// Return the cached extension settings the pipe server holds. Returns
/// the canonical defaults when no extension has pushed yet — matches
/// the extension's own `DEFAULT_SETTINGS` byte-for-byte, so the Browser
/// panel can render without waiting for the first push.
#[tauri::command]
pub async fn get_extension_settings(_core: State<'_, Core>) -> CommandResult<ExtensionSettings> {
    #[cfg(windows)]
    {
        Ok(crate::pipe::cached_extension_settings()
            .await
            .unwrap_or_else(ExtensionSettings::defaults))
    }
    #[cfg(not(windows))]
    {
        Ok(ExtensionSettings::defaults())
    }
}

/// Apply a sparse patch to the cached extension settings and broadcast
/// the resulting full snapshot to every connected pipe client. The
/// extension's `applyServerSettings` writes through to
/// `chrome.storage.sync`, which fires `chrome.storage.onChanged` —
/// hot-applying the change across every consumer.
#[tauri::command]
pub async fn apply_extension_settings_patch(
    _core: State<'_, Core>,
    patch: SettingsPatch,
) -> CommandResult<ExtensionSettings> {
    #[cfg(windows)]
    {
        let current = crate::pipe::cached_extension_settings()
            .await
            .unwrap_or_else(ExtensionSettings::defaults);
        let mut next = current;
        next.apply(patch);
        crate::pipe::store_extension_settings(next.clone()).await;
        crate::pipe::broadcast_settings_changed(next.clone()).await;
        Ok(next)
    }
    #[cfg(not(windows))]
    {
        let _ = patch;
        Err(CommandError {
            message: "extension settings round-trip is Windows-only".into(),
        })
    }
}

/// Latest per-rule metrics snapshot the pipe server cached from the
/// extension's `chrome.alarms` push. Returns an empty list until the
/// first push lands. The panel re-queries on every
/// `CoreEvent::RuleMetricsUpdated`.
#[tauri::command]
pub async fn get_rule_metrics(_core: State<'_, Core>) -> CommandResult<Vec<RuleMetric>> {
    #[cfg(windows)]
    {
        Ok(crate::pipe::cached_rule_metrics().await)
    }
    #[cfg(not(windows))]
    {
        Ok(Vec::new())
    }
}

/// Resolve a pending `ask-first` prompt — broadcast the user's choice
/// to every connected pipe client. The extension matches by `id` and
/// resumes the corresponding handoff decision. Unknown `id`s are
/// tolerated (a stale click after a port rebind shouldn't error).
#[tauri::command]
pub async fn respond_handoff(
    _core: State<'_, Core>,
    id: String,
    decision: HandoffDecision,
) -> CommandResult<()> {
    #[cfg(windows)]
    {
        crate::pipe::broadcast_handoff_decision(id, decision).await;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (id, decision);
        Err(CommandError {
            message: "ask-first handoff is Windows-only".into(),
        })
    }
}

// Browser integration

#[derive(Debug, Serialize)]
pub struct BrowserIntegrationStatus {
    pub pipe: crate::browser_integration::PipeStatus,
    pub browsers: Vec<crate::browser_integration::BrowserStatus>,
    /// Last successful extension handoff timestamp, ISO-8601 UTC. `null`
    /// when no extension-sourced download has landed.
    pub last_handoff_at: Option<String>,
    /// Extension-sourced downloads added in the rolling last 7 days.
    pub handoffs_this_week: u64,
    /// Lifetime count of extension-sourced downloads.
    pub handoffs_total: u64,
}

/// Snapshot the Settings → Browser surface needs to render the top two
/// cards (Status + Browser extensions). Cheap — all of it is in-process
/// state or a handful of `RegOpenKeyExW` calls plus two `COUNT(*)`s on
/// the `downloads` table — so the composable can re-fetch on every
/// `pipe_listening` / `download_added` event without rate-limiting.
#[tauri::command]
pub async fn get_browser_integration_status(
    core: State<'_, Core>,
) -> CommandResult<BrowserIntegrationStatus> {
    let pool = core.pool();
    let week_cutoff = chrono::Utc::now() - chrono::Duration::days(7);
    let handoffs_total =
        unduhin_core::count_by_source(pool, DownloadSource::ExtensionPipe, None).await?;
    let handoffs_this_week =
        unduhin_core::count_by_source(pool, DownloadSource::ExtensionPipe, Some(week_cutoff))
            .await?;
    let last_handoff_at = unduhin_core::last_by_source(pool, DownloadSource::ExtensionPipe)
        .await?
        .map(|dt| dt.to_rfc3339());
    Ok(BrowserIntegrationStatus {
        pipe: crate::browser_integration::pipe_status(),
        browsers: crate::browser_integration::detect_installed_browsers(),
        last_handoff_at,
        handoffs_this_week,
        handoffs_total,
    })
}

#[derive(Debug, Serialize)]
pub struct PipeHandoffTest {
    /// Round-trip duration of the Ping → Pong exchange, in microseconds.
    pub round_trip_us: u128,
    /// The pipe path used for the test. Mirrors what
    /// `get_browser_integration_status().pipe.name` reports — surfaced
    /// here so the toast can echo the path without a second invoke.
    pub pipe: String,
}

/// Open a self-loopback to the in-app pipe server, send a framed
/// `Inbound::Ping`, and time how long it takes for the corresponding
/// `Outbound::Pong` to come back. Surfaced by the Settings → Browser
/// status card's "Test handoff" button — round-trip µs is rendered as
/// a toast so the user can sanity-check the bridge without leaving the
/// page.
#[tauri::command]
pub async fn test_pipe_handoff(_core: State<'_, Core>) -> CommandResult<PipeHandoffTest> {
    #[cfg(windows)]
    {
        use std::time::{Duration, Instant};
        use tokio::net::windows::named_pipe::ClientOptions;
        use unduhin_core::wire::framing::{read_frame, write_frame};
        use unduhin_core::wire::{Inbound, Outbound};

        let name = crate::pipe::pipe_name();
        let (_, listening) = crate::pipe::listening_snapshot();
        if !listening {
            return Err(CommandError {
                message: "pipe server is not listening yet".into(),
            });
        }

        // The pipe accepts one connection at a time per instance — the
        // server immediately creates a fresh server handle after
        // each accept, but on a hot start the client connect can race
        // that boundary. Retry briefly before giving up.
        let mut client = None;
        let connect_deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < connect_deadline {
            match ClientOptions::new().open(&name) {
                Ok(c) => {
                    client = Some(c);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
        let mut client = client.ok_or_else(|| CommandError {
            message: format!("could not connect to {name}"),
        })?;

        let start = Instant::now();
        let ping = serde_json::to_vec(&Inbound::Ping).map_err(|e| CommandError {
            message: format!("serialize ping: {e}"),
        })?;
        write_frame(&mut client, &ping)
            .await
            .map_err(|e| CommandError {
                message: format!("write ping: {e}"),
            })?;

        let frame =
            match tokio::time::timeout(Duration::from_secs(2), read_frame(&mut client)).await {
                Ok(Ok(Some(buf))) => buf,
                Ok(Ok(None)) => {
                    return Err(CommandError {
                        message: "pipe closed before pong arrived".into(),
                    })
                }
                Ok(Err(e)) => {
                    return Err(CommandError {
                        message: format!("read pong: {e}"),
                    })
                }
                Err(_) => {
                    return Err(CommandError {
                        message: "pong timed out after 2s".into(),
                    })
                }
            };
        let outbound: Outbound = serde_json::from_slice(&frame).map_err(|e| CommandError {
            message: format!("parse pong: {e}"),
        })?;
        if !matches!(outbound, Outbound::Pong) {
            return Err(CommandError {
                message: format!("expected pong, got {outbound:?}"),
            });
        }

        Ok(PipeHandoffTest {
            round_trip_us: start.elapsed().as_micros(),
            pipe: name,
        })
    }
    #[cfg(not(windows))]
    {
        Err(CommandError {
            message: "pipe handoff is only available on Windows".into(),
        })
    }
}
