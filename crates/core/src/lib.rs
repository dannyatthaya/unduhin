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
mod motw;
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
    DownloadKind, DownloadRecord, DownloadSource, ErrorKind, RefreshOutcome, Status, SwarmStats,
    TorrentFile, TorrentMeta, TorrentSource, ALL_STATUSES,
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
    /// `Arc` so callers can clone the handle out and **release the mutex
    /// before awaiting on it**. `remove` awaits a worker's full exit; held
    /// across that await, this lock also blocks `poke_queue`, `start`, and
    /// `shutdown`, so one slow worker used to wedge the entire app.
    queue: Mutex<Option<Arc<QueueHandle>>>,
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
    /// Process-wide byte-rate limiter for the global speed cap. One bucket is
    /// shared by every HTTP worker (the queue hands each a clone), so the cap
    /// is total throughput. Rate `0` = unlimited. Seeded from
    /// `global_speed_limit_bps` at startup and updated live by [`Core::set_setting`].
    rate_limiter: Arc<engine::TokenBucket>,
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
        // First-run backfill: give every category a concrete download folder
        // (`<Downloads>/<Name>`, plain `<Downloads>` for "Other") so the
        // settings UI shows real paths and nothing resolves to the CWD.
        category::seed_default_folders(&pool).await?;
        // Rows captured while the output fallback was still the process CWD
        // may point into C:\WINDOWS\system32 — rewrite them before the queue
        // gets a chance to retry them at the unwritable location.
        download::repair_unwritable_output_paths(&pool).await?;
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
        // Seed the global speed limiter from the persisted setting so the cap
        // is in force from the first byte after a restart, before any
        // `set_setting` arrives. `0` (or unset) = unlimited.
        let speed_limit_bps = settings::get(&pool, settings::settings_keys::GLOBAL_SPEED_LIMIT_BPS)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        Ok(Self {
            inner: Arc::new(CoreInner {
                pool,
                events: tx,
                queue: Mutex::new(None),
                schedules: Arc::new(RwLock::new(schedules)),
                torrent_engine: Arc::new(queue::TorrentEngineCell::new()),
                rate_limiter: engine::TokenBucket::new(speed_limit_bps),
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
            self.inner.rate_limiter.clone(),
        )
        .await;
        *slot = Some(Arc::new(handle));
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

    /// Stop a download.
    ///
    /// `Muxing` is an allowed source state. It reads oddly — pausing a
    /// merge kills the ffmpeg doing it, and resuming re-runs the merge
    /// from the intermediate streams yt-dlp still has on disk — but the
    /// alternative is worse: with no user-facing cancel, a row that
    /// reaches `Muxing` and stalls there has no way out except deleting
    /// it. That was a live complaint back when the pump *inferred*
    /// `Muxing` from a byte-counter dip and stranded still-downloading
    /// HLS rows in it.
    pub async fn pause(&self, id: DownloadId) -> Result<()> {
        self.change_status(
            id,
            &[Status::Queued, Status::Active, Status::Muxing],
            Status::Paused,
        )
        .await
    }

    pub async fn resume(&self, id: DownloadId) -> Result<()> {
        self.change_status(id, &[Status::Paused, Status::Failed], Status::Queued)
            .await
    }

    /// Restart / retry a stopped download.
    ///
    /// - `Failed` → re-queue and **resume** from the partial (sidecar) — the
    ///   long-standing retry behaviour.
    /// - `Completed` → re-download **from scratch**: drop the resume sidecar and
    ///   zero the progress columns first (so the worker does a fresh GET that
    ///   overwrites the file rather than instantly re-completing), then re-queue.
    ///
    /// There is no user-facing "cancel" — pausing stops a download and deleting
    /// discards it — so `Cancelled` is no longer a retry source.
    pub async fn retry(&self, id: DownloadId) -> Result<()> {
        let was_completed = matches!(
            download::get(&self.inner.pool, id).await.map(|r| r.status),
            Ok(Status::Completed)
        );
        if was_completed {
            download::reset_for_restart(&self.inner.pool, id).await?;
        }
        self.change_status(id, &[Status::Failed, Status::Completed], Status::Queued)
            .await?;
        if was_completed {
            // The row was at 100 %; reset the live bar so it doesn't sit in the
            // queue showing a finished download.
            let _ = self.inner.events.send(CoreEvent::ProgressUpdate {
                id,
                downloaded: 0,
                total: None,
                speed_bps: 0.0,
                eta: None,
            });
        }
        Ok(())
    }

    /// Point a stopped download at a new URL, with new captured headers.
    ///
    /// The recovery path for an expired link. A row that failed with
    /// [`ErrorKind::ExpiredAuth`] cannot be fixed by [`Core::retry`] — retry
    /// re-queues the same dead URL with the same stale cookies and fails
    /// identically. This replaces both, and keeps the bytes already on disk
    /// whenever the replacement URL serves the same body.
    ///
    /// Scoped to [`DownloadKind::Http`]. Media rows re-resolve their URL on
    /// every attempt through `MediaInfo.original_url`, and torrents have no
    /// URL to refresh.
    ///
    /// `url` is untrusted — in the extension flow it comes straight from the
    /// browser — so it is parsed and scheme-checked before it reaches a
    /// request.
    ///
    /// Returns [`RefreshOutcome::SourceChanged`] **without changing anything**
    /// when the new URL serves a different body than the partial file was built
    /// from. Call again with `force_restart` to discard the partial and start
    /// over.
    pub async fn refresh_source(
        &self,
        id: DownloadId,
        url: &str,
        headers: Option<Vec<(String, String)>>,
        force_restart: bool,
    ) -> Result<RefreshOutcome> {
        let record = download::get(&self.inner.pool, id).await?;

        if record.kind != download::DownloadKind::Http {
            return Err(CoreError::InvalidArgument(format!(
                "only direct HTTP downloads can have their link refreshed (this row is {})",
                record.kind
            )));
        }
        // Refreshing a running row would swap the URL under a live worker.
        if !matches!(record.status, Status::Failed | Status::Paused) {
            return Err(CoreError::InvalidTransition {
                id,
                from: record.status.to_string(),
                to: "refreshed".to_string(),
            });
        }

        let parsed: url::Url = url
            .parse()
            .map_err(|e| CoreError::InvalidArgument(format!("invalid refresh url: {e}")))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(CoreError::InvalidArgument(format!(
                "refresh url must be http or https, got {}",
                parsed.scheme()
            )));
        }

        // Probe with the NEW headers, using the same timeout / user-agent
        // settings the worker will use. A probe that passes under different
        // settings than the download would be worthless.
        let (connect_timeout, read_timeout) = queue::timeouts(&self.inner.pool).await;
        let ua = queue::user_agent_setting(&self.inner.pool).await;
        let header_slice = headers.as_deref().unwrap_or(&[]);
        let client =
            engine::http::build_client(connect_timeout, read_timeout, ua.as_deref(), header_slice)?;
        // A dead replacement fails HERE, before the row is touched, so the
        // caller can say "that link is expired too" instead of queueing
        // another identical failure.
        let remote = engine::probe(&client, &parsed).await?;

        // Does the new URL serve the same body the partial file came from?
        // `matches_remote` is the exact check `resume_at` runs, so agreeing
        // with it here means the worker will agree with us later.
        let sidecar = engine::Meta::sidecar_path(&record.output_path);
        let mut meta = engine::Meta::load(&sidecar).await.ok();
        let resumable = match meta.as_ref() {
            Some(m) => m.matches_remote(
                remote.etag.as_deref(),
                remote.last_modified.as_deref(),
                remote.content_length,
            ),
            // No sidecar means there is nothing to resume. That is a restart,
            // not a mismatch.
            None => false,
        };

        if meta.is_some() && !resumable && !force_restart {
            return Ok(RefreshOutcome::SourceChanged {
                old_bytes: meta.as_ref().and_then(|m| m.total_bytes),
                new_bytes: remote.content_length,
            });
        }

        if !resumable {
            // Drops the sidecar and zeroes the progress columns, so the row
            // cannot show a bar it has no bytes behind.
            download::reset_for_restart(&self.inner.pool, id).await?;
        }

        download::update_source(&self.inner.pool, id, parsed.as_str(), headers.as_deref()).await?;

        if resumable {
            // The DB column is not what the worker resumes against.
            // `engine::resume_at_with_control` takes no URL argument at all —
            // it reads `Meta.url` out of the sidecar. Rewriting only the row
            // would leave the worker fetching the dead link and failing with
            // the same 403 that started this. Write the sidecar BEFORE the
            // row goes back to `Queued`, so the queue can never observe a
            // refreshed row pointing at a stale sidecar.
            if let Some(m) = meta.as_mut() {
                m.url = parsed.to_string();
                m.save(&sidecar).await?;
            }
            // `update_source` cleared the validators along with the URL. Put
            // the freshly probed ones back so the DB mirrors the sidecar the
            // worker is about to validate against.
            download::persist_progress(
                &self.inner.pool,
                id,
                record.downloaded_bytes,
                remote.content_length,
                remote.etag.as_deref(),
                remote.last_modified.as_deref(),
                None,
            )
            .await?;
        }

        self.change_status(id, &[Status::Failed, Status::Paused], Status::Queued)
            .await?;

        if resumable {
            Ok(RefreshOutcome::Resumed {
                downloaded_bytes: record.downloaded_bytes,
                total_bytes: remote.content_length,
            })
        } else {
            // Drop the live bar to zero straight away, the same way `retry`
            // does for a restarted completed row.
            let _ = self.inner.events.send(CoreEvent::ProgressUpdate {
                id,
                downloaded: 0,
                total: remote.content_length,
                speed_bps: 0.0,
                eta: None,
            });
            Ok(RefreshOutcome::Restarted)
        }
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
        // Stop the download before touching disk. An active worker is halted
        // (and its sidecar flushed) by `cancel_and_wait` below; a not-yet-
        // started `queued` row is parked as `paused` here so the manager can't
        // claim it in the window before the DELETE. (There is no user-facing
        // "cancel" — remove fully discards the row.)
        let _ =
            download::transition_status(&self.inner.pool, id, &[Status::Queued], Status::Paused)
                .await;
        // Synchronously drain the in-memory worker before touching the
        // file on disk. The `cancel` call above only flips the DB row;
        // the worker observes that on its next tick and takes additional
        // time to actually exit. If we delete now, an engine retry's
        // `open_for_segment` (which uses `create(true)`) — or yt-dlp's
        // `.part` finalization on its way out — can race the delete and
        // leave a 0-byte ghost file behind.
        //
        // Clone the handle and drop the guard first: this await is the
        // longest one in `Core`, and holding the queue lock across it
        // stalls every other queue operation behind a single worker.
        let queue = self.inner.queue.lock().await.clone();
        if let Some(q) = queue {
            q.cancel_and_wait(id).await;
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
        let relocated = download::set_category(&self.inner.pool, id, category_id).await?;
        let _ = self
            .inner
            .events
            .send(CoreEvent::CategoryChanged { id, category_id });
        // The file may have been moved into the new category's folder; tell the
        // UI so the detail pane and "open folder" point at the real location.
        if let Some((filename, output_path)) = relocated {
            let _ = self.inner.events.send(CoreEvent::PathsChanged {
                id,
                filename,
                output_path: output_path.to_string_lossy().into_owned(),
            });
        }
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
        // Apply the global speed cap live across every backend. HTTP workers
        // share the token bucket; the torrent session has its own librqbit
        // rate limiter; yt-dlp reads the value at next spawn (its `--limit-rate`
        // can't change mid-process). `0` / unset = unlimited. No restart needed.
        if key == settings::settings_keys::GLOBAL_SPEED_LIMIT_BPS {
            let bps = value.as_u64().unwrap_or(0);
            self.inner.rate_limiter.set_rate(bps).await;
            if let Some(engine) = self.inner.torrent_engine.get() {
                engine.set_download_limit(bps);
            }
        }
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

    /// Resolve the configured yt-dlp binary, the configured probe timeout,
    /// and the user's browser-impersonation toggle. Shared by
    /// [`probe_media_url`] and [`probe_media_formats`] so the two entry
    /// points (`ProbeResult` for the desktop UI, `MediaFormat` for the
    /// extension pipe) don't duplicate the binary-resolution /
    /// settings-lookup boilerplate.
    ///
    /// The impersonation lookup mirrors `queue.rs`'s `YtdlpJob`
    /// construction (identical key, identical `true` default when the row
    /// is absent) — `download()` and the probe paths must honor the same
    /// user-facing toggle the same way. This was previously missing here
    /// entirely: `probe_raw` inferred impersonation from referrer presence
    /// alone, which meant a user who explicitly disabled impersonation in
    /// Settings → Media still got impersonated probes. Read explicitly and
    /// passed down instead — see `ytdlp::mod`'s internal
    /// `wants_impersonation_probe`.
    ///
    /// [`probe_media_url`]: Core::probe_media_url
    /// [`probe_media_formats`]: Core::probe_media_formats
    async fn resolve_ytdlp_for_probe(
        &self,
    ) -> std::result::Result<(PathBuf, std::time::Duration, bool), ytdlp::YtdlpError> {
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
        let impersonate =
            settings::get(&self.inner.pool, settings::settings_keys::YTDLP_IMPERSONATE)
                .await
                .ok()
                .flatten()
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
        Ok((
            binary,
            std::time::Duration::from_millis(timeout_ms),
            impersonate,
        ))
    }

    /// Probe a URL with yt-dlp and return its format catalogue, or a
    /// typed [`ytdlp::YtdlpError`] when the URL isn't supported, yt-dlp
    /// isn't installed, or the probe times out.
    ///
    /// `referrer` is forwarded to yt-dlp's `--referer` when `Some`. The
    /// Tauri command that's the sole caller today always passes `None` —
    /// the paste-a-URL dialog has no browser referrer to source one from.
    /// Impersonation is read from the `ytdlp_impersonate` setting (see
    /// [`resolve_ytdlp_for_probe`]) and requires both that setting AND a
    /// referrer to actually be attempted — see [`ytdlp::probe`].
    ///
    /// [`resolve_ytdlp_for_probe`]: Core::resolve_ytdlp_for_probe
    pub async fn probe_media_url(
        &self,
        url: &str,
        referrer: Option<&str>,
    ) -> std::result::Result<ytdlp::ProbeResult, ytdlp::YtdlpError> {
        let (binary, timeout, impersonate) = self.resolve_ytdlp_for_probe().await?;
        ytdlp::probe(url, &binary, timeout, referrer, impersonate).await
    }

    /// Same resolution/timeout/impersonation plumbing as
    /// [`probe_media_url`], but returns the extension-facing
    /// [`wire::MediaFormat`] list via [`ytdlp::probe_media_formats`]
    /// instead of the internal [`ytdlp::ProbeResult`]. The sole caller is
    /// the pipe server's `Inbound::ProbeMedia` handler — this
    /// intentionally never goes through a Tauri command (see
    /// [`wire::MediaFormat`]'s doc comment).
    pub async fn probe_media_formats(
        &self,
        url: &str,
        referrer: Option<&str>,
    ) -> std::result::Result<Vec<wire::MediaFormat>, ytdlp::YtdlpError> {
        let (binary, timeout, impersonate) = self.resolve_ytdlp_for_probe().await?;
        ytdlp::probe_media_formats(url, &binary, timeout, referrer, impersonate).await
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

    /// Delete every yt-dlp scratch directory that no live download owns,
    /// and report how many bytes that freed. Backs Settings → General →
    /// "Clear temporary data".
    ///
    /// Directories belonging to rows that still exist are **kept**: they
    /// hold the `.part` a paused or queued download resumes from via
    /// `--continue`, and discarding them silently would restart a
    /// multi-gigabyte transfer from zero. Only genuinely orphaned scratch
    /// is removed, so the button is always safe to press.
    pub async fn clear_temporary_data(&self) -> Result<TemporaryDataCleanup> {
        Ok(sweep_scratch_dirs(&self.inner.pool).await)
    }

    /// Bytes currently sitting in orphaned scratch directories, so the
    /// settings row can show what pressing the button would reclaim.
    pub async fn temporary_data_size(&self) -> Result<u64> {
        let live = live_download_ids(&self.inner.pool).await;
        let mut total = 0u64;
        for (_, path) in orphan_scratch_dirs(&live).await {
            total = total.saturating_add(dir_size(&path).await);
        }
        Ok(total)
    }
}

/// Result of [`Core::clear_temporary_data`].
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts-rs-export", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ts-rs-export",
    ts(export, export_to = "TemporaryDataCleanup.ts")
)]
#[serde(rename_all = "camelCase")]
pub struct TemporaryDataCleanup {
    /// Scratch directories removed.
    pub removed_dirs: u32,
    /// Bytes reclaimed, measured before deleting.
    pub freed_bytes: u64,
}

/// Delete every scratch directory under [`ytdlp::scratch_root`] whose id
/// is absent from `pool`.
///
/// Deliberately **not** wired into startup. `%TEMP%\Unduhin` is shared by
/// every `Core` on the machine, and "orphaned" is judged against whichever
/// database happens to be open — so an automatic sweep from a `Core` on a
/// throwaway database (every integration test opens one) would delete the
/// running app's live scratch, silently discarding a multi-gigabyte
/// partial download. Routine cleanup is exact instead: the worker removes
/// its own directory on success, and `download::remove` removes the
/// row's. This bulk pass is reserved for the explicit
/// Settings → *Clear temporary data* action, where the caller is
/// unambiguously the user's real app.
async fn sweep_scratch_dirs(pool: &SqlitePool) -> TemporaryDataCleanup {
    let live = live_download_ids(pool).await;
    let mut cleanup = TemporaryDataCleanup::default();
    for (id, path) in orphan_scratch_dirs(&live).await {
        let size = dir_size(&path).await;
        match tokio::fs::remove_dir_all(&path).await {
            Ok(()) => {
                cleanup.removed_dirs += 1;
                cleanup.freed_bytes = cleanup.freed_bytes.saturating_add(size);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::debug!(id, error = %e, "could not remove scratch dir"),
        }
    }
    cleanup
}

/// Ids of every row still in the database. A scratch directory named for
/// an id outside this set has no owner left.
///
/// On a DB read error this returns `None`, and the callers treat that as
/// "can't prove anything is orphaned" and skip the sweep entirely —
/// deleting a live download's `.part` because a query hiccuped would be a
/// far worse outcome than leaving temp files around.
async fn live_download_ids(pool: &SqlitePool) -> Option<std::collections::HashSet<DownloadId>> {
    match sqlx::query_scalar::<_, DownloadId>("SELECT id FROM downloads")
        .fetch_all(pool)
        .await
    {
        Ok(ids) => Some(ids.into_iter().collect()),
        Err(e) => {
            tracing::warn!(error = %e, "could not list downloads for scratch sweep");
            None
        }
    }
}

/// Enumerate `%TEMP%\Unduhin\*` directories whose name parses as a
/// download id that is no longer in `live`.
async fn orphan_scratch_dirs(
    live: &Option<std::collections::HashSet<DownloadId>>,
) -> Vec<(DownloadId, PathBuf)> {
    let Some(live) = live else { return Vec::new() };
    let root = ytdlp::scratch_root();
    let Ok(mut entries) = tokio::fs::read_dir(&root).await else {
        return Vec::new();
    };
    let mut orphans = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if !entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        // Anything not named for a download id isn't ours to delete.
        let Some(id) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.parse::<DownloadId>().ok())
        else {
            continue;
        };
        if !live.contains(&id) {
            orphans.push((id, path));
        }
    }
    orphans
}

/// Recursive byte total for a directory. Best-effort: unreadable entries
/// contribute zero rather than aborting the walk, since this only feeds a
/// "you'll free about this much" figure.
async fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            match entry.metadata().await {
                Ok(m) if m.is_dir() => stack.push(entry.path()),
                Ok(m) => total = total.saturating_add(m.len()),
                Err(_) => {}
            }
        }
    }
    total
}

/// Conventional location for the user's Unduhin database when callers
/// don't supply their own path. Returns
/// `%LOCALAPPDATA%/unduhin/unduhin.db` on Windows.
pub fn default_db_path() -> Option<PathBuf> {
    directories_root().map(|dir| dir.join("unduhin.db"))
}

/// Environment variable that relocates the entire app-data root.
///
/// Set by `scripts/dev.ps1` so a `cargo tauri dev` build can run beside an
/// installed release without sharing the database, logs, managed binaries,
/// torrent state, or the canonical unpacked-extension folder. Because the
/// value is read here, every derived path moves with it — callers never
/// need their own override.
pub const DATA_ROOT_ENV: &str = "UNDUHIN_DATA_ROOT";

/// Pure half of the [`DATA_ROOT_ENV`] lookup, split out so it is testable
/// without mutating process-global environment state (which would race the
/// other unit tests in this crate).
///
/// An unset *or* blank value falls through to the platform default — the
/// same non-empty guard `resolve_db_path` applies to `UNDUHIN_DB`, so an
/// accidentally-cleared variable degrades to normal behavior instead of
/// rooting the app data at the filesystem root.
fn data_root_override(raw: Option<&str>) -> Option<PathBuf> {
    let trimmed = raw?.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

#[doc(hidden)]
pub fn directories_root() -> Option<PathBuf> {
    if let Some(root) = data_root_override(std::env::var(DATA_ROOT_ENV).ok().as_deref()) {
        return Some(root);
    }
    // We avoid pulling in `directories` here to keep deps tight; mimic the
    // behavior we want for Windows local data.
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            return Some(Path::new(&local).join("unduhin"));
        }
    }
    // macOS keeps per-user app data in `~/Library/Application Support`, not
    // in the XDG-style `~/.local/share` the Unix arm below assumes.
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return Some(
                Path::new(&home)
                    .join("Library")
                    .join("Application Support")
                    .join("unduhin"),
            );
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

/// The user's Downloads folder — where downloads land when neither a
/// category folder nor the global `default_output_path` setting supplies
/// one. Same env-var approach as [`directories_root`] to keep deps tight.
///
/// Deliberately NOT the process CWD: when the app autostarts with Windows
/// the CWD is `C:\WINDOWS\system32`, so a CWD fallback makes every
/// unconfigured download fail with "Access is denied" (os error 5).
pub fn fallback_download_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(profile) = std::env::var("USERPROFILE") {
            return Path::new(&profile).join("Downloads");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return Path::new(&home).join("Downloads");
    }
    // No user profile at all (service-like context): a writable temp dir
    // still beats failing the download outright.
    std::env::temp_dir().join("unduhin")
}

#[doc(hidden)]
pub fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod dirs_tests {
    use super::*;

    #[test]
    fn data_root_override_returns_the_configured_path() {
        assert_eq!(
            data_root_override(Some(r"C:\Users\dev\AppData\Local\unduhin-dev")),
            Some(PathBuf::from(r"C:\Users\dev\AppData\Local\unduhin-dev"))
        );
    }

    #[test]
    fn data_root_override_trims_surrounding_whitespace() {
        assert_eq!(
            data_root_override(Some("  /tmp/unduhin-dev  ")),
            Some(PathBuf::from("/tmp/unduhin-dev"))
        );
    }

    #[test]
    fn data_root_override_falls_through_when_unset() {
        assert_eq!(data_root_override(None), None);
    }

    #[test]
    fn data_root_override_falls_through_when_blank() {
        // An accidentally-cleared variable must degrade to the platform
        // default rather than rooting every app-data path at "".
        assert_eq!(data_root_override(Some("")), None);
        assert_eq!(data_root_override(Some("   ")), None);
    }
}
