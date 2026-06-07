//! # Unduhin core
//!
//! Persistence (SQLite), queue management, categories, settings, and an
//! event bus on top of the [`engine`] crate. The Tauri shell
//! wraps this crate's [`Core`] type and forwards [`CoreEvent`]s to the
//! frontend; the CLI exercises the same API from the command line.
//!
//! ## Quick tour
//!
//! - [`Core`] — the facade. Owns the [`sqlx::SqlitePool`], the event
//!   broadcast, and the running queue manager. All public mutations go
//!   through it.
//! - [`CoreEvent`] — the typed events emitted on a broadcast channel;
//!   subscribe via [`Core::subscribe`].
//! - [`DownloadRecord`], [`Status`], [`AddDownload`], [`DownloadFilter`]
//!   — types used by the downloads API.
//! - [`Category`], [`NewCategory`] — categories with extension-based
//!   auto-categorize rules.
//!
//! The engine crate is held strictly at arm's length: this crate calls
//! [`engine::download`] / [`engine::resume_at`] and forwards their
//! [`engine::ProgressEvent`]s as [`CoreEvent::ProgressUpdate`].

pub mod build_info;
pub mod category;
mod db;
pub mod download;
pub mod error;
pub mod event;
pub mod logging;
mod queue;
pub mod schedule;
mod secret;
pub mod settings;
pub mod speed;
pub mod tooling;
pub mod torrent_handoff;
pub mod wire;
pub mod ytdlp;

pub use category::{Category, CategoryId, NewCategory};
pub use download::{
    count_by_source, last_by_source, AddDownload, CategorySelector, DownloadFilter, DownloadId,
    DownloadKind, DownloadRecord, DownloadSource, Status, SwarmStats, TorrentFile, TorrentMeta,
    TorrentSource, ALL_STATUSES,
};
pub use error::{CoreError, Result};
pub use event::CoreEvent;
pub use schedule::{NewSchedule, QuietHoursState, Schedule, ScheduleId, ScheduleKind};
pub use settings::{parse_user_value, settings_keys, SettingValue};
pub use speed::TokenBucket;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{Local, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::queue::{QueueHandle, QueueManager};
use crate::schedule::SchedulesCache;

/// Default broadcast channel capacity for core events.
pub const EVENT_CHANNEL_CAPACITY: usize = 1024;

/// The facade for everything in this crate.
#[derive(Clone)]
pub struct Core {
    inner: Arc<CoreInner>,
}

struct CoreInner {
    pool: SqlitePool,
    events: broadcast::Sender<CoreEvent>,
    queue: Mutex<Option<QueueHandle>>,
    /// Shared with the queue manager so both layers see one cache. Reads
    /// dominate (queue tick + notifications gate); writes happen only on
    /// schedule CRUD, so an `RwLock` is the right shape.
    schedules: Arc<RwLock<SchedulesCache>>,
    /// Process-wide librqbit session, behind the `crates/torrent` facade.
    /// Lazily built on the first `DownloadKind::Torrent` claim (design §3.D):
    /// it owns one DHT, one listen socket, and one peer budget, so users who
    /// never touch torrents bind no socket and start no DHT. Shared with the
    /// queue manager (the worker resolves it from here). The
    /// `fetch_torrent_metadata` command does NOT use this session — it spins up
    /// a separate short-lived `list_only` probe session (ephemeral port, UPnP
    /// off) so an add-dialog metadata probe never contends with active
    /// downloads.
    torrent_engine: Arc<queue::TorrentEngineCell>,
}

impl Core {
    /// Open a `Core` against a SQLite database at `db_path`, applying
    /// migrations on connect. The file is created if it does not exist.
    pub async fn open(db_path: impl AsRef<Path>) -> Result<Self> {
        let db_path = db_path.as_ref();
        if let Some(parent) = db_path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(CoreError::io)?;
            }
        }
        let opts = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(5))
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await?;
        Self::from_pool(pool).await
    }

    /// Open an in-memory database; useful for tests.
    pub async fn open_in_memory() -> Result<Self> {
        let opts = SqliteConnectOptions::new()
            .in_memory(true)
            .busy_timeout(std::time::Duration::from_secs(5))
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;
        Self::from_pool(pool).await
    }

    async fn from_pool(pool: SqlitePool) -> Result<Self> {
        db::migrate(&pool).await?;
        let (tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        // On restart, any download stuck in `active` *or* `muxing` was
        // interrupted mid-flight (the latter when a crash hit a yt-dlp
        // download during its second-stream/ffmpeg-merge phase); flip it
        // back to `queued` so the queue manager picks it up. Without
        // `muxing` here the row would have no in-memory worker and would
        // never be re-claimed, leaving it stuck on "Muxing" forever.
        sqlx::query("UPDATE downloads SET status = 'queued' WHERE status IN ('active', 'muxing')")
            .execute(&pool)
            .await?;
        let schedules = SchedulesCache::load(&pool).await?;
        Ok(Self {
            inner: Arc::new(CoreInner {
                pool,
                events: tx,
                queue: Mutex::new(None),
                schedules: Arc::new(RwLock::new(schedules)),
                torrent_engine: Arc::new(queue::TorrentEngineCell::new()),
            }),
        })
    }

    /// Subscribe to the event stream. Lagged subscribers will receive
    /// [`broadcast::error::RecvError::Lagged`] but the producer is never
    /// blocked.
    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.inner.events.subscribe()
    }

    /// Publish an arbitrary [`CoreEvent`] onto the bus. Intended for
    /// non-core producers (the Tauri shell's pipe server, in particular)
    /// that observe out-of-band signals like "named-pipe listener
    /// bound". Errors are swallowed — same drop policy as the rest of
    /// the internal `.send()` sites.
    pub fn publish_event(&self, event: CoreEvent) {
        let _ = self.inner.events.send(event);
    }

    /// Reference to the underlying pool. The Tauri layer may want
    /// this for read-only ad-hoc queries; mutations should still go
    /// through the typed methods below.
    pub fn pool(&self) -> &SqlitePool {
        &self.inner.pool
    }

    /// Start the queue manager. Idempotent — subsequent calls are no-ops.
    pub async fn start(&self) -> Result<()> {
        let mut slot = self.inner.queue.lock().await;
        if slot.is_some() {
            return Ok(());
        }
        let handle = QueueManager::spawn(
            self.inner.pool.clone(),
            self.inner.events.clone(),
            self.inner.schedules.clone(),
            self.inner.torrent_engine.clone(),
        )
        .await;
        *slot = Some(handle);
        Ok(())
    }

    /// Shut the queue manager down: pause any actively-running transfers
    /// (preserving their sidecars for resume) and flush state. Idempotent.
    pub async fn shutdown(&self) -> Result<()> {
        let handle = {
            let mut slot = self.inner.queue.lock().await;
            slot.take()
        };
        if let Some(h) = handle {
            h.shutdown().await;
        }
        Ok(())
    }

    // Downloads

    /// Add a new download. Returns the assigned id. Emits
    /// [`CoreEvent::DownloadAdded`].
    pub async fn add_download(&self, input: AddDownload) -> Result<DownloadId> {
        let record = download::insert(&self.inner.pool, input).await?;
        let id = record.id;
        let _ = self.inner.events.send(CoreEvent::DownloadAdded {
            id,
            snapshot: Box::new(record),
        });
        self.poke_queue().await;
        Ok(id)
    }

    /// Resolve a torrent's metadata (file list) WITHOUT downloading — backs the
    /// add-dialog file picker. Uses the queue's SHARED process-wide librqbit
    /// session (built on first use from the `torrent_*` settings), NOT a
    /// throwaway one: a second session starts a second `PersistentDht` on the
    /// same persisted UDP port, which fails to bind ("error initializing
    /// persistent DHT") and would starve the real download of peers. `list_only`
    /// only probes — it adds nothing to the session and doesn't disturb active
    /// downloads.
    pub async fn fetch_torrent_metadata(
        &self,
        input: torrent::TorrentInput,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<torrent::TorrentMetadata> {
        let engine = self
            .inner
            .torrent_engine
            .get_or_init(&self.inner.pool)
            .await?;
        let meta = engine
            .fetch_metadata(&input, cancel)
            .await
            .map_err(engine::EngineError::from)?;
        Ok(meta)
    }

    pub async fn list_downloads(&self, filter: DownloadFilter) -> Result<Vec<DownloadRecord>> {
        download::list(&self.inner.pool, filter).await
    }

    pub async fn get_download(&self, id: DownloadId) -> Result<DownloadRecord> {
        download::get(&self.inner.pool, id).await
    }

    pub async fn pause(&self, id: DownloadId) -> Result<()> {
        self.change_status(id, &[Status::Queued, Status::Active], Status::Paused)
            .await
    }

    pub async fn resume(&self, id: DownloadId) -> Result<()> {
        self.change_status(id, &[Status::Paused, Status::Failed], Status::Queued)
            .await
    }

    pub async fn cancel(&self, id: DownloadId) -> Result<()> {
        self.change_status(
            id,
            &[
                Status::Queued,
                Status::Active,
                Status::Muxing,
                Status::Paused,
            ],
            Status::Cancelled,
        )
        .await
    }

    pub async fn retry(&self, id: DownloadId) -> Result<()> {
        self.change_status(id, &[Status::Failed, Status::Cancelled], Status::Queued)
            .await
    }

    /// Remove a download row. When `delete_data` is true, the file on
    /// disk (and the engine sidecar if any) are deleted too. Errors
    /// reading the file are returned so the UI can surface them, but
    /// the DB row is removed regardless — the user asked to forget the
    /// download and we honour that even if the file is locked.
    pub async fn remove(&self, id: DownloadId, delete_data: bool) -> Result<()> {
        // Snapshot the row before deleting so we know whether it's a torrent
        // (and its info_hash) — needed to forget it from the live librqbit
        // session below.
        let record = download::get(&self.inner.pool, id).await.ok();
        // Cancel first so the queue manager drops any active handle and
        // flushes the sidecar, then delete the row.
        let _ = self.cancel(id).await;
        // Synchronously drain the in-memory worker before touching the
        // file on disk. The `cancel` call above only flips the DB row;
        // the worker observes that on its next tick and takes additional
        // time to actually exit. If we delete now, an engine retry's
        // `open_for_segment` (which uses `create(true)`) — or yt-dlp's
        // `.part` finalization on its way out — can race the delete and
        // leave a 0-byte ghost file behind.
        {
            let slot = self.inner.queue.lock().await;
            if let Some(q) = slot.as_ref() {
                q.cancel_and_wait(id).await;
            }
        }
        // For torrents, remove it from the live librqbit session so a later
        // re-add starts FRESH. Without this, librqbit keeps it managed in
        // memory and `add_torrent` returns `AlreadyManaged`, resuming from where
        // it left off (e.g. 50%) even after the row and files are gone. Only
        // when a session already exists; `delete_data` lets librqbit drop the
        // content + fastresume too.
        if let Some(rec) = record
            .as_ref()
            .filter(|r| r.kind == download::DownloadKind::Torrent)
        {
            if let Some(meta) = rec.torrent.as_ref().filter(|m| !m.info_hash.is_empty()) {
                if let Some(engine) = self.inner.torrent_engine.get() {
                    let _ = engine.forget(&meta.info_hash, delete_data).await;
                }
            }
        }
        let outcome = download::remove(&self.inner.pool, id, delete_data).await?;
        let _ = self.inner.events.send(CoreEvent::Removed { id });
        self.poke_queue().await;
        if let Some(err) = outcome.data_error {
            // The row is gone but the file wasn't deletable. Treat this
            // as a soft error so the UI can toast it.
            return Err(CoreError::Io(std::io::Error::other(format!(
                "could not delete file: {err}"
            ))));
        }
        Ok(())
    }

    pub async fn set_priority(&self, id: DownloadId, priority: i64) -> Result<()> {
        download::set_priority(&self.inner.pool, id, priority).await?;
        self.poke_queue().await;
        Ok(())
    }

    /// Reassign a download to a different category (or clear the
    /// assignment with `None`). Validates that the row exists and — when
    /// `category_id` is `Some` — that the category exists. Emits
    /// [`CoreEvent::CategoryChanged`] so the sidebar counts and any open
    /// detail pane update live.
    pub async fn set_category(
        &self,
        id: DownloadId,
        category_id: Option<CategoryId>,
    ) -> Result<()> {
        if let Some(cid) = category_id {
            // Surfaces `CategoryNotFound` if the id is bogus; we don't
            // want to silently strip the assignment on a typo.
            let _ = category::get(&self.inner.pool, cid).await?;
        }
        download::set_category(&self.inner.pool, id, category_id).await?;
        let _ = self
            .inner
            .events
            .send(CoreEvent::CategoryChanged { id, category_id });
        Ok(())
    }

    /// Change the worker-pool size for a download. Bounded 1..=32 (see
    /// [`engine::MAX_SEGMENTS`]). Persists the intent to the DB and, when
    /// the download is actively transferring, dispatches a control
    /// message to the engine to apply the split/join live.
    ///
    /// Rejects when:
    /// - `n` is outside the allowed bounds: `InvalidArgument`.
    /// - the download is in a terminal status (`completed`, `failed`,
    ///   `cancelled`): `InvalidTransition`.
    /// - the download is mid-flight on a server that doesn't honor byte
    ///   ranges (`Accept-Ranges: none`): `NotResumable`. (Queued or
    ///   paused downloads with no sidecar yet are accepted — the engine
    ///   will fall back to a single segment at start time if needed.)
    pub async fn set_segments(&self, id: DownloadId, n: usize) -> Result<()> {
        if !(engine::MIN_SEGMENTS..=engine::MAX_SEGMENTS).contains(&n) {
            return Err(CoreError::InvalidArgument(format!(
                "segments must be {}..={}",
                engine::MIN_SEGMENTS,
                engine::MAX_SEGMENTS
            )));
        }
        let record = download::get(&self.inner.pool, id).await?;
        if record.status.is_terminal() {
            return Err(CoreError::InvalidTransition {
                id,
                from: record.status.to_string(),
                to: format!("set_segments({n})"),
            });
        }
        // Persist intent. Active engines will read the live count via
        // the control channel below; queued / paused rows pick up the
        // new value on next start.
        download::update_segments(&self.inner.pool, id, n as u32).await?;

        if let Some(ctrl) = self.control_for(id).await {
            let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
            if ctrl
                .send(engine::Control::SetSegments { n, ack: ack_tx })
                .await
                .is_err()
            {
                return Err(CoreError::ControlClosed);
            }
            match ack_rx.await {
                Ok(Ok(())) => {}
                Ok(Err(engine_err)) => {
                    // Engine refusing on non-resumable downloads is the
                    // common case — map it to the dedicated variant so
                    // the UI can surface a specific message.
                    let msg = engine_err.to_string();
                    if msg.contains("not resumable") {
                        return Err(CoreError::NotResumable);
                    }
                    return Err(CoreError::Engine(engine_err));
                }
                Err(_) => return Err(CoreError::ControlClosed),
            }
        }

        let _ = self.inner.events.send(CoreEvent::SegmentsChanged { id, n });
        Ok(())
    }

    async fn control_for(
        &self,
        id: DownloadId,
    ) -> Option<tokio::sync::mpsc::Sender<engine::Control>> {
        let slot = self.inner.queue.lock().await;
        if let Some(h) = slot.as_ref() {
            h.control_for(id).await
        } else {
            None
        }
    }

    /// HEAD-probe `url` and return the best-effort derived filename.
    /// Used by AddUrlDialog to prefill the rename field. Returns
    /// `Ok(None)` when nothing usable could be derived; the caller can
    /// treat that as "ask the user to type a name."
    pub async fn preview_filename(&self, url: &str) -> Result<Option<String>> {
        let parsed: url::Url = url
            .parse()
            .map_err(|e: url::ParseError| CoreError::InvalidArgument(e.to_string()))?;

        let (connect, read) = (
            settings::get(
                &self.inner.pool,
                settings::settings_keys::CONNECT_TIMEOUT_SECS,
            )
            .await?
            .and_then(|v| v.as_u64())
            .unwrap_or(15)
            .min(5),
            settings::get(&self.inner.pool, settings::settings_keys::READ_TIMEOUT_SECS)
                .await?
                .and_then(|v| v.as_u64())
                .unwrap_or(60)
                .min(5),
        );
        let user_agent = settings::get(&self.inner.pool, settings::settings_keys::USER_AGENT)
            .await?
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .filter(|s| !s.is_empty());

        let client = engine::http::build_client(
            std::time::Duration::from_secs(connect),
            std::time::Duration::from_secs(read),
            user_agent.as_deref(),
            &[],
        )
        .map_err(CoreError::Engine)?;

        match engine::probe(&client, &parsed).await {
            Ok(info) => Ok(info.filename_hint),
            Err(e) => {
                tracing::debug!(error = %e, "preview_filename probe failed");
                Ok(None)
            }
        }
    }

    async fn change_status(
        &self,
        id: DownloadId,
        allowed_from: &[Status],
        to: Status,
    ) -> Result<()> {
        let from = download::transition_status(&self.inner.pool, id, allowed_from, to).await?;
        let _ = self
            .inner
            .events
            .send(CoreEvent::StatusChanged { id, from, to });
        self.poke_queue().await;
        Ok(())
    }

    async fn poke_queue(&self) {
        let slot = self.inner.queue.lock().await;
        if let Some(h) = slot.as_ref() {
            h.poke();
        }
    }

    // Categories

    pub async fn list_categories(&self) -> Result<Vec<Category>> {
        category::list(&self.inner.pool).await
    }

    pub async fn get_category(&self, id: CategoryId) -> Result<Category> {
        category::get(&self.inner.pool, id).await
    }

    pub async fn find_category_by_name(&self, name: &str) -> Result<Option<Category>> {
        category::find_by_name(&self.inner.pool, name).await
    }

    pub async fn add_category(&self, input: NewCategory) -> Result<CategoryId> {
        category::insert(&self.inner.pool, input).await
    }

    pub async fn update_category(&self, id: CategoryId, input: NewCategory) -> Result<()> {
        category::update(&self.inner.pool, id, input).await
    }

    pub async fn remove_category(&self, id: CategoryId) -> Result<()> {
        category::remove(&self.inner.pool, id).await
    }

    /// Rewrite the category display order. The supplied id set must equal
    /// the current id set.
    pub async fn set_category_order(&self, ids: Vec<CategoryId>) -> Result<()> {
        category::set_order(&self.inner.pool, &ids).await
    }

    // Settings

    pub async fn get_setting(&self, key: &str) -> Result<Option<SettingValue>> {
        settings::get(&self.inner.pool, key).await
    }

    pub async fn set_setting(&self, key: &str, value: SettingValue) -> Result<()> {
        settings::set(&self.inner.pool, key, &value).await?;
        let _ = self.inner.events.send(CoreEvent::SettingChanged {
            key: key.to_string(),
        });
        self.poke_queue().await;
        Ok(())
    }

    pub async fn all_settings(&self) -> Result<std::collections::HashMap<String, SettingValue>> {
        settings::all(&self.inner.pool).await
    }

    // Media (yt-dlp / ffmpeg)

    /// Probe a URL with yt-dlp and return its format catalogue, or a
    /// typed [`ytdlp::YtdlpError`] when the URL isn't supported, yt-dlp
    /// isn't installed, or the probe times out.
    pub async fn probe_media_url(
        &self,
        url: &str,
    ) -> std::result::Result<ytdlp::ProbeResult, ytdlp::YtdlpError> {
        let binary = tooling::resolve_path(tooling::Tool::YtDlp, &self.inner.pool)
            .await
            .ok_or(ytdlp::YtdlpError::NotInstalled)?;
        let timeout_ms = settings::get(
            &self.inner.pool,
            settings::settings_keys::YTDLP_PROBE_TIMEOUT_MS,
        )
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_u64())
        .unwrap_or(3000);
        ytdlp::probe(url, &binary, std::time::Duration::from_millis(timeout_ms)).await
    }

    /// Report whether `tool` is installed and at what version.
    pub async fn tool_status(&self, tool: tooling::Tool) -> tooling::ToolStatus {
        tooling::status(tool, &self.inner.pool).await
    }

    /// Kick off an install/update of `tool` in the background. Progress
    /// events fire on the core event bus (`tool_install_*` variants);
    /// awaiting this returns the post-install [`tooling::ToolStatus`].
    pub async fn install_tool(
        &self,
        tool: tooling::Tool,
    ) -> std::result::Result<tooling::ToolStatus, tooling::ToolingError> {
        let cancel = engine::CancellationToken::new();
        tooling::install_or_update(tool, &self.inner.pool, self.inner.events.clone(), cancel).await
    }

    // Schedules

    /// All persisted schedule rows, oldest first.
    pub async fn list_schedules(&self) -> Result<Vec<Schedule>> {
        schedule::list_all(&self.inner.pool).await
    }

    /// Create a new schedule row. Validation rules:
    /// - `start_at` / `after_queue` require a `download_id`.
    /// - `quiet_hours` rejects a `download_id` (the row is global).
    /// - `start_at.start_iso` must be RFC3339; `quiet_hours.start_iso` and
    ///   `quiet_hours.end_iso` must be `"HH:MM"` (24-hour, local TZ).
    pub async fn add_schedule(&self, input: NewSchedule) -> Result<ScheduleId> {
        let id = schedule::insert(&self.inner.pool, input).await?;
        self.after_schedules_changed().await;
        Ok(id)
    }

    pub async fn update_schedule(&self, id: ScheduleId, input: NewSchedule) -> Result<()> {
        schedule::update(&self.inner.pool, id, input).await?;
        self.after_schedules_changed().await;
        Ok(())
    }

    pub async fn remove_schedule(&self, id: ScheduleId) -> Result<()> {
        schedule::remove(&self.inner.pool, id).await?;
        self.after_schedules_changed().await;
        Ok(())
    }

    /// Snapshot of the current quiet-hours window. Used by the frontend
    /// notifications gate and the tray badge gate.
    pub async fn quiet_hours_state(&self) -> QuietHoursState {
        let now = Local::now();
        let cache = self.inner.schedules.read().await;
        if !cache.quiet_hours_active(now) {
            return QuietHoursState {
                active: false,
                until: None,
            };
        }
        let until = cache
            .quiet_hours_active_until(now)
            .map(|t| t.with_timezone(&Utc).to_rfc3339());
        QuietHoursState {
            active: true,
            until,
        }
    }

    /// Hook for the queue manager — exposed at crate level so
    /// `queue.rs` can refresh the cache and emit the event when it
    /// reaps a fired `start_at` row.
    pub(crate) async fn after_schedules_changed(&self) {
        // Best-effort reload; a transient DB error here just means the
        // cache stays slightly stale until the next mutation.
        if let Err(e) = self
            .inner
            .schedules
            .write()
            .await
            .reload(&self.inner.pool)
            .await
        {
            tracing::warn!(error = %e, "schedules cache reload failed");
        }
        let _ = self.inner.events.send(CoreEvent::SchedulesChanged);
        self.poke_queue().await;
    }
}

/// Conventional location for the user's Unduhin database when callers
/// don't supply their own path. Returns
/// `%LOCALAPPDATA%/unduhin/unduhin.db` on Windows.
pub fn default_db_path() -> Option<PathBuf> {
    directories_root().map(|dir| dir.join("unduhin.db"))
}

#[doc(hidden)]
pub fn directories_root() -> Option<PathBuf> {
    // We avoid pulling in `directories` here to keep deps tight; mimic the
    // behavior we want for Windows local data.
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            return Some(Path::new(&local).join("unduhin"));
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return Some(
            Path::new(&home)
                .join(".local")
                .join("share")
                .join("unduhin"),
        );
    }
    None
}

#[doc(hidden)]
pub fn now_iso() -> String {
    Utc::now().to_rfc3339()
}
