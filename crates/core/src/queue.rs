//! Queue manager: pulls queued downloads, runs them via [`engine`], and
//! reconciles state with the database on every tick.
//!
//! ## Design
//!
//! - A single async task runs the manager loop. It wakes on a 500 ms
//!   timer, on an explicit `poke()` from `Core` (e.g. after `add_download`
//!   or `pause`), or on shutdown.
//! - For each tick, the manager (1) reconciles its in-memory active
//!   handles against the DB — if the row is no longer `active`, the
//!   matching cancellation token fires; (2) reaps completed handles;
//!   (3) claims new `queued` rows up to `max_concurrent_downloads` and
//!   spawns a worker per claim.
//! - Each worker calls [`engine::download`] (fresh) or
//!   [`engine::resume_at`] (sidecar exists) and forwards
//!   [`engine::ProgressEvent`]s onto the core event bus while persisting
//!   progress + sidecar JSON snapshots into the `downloads` row.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use engine::{Backoff, CancellationToken, Control, DownloadOptions, Meta, ProgressEvent};
use sqlx::SqlitePool;
use tokio::sync::{broadcast, mpsc, Mutex, OnceCell, RwLock};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use url::Url;

use crate::download::{
    self, DownloadId, DownloadKind, DownloadRecord, ErrorKind, Status, SwarmStats, TorrentSource,
};
use crate::event::CoreEvent;
use crate::schedule::{self, SchedulesCache};
use crate::settings;
use crate::tooling::{self, Tool};
use crate::ytdlp::{self, YtdlpJob};

/// Lazily-built process-wide torrent session, behind the `crates/torrent`
/// facade. Stored on [`crate::CoreInner`] and shared with the queue manager;
/// the worker resolves the [`torrent::TorrentEngine`] from it on the first
/// `DownloadKind::Torrent` claim (design §3.D — one DHT, one listen socket,
/// one peer budget for the whole process). Construction is config-from-settings
/// and runs at most once thanks to [`OnceCell`].
pub(crate) struct TorrentEngineCell {
    cell: OnceCell<Arc<torrent::TorrentEngine>>,
}

impl TorrentEngineCell {
    pub(crate) fn new() -> Self {
        Self {
            cell: OnceCell::new(),
        }
    }

    /// Resolve the shared engine, building it on first use from the
    /// `torrent_*` settings (design §3.G) + the managed state dir. The build
    /// runs exactly once even under concurrent first claims; subsequent calls
    /// return the cached `Arc`.
    pub(crate) async fn get_or_init(
        &self,
        pool: &SqlitePool,
    ) -> Result<Arc<torrent::TorrentEngine>, engine::EngineError> {
        self.cell
            .get_or_try_init(|| async {
                let cfg = build_torrent_config(pool).await?;
                let engine = torrent::TorrentEngine::new(cfg)
                    .await
                    .map_err(engine::EngineError::from)?;
                Ok::<_, engine::EngineError>(Arc::new(engine))
            })
            .await
            .cloned()
    }

    /// The shared engine if it has ALREADY been built, without initializing it.
    /// `Core::remove` uses this to forget a torrent from a live session only
    /// when one exists — no point spinning a session up just to remove
    /// something that was therefore never in it.
    pub(crate) fn get(&self) -> Option<Arc<torrent::TorrentEngine>> {
        self.cell.get().cloned()
    }
}

/// Build the facade's [`torrent::TorrentConfig`] from the `torrent_*` settings
/// seeded by the P1 migration (design §3.G). The session-wide content default
/// is `torrent_download_dir` (or the global default); per-download content dirs
/// override it at `run` time. Session JSON + fastresume live in the managed
/// `<app_data>/torrents` dir (design §3.D), distinct from content so resume
/// state survives content moves.
async fn build_torrent_config(
    pool: &SqlitePool,
) -> Result<torrent::TorrentConfig, engine::EngineError> {
    // Session-default content dir: `torrent_download_dir`, else the global
    // default, else the user's Downloads folder (never the CWD — that is
    // `C:\WINDOWS\system32` when the app autostarts with Windows). This is
    // only the session default librqbit records; each `run` passes an
    // explicit per-download content dir that overrides it.
    let configured = settings::get(pool, "torrent_download_dir")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());
    let download_dir = match configured {
        Some(d) => PathBuf::from(d),
        None => settings::get(pool, settings::settings_keys::DEFAULT_OUTPUT_PATH)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(crate::fallback_download_dir),
    };

    let state_dir = crate::directories_root()
        .map(|d| d.join("torrents"))
        .unwrap_or_else(|| download_dir.join(".unduhin-torrents"));

    let listen_port = settings::get(pool, "torrent_listen_port")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        .min(u16::MAX as u64) as u16;
    let enable_dht = settings::get(pool, "torrent_enable_dht")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    // Off unless the user turned it on: UPnP reconfigures their router.
    let enable_upnp = settings::get(pool, "torrent_enable_upnp")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // Seed-until ratio in thousandths; `0` (the seeded default) = forget at
    // 100 %, no seeding. Clamp into u32 — the UI bounds it to 0..=100_000.
    let seed_ratio_milli = settings::get(pool, "torrent_seed_ratio_milli")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        .min(u32::MAX as u64) as u32;
    // Global (cross-backend) download speed cap, in bytes/sec. Shared with the
    // HTTP engine and yt-dlp; `0` = unlimited. Live changes are pushed to the
    // running session by `Core::set_setting`.
    let download_limit_bps = settings::get(pool, settings::settings_keys::GLOBAL_SPEED_LIMIT_BPS)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let mut cfg = torrent::TorrentConfig::new(download_dir, state_dir);
    cfg.listen_port = listen_port;
    cfg.enable_dht = enable_dht;
    cfg.enable_upnp = enable_upnp;
    cfg.seed_ratio_milli = seed_ratio_milli;
    cfg.download_limit_bps = download_limit_bps;
    Ok(cfg)
}

/// Period of the manager's reconciliation loop when nothing pokes it.
const TICK_PERIOD: Duration = Duration::from_millis(500);

/// How far yt-dlp's byte counter may fall between two ticks before the
/// pump reads it as "moved on to the next stream" rather than jitter.
const STREAM_BOUNDARY_SLACK: u64 = 1024;

/// How many consecutive reconcile ticks a worker may stay in the active
/// map after being cancelled before the manager complains.
///
/// A cancelled worker is *supposed* to exit within a tick or two; a
/// yt-dlp cancel now has a bounded drain, so lingering past this means
/// something is genuinely stuck. Logging it turns what used to be a
/// silent zombie (still downloading, no longer reachable) into a
/// diagnosable one.
const CANCEL_LINGER_TICKS: u32 = 20;

/// How long the active worker set must stay empty (after having been
/// non-empty) before `CoreEvent::QueueEmptied` fires. A brief gap between
/// two downloads — `fill_capacity` clearing one row and claiming the next
/// — must not look like a drain. One second is comfortably above the
/// 500 ms tick period plus the worst-case fill latency.
const QUEUE_EMPTY_DEBOUNCE: Duration = Duration::from_secs(1);

/// Channel handle returned to `Core` once the manager is running.
pub(crate) struct QueueHandle {
    wake_tx: mpsc::Sender<()>,
    shutdown: CancellationToken,
    join: Mutex<Option<JoinHandle<()>>>,
    active: Arc<Mutex<HashMap<DownloadId, ActiveHandle>>>,
}

impl QueueHandle {
    /// Hint the manager to re-evaluate state immediately rather than
    /// waiting for the next tick. Cheap and coalesced — multiple pokes
    /// in flight produce at most one extra tick.
    pub(crate) fn poke(&self) {
        let _ = self.wake_tx.try_send(());
    }

    /// Cancel all active transfers (their sidecars are preserved by the
    /// engine on cancel) and wait for the manager loop to exit.
    pub(crate) async fn shutdown(&self) {
        self.shutdown.cancel();
        if let Some(handle) = self.join.lock().await.take() {
            let _ = handle.await;
        }
    }

    /// Look up the live control sender for a download. Returns `None`
    /// when the download is not currently active or is a yt-dlp job
    /// (which does not honor live re-segmentation).
    pub(crate) async fn control_for(&self, id: DownloadId) -> Option<mpsc::Sender<Control>> {
        let active = self.active.lock().await;
        active.get(&id).and_then(|h| h.control.clone())
    }

    /// Cancel the worker for `id` (if any) and synchronously wait for
    /// its task to exit. No-op when no worker is in flight.
    ///
    /// Used by [`crate::Core::remove`] so the engine / yt-dlp child has
    /// fully released its file handles before the caller deletes the
    /// file on disk. Without this, the worker can re-open the path on
    /// a mid-flight retry (engine `try_segment` uses `create(true)`)
    /// or yt-dlp can finalize its `.part` rename right after our
    /// delete, leaving a 0-byte ghost behind.
    pub(crate) async fn cancel_and_wait(&self, id: DownloadId) {
        let handle = {
            let mut active = self.active.lock().await;
            active.remove(&id)
        };
        let Some(h) = handle else { return };
        h.cancel.cancel();
        let _ = h.join.await;
    }
}

pub(crate) struct QueueManager {
    pool: SqlitePool,
    events: broadcast::Sender<CoreEvent>,
    active: Arc<Mutex<HashMap<DownloadId, ActiveHandle>>>,
    shutdown: CancellationToken,
    drain: Mutex<DrainTracker>,
    /// Shared with [`crate::Core`]; the manager reads it on every fill
    /// pass (start_at gating + after_queue deferral) and writes back when
    /// it reaps a fired `start_at` row.
    schedules: Arc<RwLock<SchedulesCache>>,
    /// Shared lazy torrent session (design §3.D). Threaded into each torrent
    /// worker so they all reuse one process-wide librqbit session.
    torrent_engine: Arc<TorrentEngineCell>,
    /// Process-wide global speed limiter, handed to every HTTP worker so the
    /// `global_speed_limit_bps` cap applies across all downloads at once.
    rate_limiter: Arc<engine::TokenBucket>,
}

/// Debounce state for the `QueueEmptied` emission. Owned by the manager
/// (a tokio `Mutex` so `tick()` can mutate it via `&self`). A fresh
/// non-empty observation arms the tracker; a sustained empty observation
/// fires the emit exactly once per drain.
#[derive(Default)]
struct DrainTracker {
    /// `true` once we've observed at least one active worker since the
    /// last emission. Without this, an empty queue at startup would emit
    /// `QueueEmptied` on tick #1, which would be a lie.
    saw_non_empty: bool,
    /// First instant we observed `active.is_empty()` while `saw_non_empty`
    /// was true. The emit fires once `now - empty_since >= debounce`.
    empty_since: Option<Instant>,
}

struct ActiveHandle {
    cancel: CancellationToken,
    join: JoinHandle<()>,
    /// Per-download control channel — present only for engine-driven
    /// downloads (yt-dlp paths leave this `None` since they don't honor
    /// live re-segmentation).
    control: Option<mpsc::Sender<Control>>,
    /// Reconcile ticks observed since this worker was first told to stop.
    /// `0` means "not cancelled". Purely diagnostic — see
    /// [`CANCEL_LINGER_TICKS`].
    cancel_ticks: u32,
}

impl QueueManager {
    pub(crate) async fn spawn(
        pool: SqlitePool,
        events: broadcast::Sender<CoreEvent>,
        schedules: Arc<RwLock<SchedulesCache>>,
        torrent_engine: Arc<TorrentEngineCell>,
        rate_limiter: Arc<engine::TokenBucket>,
    ) -> QueueHandle {
        let shutdown = CancellationToken::new();
        let (wake_tx, wake_rx) = mpsc::channel(1);
        let active = Arc::new(Mutex::new(HashMap::new()));
        let manager = Arc::new(Self {
            pool,
            events,
            active: active.clone(),
            shutdown: shutdown.clone(),
            drain: Mutex::new(DrainTracker::default()),
            schedules,
            torrent_engine,
            rate_limiter,
        });
        let handle_manager = manager.clone();
        let join = tokio::spawn(async move {
            handle_manager.run(wake_rx).await;
        });
        QueueHandle {
            wake_tx,
            shutdown,
            join: Mutex::new(Some(join)),
            active,
        }
    }

    async fn run(self: Arc<Self>, mut wake: mpsc::Receiver<()>) {
        loop {
            self.tick().await;
            tokio::select! {
                _ = self.shutdown.cancelled() => break,
                _ = sleep(TICK_PERIOD) => {}
                _ = wake.recv() => {}
            }
        }
        self.shutdown_all().await;
    }

    /// One pass of the reconciliation loop.
    async fn tick(&self) {
        if let Err(err) = self.reconcile_active().await {
            tracing::warn!(%err, "queue: reconcile pass failed");
        }
        if let Err(err) = self.reap_completed().await {
            tracing::warn!(%err, "queue: reap pass failed");
        }
        if let Err(err) = self.fill_capacity().await {
            tracing::warn!(%err, "queue: fill pass failed");
        }
        // Drain detection runs *after* the fill pass so a brand-new claim
        // is already reflected in `active` and we don't fire QueueEmptied
        // in the gap between a reap and a fill.
        self.update_drain_state(Instant::now()).await;
    }

    /// Update the drain debounce tracker and emit `QueueEmptied` once a
    /// sustained empty observation crosses `QUEUE_EMPTY_DEBOUNCE`. Pure
    /// time-driven logic split out for testability — `now` is passed in
    /// so unit tests can drive the clock without sleeping.
    async fn update_drain_state(&self, now: Instant) {
        let active_len = self.active.lock().await.len();
        let mut drain = self.drain.lock().await;
        if active_len > 0 {
            drain.saw_non_empty = true;
            drain.empty_since = None;
            return;
        }
        if !drain.saw_non_empty {
            return;
        }
        match drain.empty_since {
            None => {
                drain.empty_since = Some(now);
            }
            Some(t0) if now.duration_since(t0) >= QUEUE_EMPTY_DEBOUNCE => {
                // Fire and disarm; require a fresh non-empty observation
                // before the next emission.
                let _ = self.events.send(CoreEvent::QueueEmptied);
                drain.saw_non_empty = false;
                drain.empty_since = None;
            }
            Some(_) => {
                // Still within the debounce window; wait for the next tick.
            }
        }
    }

    /// Cancel any active worker whose DB row has moved out of `active`.
    ///
    /// The handle deliberately stays in the map until the worker actually
    /// exits and [`Self::reap_completed`] removes it. That is what keeps a
    /// stopping worker *reachable*: `fill_capacity` skips ids that are
    /// still present, so a quick pause→resume can't spawn a second yt-dlp
    /// against the same output path while the first is still winding down.
    async fn reconcile_active(&self) -> crate::error::Result<()> {
        let db_active = download::active_ids(&self.pool).await?;
        let mut active = self.active.lock().await;
        let to_cancel: Vec<DownloadId> = active
            .keys()
            .copied()
            .filter(|id| !db_active.contains(id))
            .collect();
        for id in to_cancel {
            let Some(h) = active.get_mut(&id) else {
                continue;
            };
            if h.cancel_ticks == 0 {
                tracing::debug!(id, "queue: cancelling worker (db status changed)");
                h.cancel.cancel();
            }
            h.cancel_ticks = h.cancel_ticks.saturating_add(1);
            if h.cancel_ticks == CANCEL_LINGER_TICKS {
                tracing::warn!(
                    id,
                    ticks = h.cancel_ticks,
                    "queue: worker still running long after cancel; the row stays blocked \
                     from being re-claimed until it exits"
                );
            }
        }
        Ok(())
    }

    /// Drop join handles for workers that have finished.
    async fn reap_completed(&self) -> crate::error::Result<()> {
        let mut active = self.active.lock().await;
        let ids: Vec<DownloadId> = active
            .iter()
            .filter(|(_, h)| h.join.is_finished())
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            if let Some(h) = active.remove(&id) {
                // Already finished — await is cheap and ensures any panic
                // surfaces in the logs.
                if let Err(e) = h.join.await {
                    tracing::warn!(id, error = %e, "queue: worker join error");
                }
            }
        }
        Ok(())
    }

    /// Claim queued rows up to the concurrency limit and spawn workers.
    /// Claims are gated through the schedules cache: rows with
    /// future `start_at` stay queued; rows tagged `after_queue` only
    /// claim when nothing else is in flight.
    async fn fill_capacity(&self) -> crate::error::Result<()> {
        let limit = max_concurrent(&self.pool).await? as usize;
        let queued = download::list_queued(&self.pool).await?;
        let now = Utc::now();
        let mut active = self.active.lock().await;
        // Reaped after the claim loop so we touch the cache + emit once
        // per fill pass instead of once per fired start_at.
        let mut start_at_reaped: Vec<DownloadId> = Vec::new();
        for record in queued {
            if active.len() >= limit {
                break;
            }
            let id = record.id;
            // A worker already owns this id. Leave the row `queued` and
            // pick it up on a later tick, once the old worker has exited
            // and `reap_completed` has dropped its handle.
            //
            // Without this guard, `active.insert` below silently replaced
            // the previous `ActiveHandle` — which drops its `JoinHandle`
            // (detaching the task) and the last clone of its
            // `CancellationToken` (making it permanently uncancellable).
            // The old yt-dlp kept downloading to the same output path
            // forever, invisible to pause, delete, and shutdown alike.
            // A pause immediately followed by a resume was enough to hit
            // it, because `reconcile_active` cancels without waiting.
            if active.contains_key(&id) {
                tracing::debug!(id, "queue: skipping claim; a worker is still winding down");
                continue;
            }
            let (runnable, was_start_at_gated) = {
                let cache = self.schedules.read().await;
                let runnable = cache.is_runnable(id, now, active.is_empty());
                // A row "was gated by start_at" if a row exists for it —
                // by the time we're here, runnable==true means the time
                // has come (or no such row exists). We only want to reap
                // when the row actually had a start_at gate.
                let had_start_at = cache.all().iter().any(|r| {
                    r.kind == schedule::ScheduleKind::StartAt
                        && r.active
                        && r.download_id == Some(id)
                });
                (runnable, had_start_at)
            };
            if !runnable {
                continue;
            }
            if !download::claim(&self.pool, id).await? {
                // Someone else changed status before us — try the next row.
                continue;
            }
            if was_start_at_gated {
                // Mark in-memory so subsequent ticks in this pass don't
                // re-gate the row before the DB delete + reload below.
                self.schedules.write().await.mark_start_at_consumed(id);
                start_at_reaped.push(id);
            }
            let _ = self.events.send(CoreEvent::StatusChanged {
                id,
                from: Status::Queued,
                to: Status::Active,
            });
            let cancel = CancellationToken::new();
            // Only engine-driven (HTTP) downloads use the control channel
            // for live re-segmentation. yt-dlp and torrent paths ignore it.
            let (control_tx, control_rx) = if record.kind == DownloadKind::Http {
                let (tx, rx) = mpsc::channel::<Control>(8);
                (Some(tx), Some(rx))
            } else {
                (None, None)
            };
            let join = spawn_worker(
                self.pool.clone(),
                self.events.clone(),
                record,
                cancel.clone(),
                control_rx,
                self.torrent_engine.clone(),
                self.rate_limiter.clone(),
            );
            // Guarded by the `contains_key` check at the top of the loop —
            // an occupied slot here would mean a second worker for one
            // download, so shout rather than clobber.
            if let Some(previous) = active.insert(
                id,
                ActiveHandle {
                    cancel,
                    join,
                    control: control_tx,
                    cancel_ticks: 0,
                },
            ) {
                tracing::error!(
                    id,
                    "queue: replaced a live worker handle; the previous worker is now \
                     detached and uncancellable"
                );
                previous.cancel.cancel();
            }
        }
        drop(active);

        if !start_at_reaped.is_empty() {
            let mut touched = false;
            for id in start_at_reaped {
                if let Err(e) = schedule::delete_start_at_for(&self.pool, id).await {
                    tracing::warn!(id, error = %e, "queue: failed to reap fired start_at row");
                } else {
                    touched = true;
                }
            }
            if touched {
                if let Err(e) = self.schedules.write().await.reload(&self.pool).await {
                    tracing::warn!(error = %e, "queue: schedules cache reload failed after reap");
                }
                let _ = self.events.send(CoreEvent::SchedulesChanged);
            }
        }
        Ok(())
    }

    /// Cancel everything in flight (engine flushes sidecars on cancel).
    /// Called once during shutdown.
    async fn shutdown_all(&self) {
        let handles: Vec<(DownloadId, ActiveHandle)> = {
            let mut active = self.active.lock().await;
            active.drain().collect()
        };
        for (id, h) in handles {
            tracing::debug!(id, "queue: shutdown — cancelling worker");
            h.cancel.cancel();
            let _ = h.join.await;
        }
    }
}

async fn max_concurrent(pool: &SqlitePool) -> crate::error::Result<u64> {
    Ok(
        settings::get(pool, settings::settings_keys::MAX_CONCURRENT_DOWNLOADS)
            .await?
            .and_then(|v| v.as_u64())
            .unwrap_or(3),
    )
}

/// Spawn the per-download worker task.
fn spawn_worker(
    pool: SqlitePool,
    events: broadcast::Sender<CoreEvent>,
    record: DownloadRecord,
    cancel: CancellationToken,
    control_rx: Option<mpsc::Receiver<Control>>,
    torrent_engine: Arc<TorrentEngineCell>,
    rate_limiter: Arc<engine::TokenBucket>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        run_worker(
            pool,
            events,
            record,
            cancel,
            control_rx,
            torrent_engine,
            rate_limiter,
        )
        .await;
    })
}

async fn run_worker(
    pool: SqlitePool,
    events: broadcast::Sender<CoreEvent>,
    record: DownloadRecord,
    cancel: CancellationToken,
    control_rx: Option<mpsc::Receiver<Control>>,
    torrent_engine: Arc<TorrentEngineCell>,
    rate_limiter: Arc<engine::TokenBucket>,
) {
    let id = record.id;
    let output_path = record.output_path.clone();
    let meta_path = Meta::sidecar_path(&output_path);
    let pump_meta_path = meta_path.clone();
    // The URL parse is HTTP/media-only: those backends key everything off
    // `record.url`. Torrent rows read `record.torrent` instead and may
    // carry a bare info-hash that is not a valid `Url`, so the parse is
    // pushed into the Http/Media arms below (Q3) — never gating torrents.
    let url: Option<Url> = if record.kind == DownloadKind::Torrent {
        None
    } else {
        match record.url.parse::<Url>() {
            Ok(u) => Some(u),
            Err(e) => {
                mark_worker_failed(
                    &pool,
                    &events,
                    id,
                    &format!("invalid url: {e}"),
                    ErrorKind::Other,
                )
                .await;
                return;
            }
        }
    };

    let (connect_timeout, read_timeout) = timeouts(&pool).await;
    let user_agent = user_agent_setting(&pool).await;
    let segments = record.segments as usize;

    let (tx, mut rx) =
        tokio::sync::broadcast::channel::<ProgressEvent>(engine::DEFAULT_CHANNEL_CAPACITY);

    // Forward engine events onto the core event bus while persisting
    // progress to the DB. Lives in a separate task because the engine
    // owns the producing side and we want to react in real time.
    let pump_pool = pool.clone();
    let pump_events = events.clone();
    // The HTTP engine emits `FilenameLearned` and reconciles against
    // `pump_url` (`None` for torrents). Torrents ALSO emit `FilenameLearned`
    // (the librqbit facade fires it once magnet metadata resolves the real
    // name) but reconcile via a directory-safe display-only path — gated on
    // this flag so the two never cross-wire.
    let pump_url = url.clone();
    let pump_is_torrent = record.kind == DownloadKind::Torrent;
    // Only yt-dlp reports per-stream byte counters that restart at 0.
    let pump_is_media = record.kind == DownloadKind::Media;
    let pump = tokio::spawn(async move {
        // yt-dlp downloads multi-stream formats (video + audio) one
        // stream at a time, and each stream's progress restarts from
        // byte 0. `stream_base` accumulates the completed streams so the
        // bar keeps climbing instead of snapping back.
        //
        // This used to double as the `Muxing` trigger: a byte-counter
        // regression was read as "yt-dlp moved on to the merge". That
        // inference is wrong for anything fragmented — a retried HLS
        // fragment or a resumed run regresses the counter mid-download —
        // and it stranded still-downloading rows on "Merging audio +
        // video…", a state with no pause button. The two concerns are now
        // separate: this offset keeps the bar honest, and `Muxing` comes
        // from yt-dlp's own `postprocess:` hook via `PostProcessing`.
        let mut last_downloaded: u64 = 0;
        let mut stream_base: u64 = 0;
        let mut muxing_emitted = false;
        // Raw per-tick speed samples, downsampled and persisted to the
        // `speed_samples` column once the stream ends so the detail-pane
        // sparkline survives a relaunch (Bug: empty sparkline after finish).
        let mut speed_samples: Vec<u32> = Vec::new();
        loop {
            match rx.recv().await {
                Ok(ProgressEvent::Started { total, .. }) => {
                    if let Some(t) = total {
                        let _ = sqlx::query(
                            "UPDATE downloads SET total_bytes = COALESCE(total_bytes, ?) \
                             WHERE id = ?",
                        )
                        .bind(t as i64)
                        .bind(id)
                        .execute(&pump_pool)
                        .await;
                    }
                }
                Ok(ProgressEvent::Tick {
                    downloaded,
                    total,
                    speed_bps,
                    eta,
                }) => {
                    // A meaningful drop in yt-dlp's byte counter means it
                    // finished one stream and started the next; carry the
                    // completed bytes forward so the bar doesn't snap back
                    // to 0 %. Media-only: librqbit's torrent counts aren't
                    // monotonic either (piece verification, the metadata →
                    // content transition) but they're already absolute, so
                    // offsetting them would inflate the row.
                    if pump_is_media && downloaded + STREAM_BOUNDARY_SLACK < last_downloaded {
                        stream_base = stream_base.saturating_add(last_downloaded);
                    }
                    last_downloaded = downloaded;
                    let downloaded = downloaded.saturating_add(stream_base);
                    let total = total.map(|t| t.saturating_add(stream_base));

                    // Re-read the sidecar lazily — engine writes it on
                    // every tick, so a stale read here just means slightly
                    // older segment positions in the DB.
                    let segments_meta = read_sidecar_segments(&pump_meta_path).await;
                    if let Err(e) = download::persist_progress(
                        &pump_pool,
                        id,
                        downloaded,
                        total,
                        None,
                        None,
                        segments_meta.as_deref(),
                    )
                    .await
                    {
                        tracing::warn!(id, error = %e, "queue: persist_progress failed");
                    }
                    let _ = pump_events.send(CoreEvent::ProgressUpdate {
                        id,
                        downloaded,
                        total,
                        speed_bps,
                        eta,
                    });
                    speed_samples.push(speed_bps.max(0.0) as u32);
                }
                Ok(ProgressEvent::PostProcessing) => {
                    // yt-dlp reached the ffmpeg merge / remux / fixup step:
                    // the bytes are on disk and the progress bar has
                    // nothing left to report, so the row shows a distinct
                    // "Merging audio + video…" state.
                    //
                    // Only yt-dlp emits this (design §3.C: Muxing is
                    // yt-dlp-only), and only once per run — yt-dlp ticks
                    // this hook for every post-processor it runs.
                    if !muxing_emitted
                        && download::transition_status(
                            &pump_pool,
                            id,
                            &[Status::Active],
                            Status::Muxing,
                        )
                        .await
                        .is_ok()
                    {
                        let _ = pump_events.send(CoreEvent::StatusChanged {
                            id,
                            from: Status::Active,
                            to: Status::Muxing,
                        });
                        muxing_emitted = true;
                    }
                }
                Ok(ProgressEvent::Completed { bytes }) => {
                    tracing::info!(id, bytes, "queue: pump received Completed");
                    // Also persist so the DB matches the in-memory state —
                    // mark_completed below uses COALESCE on total_bytes,
                    // which would otherwise keep a stale second-stream
                    // total and leave downloaded < total in the row.
                    if let Err(e) = sqlx::query(
                        "UPDATE downloads SET downloaded_bytes = ?, total_bytes = ? \
                         WHERE id = ?",
                    )
                    .bind(bytes as i64)
                    .bind(bytes as i64)
                    .bind(id)
                    .execute(&pump_pool)
                    .await
                    {
                        tracing::warn!(id, error = %e, "queue: persist on Completed failed");
                    }
                    let _ = pump_events.send(CoreEvent::ProgressUpdate {
                        id,
                        downloaded: bytes,
                        total: Some(bytes),
                        speed_bps: 0.0,
                        eta: None,
                    });
                }
                Ok(ProgressEvent::SegmentProgress {
                    index,
                    bytes_downloaded,
                    segment_total,
                    speed_bps,
                    state,
                }) => {
                    let _ = pump_events.send(CoreEvent::SegmentProgress {
                        id,
                        index,
                        bytes: bytes_downloaded,
                        total: segment_total,
                        speed_bps,
                        state,
                    });
                }
                Ok(ProgressEvent::FilenameLearned { hint }) => {
                    // Torrents: the facade learned the real torrent name once
                    // librqbit resolved magnet metadata. Reconcile the row's
                    // DISPLAY name + category WITHOUT moving anything on disk
                    // (the content root is a live directory librqbit is writing
                    // into). Mirrors `finalize_ytdlp_completion`'s category
                    // logic, display-only.
                    if pump_is_torrent {
                        match download::reconcile_torrent_filename(&pump_pool, id, &hint).await {
                            Ok(Some((filename, category_id, category_changed))) => {
                                // The on-disk content root is unchanged; report
                                // the current `output_path` so the UI keeps a
                                // valid path while updating the display name.
                                if let Ok(record) = download::get(&pump_pool, id).await {
                                    let _ = pump_events.send(CoreEvent::PathsChanged {
                                        id,
                                        filename,
                                        output_path: record
                                            .output_path
                                            .to_string_lossy()
                                            .into_owned(),
                                    });
                                }
                                if category_changed {
                                    let _ = pump_events
                                        .send(CoreEvent::CategoryChanged { id, category_id });
                                }
                            }
                            Ok(None) => {}
                            Err(e) => {
                                tracing::warn!(id, error = %e, "queue: reconcile_torrent_filename failed");
                            }
                        }
                        continue;
                    }
                    // The engine learned the real filename from the response
                    // headers while the bytes are still flowing. Update the
                    // row's display name + category now so the UI stops showing
                    // the random URL slug — the on-disk file is relocated to
                    // match at completion (`apply_engine_filename`). Only the
                    // HTTP engine has a meaningful `url` to reconcile against.
                    let Some(pump_url) = pump_url.as_ref() else {
                        continue;
                    };
                    match download::mark_learned_filename(&pump_pool, id, pump_url, &hint).await {
                        Ok(Some(learned)) => {
                            let _ = pump_events.send(CoreEvent::PathsChanged {
                                id,
                                filename: learned.filename,
                                output_path: learned.output_path.to_string_lossy().into_owned(),
                            });
                            if learned.category_changed {
                                let _ = pump_events.send(CoreEvent::CategoryChanged {
                                    id,
                                    category_id: learned.category_id,
                                });
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            tracing::warn!(id, error = %e, "queue: mark_learned_filename failed");
                        }
                    }
                }
                Ok(ev @ ProgressEvent::SwarmProgress { .. }) => {
                    // Torrent-only (design §3.C). Mirror the SegmentProgress
                    // translate-and-re-emit pattern, but also persist the
                    // snapshot into the row's `torrent` JSON so peers/seeds
                    // survive a relaunch. `persist_swarm` is a no-op on a row
                    // with no torrent blob, so a stray HTTP/media emission
                    // (there is none today) can't corrupt anything.
                    if let Some((swarm, core_ev)) = translate_swarm(id, &ev) {
                        if let Err(e) = download::persist_swarm(&pump_pool, id, &swarm).await {
                            tracing::warn!(id, error = %e, "queue: persist_swarm failed");
                        }
                        let _ = pump_events.send(core_ev);
                    }
                }
                Ok(ev @ ProgressEvent::FileProgress { .. }) => {
                    // Torrent-only per-file heartbeat. Re-emit only — the
                    // detail pane holds it in memory like SegmentProgress; we
                    // do not persist per-file byte counts (design §3.C).
                    if let Some(core_ev) = translate_file_progress(id, &ev) {
                        let _ = pump_events.send(core_ev);
                    }
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }

        // Persist the captured speed series (downsampled, trailing zeros
        // trimmed) so the sparkline can be rebuilt after a relaunch.
        let series = downsample_speeds(&speed_samples, SPEED_SAMPLE_CAP);
        if !series.is_empty() {
            if let Ok(json) = serde_json::to_string(&series) {
                let _ = sqlx::query("UPDATE downloads SET speed_samples = ? WHERE id = ?")
                    .bind(json)
                    .bind(id)
                    .execute(&pump_pool)
                    .await;
            }
        }
    });

    let backoff = Backoff::default();
    // Branch on the explicit backend discriminator. `Media` delegates to
    // yt-dlp, `Torrent` to the librqbit facade, and `Http` to the engine
    // (resume when a sidecar exists, fresh otherwise). The URL parse above
    // guarantees `url` is `Some` for the Http/Media arms (Q3).
    let result: Result<engine::DownloadSummary, engine::EngineError> = match record.kind {
        DownloadKind::Media => {
            // yt-dlp-driven downloads bypass the engine entirely. The pump
            // still receives `ProgressEvent`s — yt-dlp produces them via
            // `ytdlp::download` parsing `--progress-template` lines.
            match record.media_info.clone() {
                Some(info) => {
                    run_ytdlp(
                        &pool,
                        &events,
                        info,
                        &record,
                        user_agent.clone(),
                        cancel.clone(),
                        tx.clone(),
                        // Current global cap (live value from the shared bucket).
                        rate_limiter.rate().await,
                    )
                    .await
                }
                None => Err(engine::EngineError::other(
                    "media download is missing its media_info",
                )),
            }
        }
        DownloadKind::Torrent => {
            run_torrent(&pool, &torrent_engine, &record, cancel.clone(), tx.clone()).await
        }
        DownloadKind::Http if meta_path.exists() => {
            engine::resume_at_with_control(
                meta_path.clone(),
                backoff,
                connect_timeout,
                read_timeout,
                user_agent.clone(),
                record.headers.clone().unwrap_or_default(),
                cancel.clone(),
                Some(tx.clone()),
                control_rx,
                // Global speed cap also applies to resumed transfers.
                Some(rate_limiter.clone()),
            )
            .await
        }
        DownloadKind::Http => {
            let http_url = url
                .clone()
                .expect("http rows always have a parsed url (Q3)");
            let mut opts = DownloadOptions::new(http_url, output_path.clone());
            opts.segments = segments;
            opts.connect_timeout = connect_timeout;
            opts.read_timeout = read_timeout;
            opts.backoff = backoff;
            opts.user_agent = user_agent.clone();
            opts.headers = record.headers.clone().unwrap_or_default();
            // Global speed cap: all HTTP workers share this one bucket.
            opts.rate_limiter = Some(rate_limiter.clone());
            engine::download_with_control(opts, cancel.clone(), Some(tx.clone()), control_rx).await
        }
    };

    drop(tx);
    let _ = pump.await;
    tracing::info!(id, "queue: worker download phase complete, finalizing");

    match result {
        Ok(summary) => {
            // Completion gate: the HTTP engine reports a
            // transfer as "complete" even when the server returned 0 bytes
            // or an HTML landing page — common with one-click file hosts
            // whose captured bare URL needs a browser session/token to
            // resolve. Refuse to call that a success; mark it Failed with a
            // clear message and discard the bogus artefact. Scoped to the
            // HTTP path: yt-dlp rows always produce a real media file, and
            // torrents have no single output file to inspect.
            if record.kind == DownloadKind::Http {
                if let Some(reason) = http_completion_rejection(&summary) {
                    if matches!(
                        current_status(&pool, id).await.ok(),
                        Some(Status::Active | Status::Muxing)
                    ) {
                        tracing::warn!(
                            id,
                            bytes = summary.bytes,
                            %reason,
                            "queue: rejecting download at completion gate"
                        );
                        // Drop the empty/HTML file and its sidecar so a retry
                        // starts clean and the user isn't left with a bogus
                        // 0 B file on disk.
                        let _ = tokio::fs::remove_file(&output_path).await;
                        let _ = tokio::fs::remove_file(&meta_path).await;
                        // The 0-byte and HTML-landing-page cases are exactly
                        // an expired link failing *silently* — the server
                        // answers 200 and hands back a click-through page
                        // instead of the bytes. Same remedy as a 403: the
                        // row needs a fresh URL, not a retry.
                        mark_worker_failed(&pool, &events, id, &reason, ErrorKind::ExpiredAuth)
                            .await;
                    }
                    return;
                }

                // The authoritative download GET may have revealed a real
                // filename (Content-Disposition / final-redirect URL) that
                // the add-time HEAD probe couldn't see — single-use-token
                // one-click hosts name the file only on the GET. Apply it
                // now, renaming the slug-named file and reconciling the
                // category. Conservative: only overrides our own URL-tail
                // fallback, never a user-typed or add-time-probed name.
                if let Some(hint) = summary.filename_hint.as_deref() {
                    let http_url = url
                        .as_ref()
                        .expect("http rows always have a parsed url (Q3)");
                    match download::apply_engine_filename(&pool, id, http_url, hint).await {
                        Ok(Some(renamed)) => {
                            let _ = events.send(CoreEvent::PathsChanged {
                                id,
                                filename: renamed.filename,
                                output_path: renamed.path.to_string_lossy().into_owned(),
                            });
                            if renamed.category_changed {
                                let _ = events.send(CoreEvent::CategoryChanged {
                                    id,
                                    category_id: renamed.category_id,
                                });
                            }
                        }
                        Ok(None) => {}
                        Err(e) => tracing::warn!(
                            id, error = %e,
                            "queue: failed to apply engine filename hint"
                        ),
                    }
                }
            }
            // If a user cancel landed at the very last moment, the DB row
            // may already say "cancelled" — only flip to completed if the
            // row is still in a running state. `Muxing` is included
            // because yt-dlp may have transitioned us there mid-flight
            // when it started the second (audio) stream.
            let from = match current_status(&pool, id).await.ok() {
                Some(s @ (Status::Active | Status::Muxing)) => Some(s),
                other => {
                    tracing::warn!(
                        id,
                        ?other,
                        "queue: completion path saw unexpected current_status; skipping mark_completed"
                    );
                    None
                }
            };
            if let Some(from) = from {
                // Last check before the row goes green: is the file actually
                // in its category's folder? Everything above relocates only
                // as a side effect of learning a better NAME, so a row whose
                // category changed while it downloaded — the user
                // recategorized an active row, or a mid-flight reroute never
                // got its move — finishes with the label naming one folder
                // and the bytes sitting in another. This is the one step
                // that checks the folder on its own.
                //
                // Inside the `from` guard, so a cancel that landed in the
                // last moment does not get its partial file moved. Before
                // `mark_completed`, so the row is never briefly visible as
                // completed at a path it is about to leave. Torrents opt out
                // inside the function (their output path is a directory
                // tree), as does any file the user placed at an explicit
                // path.
                match download::reconcile_category_folder(&pool, id).await {
                    Ok(Some((filename, path))) => {
                        let _ = events.send(CoreEvent::PathsChanged {
                            id,
                            filename,
                            output_path: path.to_string_lossy().into_owned(),
                        });
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!(
                        id, error = %e,
                        "queue: failed to reconcile the download's category folder"
                    ),
                }

                // Tag the files as downloaded from the internet, as a browser
                // would have. Last, so every rename and move above is done
                // and the tag lands on the file's final path.
                if let Ok(record) = download::get(&pool, id).await {
                    crate::motw::mark_download(&record).await;
                }

                tracing::info!(id, ?from, bytes = summary.bytes, "queue: marking completed");
                if let Err(e) = download::mark_completed(&pool, id, summary.bytes).await {
                    tracing::warn!(id, error = %e, "queue: mark_completed failed");
                }
                // Send a final ProgressUpdate so the bar lands at 100 %
                // before the status flip — yt-dlp's progress hook stops
                // firing during the ffmpeg merge phase, so without this
                // the row would jump from `Muxing X %` straight to
                // `Completed X %` with the last mid-stream-2 value.
                let _ = events.send(CoreEvent::ProgressUpdate {
                    id,
                    downloaded: summary.bytes,
                    total: Some(summary.bytes),
                    speed_bps: 0.0,
                    eta: None,
                });
                let _ = events.send(CoreEvent::StatusChanged {
                    id,
                    from,
                    to: Status::Completed,
                });
                let _ = events.send(CoreEvent::Completed {
                    id,
                    bytes: summary.bytes,
                });
            }
        }
        Err(engine::EngineError::Cancelled) => {
            // Whoever cancelled us already wrote the new status; leave it.
        }
        Err(engine::EngineError::RemoteChanged) => {
            // The remote content changed under a resume (detected via the
            // `If-Range` guard in the engine). Continuing would stitch old
            // and new bytes into a corrupt file, so discard the partial
            // download and its sidecar and restart from scratch. Bounded
            // per session so a constantly-changing remote can't loop
            // forever — after the cap we fail with a clear message.
            if matches!(
                current_status(&pool, id).await.ok(),
                Some(Status::Active | Status::Muxing)
            ) {
                let _ = tokio::fs::remove_file(&meta_path).await;
                let _ = tokio::fs::remove_file(&output_path).await;
                let restarts = record_remote_changed_restart(id);
                if restarts <= MAX_REMOTE_CHANGED_RESTARTS {
                    tracing::info!(
                        id,
                        restarts,
                        "queue: remote changed; discarding partial and restarting from scratch"
                    );
                    match download::transition_status(
                        &pool,
                        id,
                        &[Status::Active, Status::Muxing],
                        Status::Queued,
                    )
                    .await
                    {
                        Ok(from) => {
                            let _ = events.send(CoreEvent::StatusChanged {
                                id,
                                from,
                                to: Status::Queued,
                            });
                        }
                        Err(e) => {
                            tracing::warn!(id, error = %e, "queue: failed to re-queue after remote change");
                        }
                    }
                } else {
                    mark_worker_failed(
                        &pool,
                        &events,
                        id,
                        "remote file kept changing; gave up after repeated restarts",
                        ErrorKind::Other,
                    )
                    .await;
                }
            }
        }
        Err(err) => {
            // Same idea: only mark failed if DB still considers us
            // mid-flight (Active or Muxing).
            if matches!(
                current_status(&pool, id).await.ok(),
                Some(Status::Active | Status::Muxing)
            ) {
                mark_worker_failed(
                    &pool,
                    &events,
                    id,
                    &err.to_string(),
                    classify_engine_error(&err),
                )
                .await;
            }
        }
    }
}

/// Drive a torrent download via the librqbit facade. Sibling to
/// [`run_ytdlp`]: reads `record.torrent` (never `record.url`), emits the
/// same [`engine::ProgressEvent`]s through the shared pump, and returns a
/// [`engine::DownloadSummary`] so the worker's completion tail is reused
/// unchanged.
///
/// The librqbit `Session` is process-wide (design §3.D): resolved lazily from
/// the shared [`TorrentEngineCell`] on the first torrent claim and reused for
/// every torrent thereafter. The worker's `CancellationToken` is honored for
/// pause/shutdown (librqbit flushes fastresume on pause), and the broadcast
/// `tx` carries progress into the same pump HTTP/media use.
async fn run_torrent(
    pool: &SqlitePool,
    torrent_engine: &TorrentEngineCell,
    record: &DownloadRecord,
    cancel: CancellationToken,
    tx: tokio::sync::broadcast::Sender<engine::ProgressEvent>,
) -> Result<engine::DownloadSummary, engine::EngineError> {
    // Read the torrent state off the row — never `record.url`. A torrent row
    // with no `torrent` blob is a data bug (insert normalizes `kind`/`torrent`
    // in lock-step); fail clearly rather than panic.
    let meta = record.torrent.as_ref().ok_or_else(|| {
        engine::EngineError::other("torrent download is missing its torrent metadata")
    })?;

    // Build the facade input from the persisted source. `TorrentInput` hides
    // every librqbit type; the facade reads `.torrent` bytes itself.
    let input = match &meta.source {
        TorrentSource::Magnet { uri } => torrent::TorrentInput::Magnet(uri.clone()),
        TorrentSource::File { path } => torrent::TorrentInput::TorrentFile(path.clone()),
        TorrentSource::InfoHash { hash } => torrent::TorrentInput::InfoHash(hash.clone()),
    };

    // The content root is the row's `output_path` (a DIRECTORY for torrents —
    // `resolve_torrent_output_dir`). librqbit writes the selected file(s)
    // directly under it, so it is also the summary's `output` and the key
    // Q4's `remove_dir_all` deletes.
    let content_dir = record.output_path.clone();
    let only_files = meta.selected_files.clone();

    // Resolve the shared process-wide session (built on first use).
    let engine = torrent_engine.get_or_init(pool).await?;

    // `?` maps `TorrentError → engine::EngineError` (the facade's `From` impl):
    // `Cancelled → ::Cancelled` (the worker's tail leaves the status alone),
    // everything else → `::other` (marked Failed). Same trick `run_ytdlp` uses.
    let summary = engine
        .run(input, content_dir, only_files, cancel, Some(tx))
        .await?;

    Ok(engine::DownloadSummary {
        // `record.url` is the magnet/source string, not necessarily a valid
        // `Url`. The summary `url` is only used by the HTTP completion gate
        // (skipped for torrents) and the unused field is otherwise inert, so
        // fall back to a sentinel when the source string isn't a `Url`.
        url: record
            .url
            .parse::<url::Url>()
            .unwrap_or_else(|_| url::Url::parse("about:blank").expect("about:blank parses")),
        output: summary.output_root,
        bytes: summary.bytes,
        // Torrents have no HTTP segments; one logical unit (design §3.C).
        segments: 1,
        resumed: summary.resumed,
        // Torrents have no single output file to inspect; the HTTP-only
        // completion gate and engine-filename hint do not apply.
        content_type: None,
        filename_hint: None,
    })
}

/// Extensions [`ytdlp_output_stem`] strips off a stored file name.
///
/// yt-dlp appends the real extension itself through `%(ext)s`, so a name
/// that already carries one has to lose it or the file lands as
/// `clip.mp4.mp4`. The list is an allowlist on purpose — see
/// [`ytdlp_output_stem`] for why anything looser breaks real titles.
/// Container and audio formats yt-dlp can produce, plus the two manifest
/// extensions a captured stream arrives with.
const STRIPPABLE_EXTENSIONS: &[&str] = &[
    // Video containers.
    "mp4", "m4v", "mkv", "webm", "mov", "avi", "flv", "ts", "m2ts", "mpg", "mpeg", "wmv", "ogv",
    "3gp", // Audio.
    "mp3", "m4a", "aac", "opus", "ogg", "oga", "flac", "wav", "wma", "alac",
    // Streaming manifests — what a captured stream's name is derived from.
    "m3u8", "mpd",
];

/// Turn a download row's stored file name into the stem half of a yt-dlp
/// `--output` template.
///
/// Two things have to happen to it.
///
/// **Drop a real extension, keep a title.** yt-dlp picks the extension
/// once it knows the chosen format, so one already on the name is
/// removed. `Path::file_stem` is the obvious tool and the wrong one: it
/// cuts everything after the LAST dot whatever that is. A title like
/// `"Ep. 5 - The Finale"` became the template `Ep.%(ext)s` and the file
/// landed on disk as `Ep.mp4`. Media titles carry dots all the time —
/// `"Part 2.5"`, `"Show S01.E05"`, `"Vol. 3"` — and every one of them
/// lost its tail.
///
/// A length-and-shape heuristic does not save it either: the tail of
/// `"Part 2.5"` is short and alphanumeric, and the tail of
/// `"Show S01.E05"` is short, alphanumeric, and has a letter. Only an
/// allowlist separates the two cases cleanly, so only an extension in
/// [`STRIPPABLE_EXTENSIONS`] is dropped. Stripping repeats, which is
/// what reduces the `name.mp4.m3u8` an HLS capture produces down to
/// `name`. The loop ends on its own — the string shrinks each pass, and
/// an empty base stops it.
///
/// Being wrong in the safe direction matters here: a kept extension
/// costs a doubled suffix the user can see and rename, while a wrongly
/// cut one silently truncates the file name.
///
/// **Escape percent signs.** yt-dlp reads `%(field)s` in an output
/// template, and `sanitize_filename` does not remove `%` — it only
/// strips the characters Windows rejects in a path. A title holding a
/// literal `%` would be read as the start of a field. `%%` is yt-dlp's
/// escape for one literal percent sign. Escaping the stem before it is
/// formatted leaves the `%(ext)s` the caller appends untouched.
fn ytdlp_output_stem(name: &str) -> String {
    media_stem(name).replace('%', "%%")
}

/// The part of a stored media file name yt-dlp keeps: `name` minus any
/// trailing [`STRIPPABLE_EXTENSIONS`]. yt-dlp writes the finished file as
/// `<media_stem>.<ext>`, which is what add-time name resolution has to
/// keep free. Unescaped — see [`ytdlp_output_stem`] for the template form.
pub(crate) fn media_stem(name: &str) -> &str {
    let mut stem = name;
    while let Some((base, ext)) = stem.rsplit_once('.') {
        if base.is_empty() || !STRIPPABLE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
            break;
        }
        stem = base;
    }
    stem
}

/// Drive a yt-dlp child process for a media-info-tagged download. Wraps
/// the result in [`engine::EngineError`] so the caller's match arms work
/// uniformly across engine and yt-dlp paths.
#[allow(clippy::too_many_arguments)]
async fn run_ytdlp(
    pool: &SqlitePool,
    events: &broadcast::Sender<CoreEvent>,
    info: ytdlp::MediaInfo,
    record: &DownloadRecord,
    user_agent: Option<String>,
    cancel: CancellationToken,
    tx: tokio::sync::broadcast::Sender<engine::ProgressEvent>,
    limit_rate_bps: u64,
) -> Result<engine::DownloadSummary, engine::EngineError> {
    let binary = tooling::resolve_path(Tool::YtDlp, pool)
        .await
        .ok_or_else(|| engine::EngineError::other("yt-dlp is not installed"))?;
    let ffmpeg = if info.needs_ffmpeg {
        Some(
            tooling::resolve_path(Tool::Ffmpeg, pool)
                .await
                .ok_or_else(|| {
                    engine::EngineError::other(
                        "ffmpeg is not installed but the selected format needs it",
                    )
                })?,
        )
    } else {
        None
    };

    let output_dir = record
        .output_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let output_template = record
        .output_path
        .file_name()
        .map(|n| format!("{}.%(ext)s", ytdlp_output_stem(&n.to_string_lossy())))
        .unwrap_or_else(|| "%(title)s.%(ext)s".to_string());

    // Browser impersonation toggle. Defaults to `true` when the row is
    // absent so a fresh DB (pre-migration cache, tests) still defeats
    // Cloudflare on captured streams.
    let impersonate = settings::get(pool, settings::settings_keys::YTDLP_IMPERSONATE)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // Scratch lives outside the download folder (`%TEMP%\Unduhin\<id>`),
    // keyed by download id so `--continue` finds the same `.part` after a
    // pause/resume or an app restart.
    let temp_dir = ytdlp::scratch_dir_for(&output_dir, record.id);

    let job = YtdlpJob {
        url: info.original_url.clone(),
        format_selector: info.format_selector.clone(),
        output_dir,
        temp_dir: temp_dir.clone(),
        output_template,
        binary_path: binary,
        ffmpeg_path: ffmpeg,
        user_agent,
        extra_headers: record.headers.clone().unwrap_or_default(),
        limit_rate_bps: (limit_rate_bps > 0).then_some(limit_rate_bps),
        impersonate,
    };

    let parsed_url = record
        .url
        .parse::<url::Url>()
        .unwrap_or_else(|_| url::Url::parse("about:blank").expect("about:blank parses"));

    match ytdlp::download(job, cancel.clone(), Some(tx)).await {
        Ok(outcome) => {
            // yt-dlp leaves the (now-empty) scratch dir behind after it
            // moves the finished file to `home:`. Only on success —
            // a cancel must leave the `.part`/`.ytdl` in place so a
            // resume can `--continue` from them.
            if let Err(e) = tokio::fs::remove_dir_all(&temp_dir).await {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::debug!(id = record.id, error = %e, "yt-dlp: scratch dir cleanup failed");
                }
            }
            // The DB row was inserted with a title-only stem (no
            // extension), a NULL total_bytes, and an "Other" category.
            // yt-dlp now knows the truth: extension after `%(ext)s`
            // expansion / post-mux, on-disk size, and therefore the
            // right category. Reconcile all four in one shot.
            let mut final_path = match outcome.final_path.clone() {
                Some(p) => p,
                None => ytdlp::resolve_final_path_fallback(&record.output_path)
                    .await
                    .unwrap_or_else(|| record.output_path.clone()),
            };
            let bytes_on_disk = tokio::fs::metadata(&final_path)
                .await
                .map(|m| m.len())
                .unwrap_or(outcome.bytes);

            // Own the name so finalize can move/rename the file out from
            // under this borrow (we reassign `final_path` to the moved path).
            if let Some(name) = final_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
            {
                match download::finalize_ytdlp_completion(
                    pool,
                    record.id,
                    &name,
                    &final_path,
                    bytes_on_disk,
                )
                .await
                {
                    Ok((moved_path, new_cat)) => {
                        let moved_name = moved_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .map(|s| s.to_string())
                            .unwrap_or(name);
                        let _ = events.send(CoreEvent::PathsChanged {
                            id: record.id,
                            filename: moved_name,
                            output_path: moved_path.to_string_lossy().into_owned(),
                        });
                        if new_cat.is_some() {
                            let _ = events.send(CoreEvent::CategoryChanged {
                                id: record.id,
                                category_id: new_cat,
                            });
                        }
                        // Report the post-move location in the summary.
                        final_path = moved_path;
                    }
                    Err(e) => tracing::warn!(id = record.id, error = %e,
                        "yt-dlp: failed to finalize download row"),
                }
            }
            Ok(engine::DownloadSummary {
                url: parsed_url,
                output: final_path,
                bytes: bytes_on_disk,
                segments: 1,
                resumed: false,
                // yt-dlp writes a real media file to disk; the HTML/empty
                // completion gate is an HTTP-engine concern only.
                content_type: None,
                // yt-dlp owns its own naming (`finalize_ytdlp_completion`
                // above); no engine-learned hint to apply.
                filename_hint: None,
            })
        }
        Err(ytdlp::YtdlpError::Process { message, .. }) if message == "cancelled" => {
            Err(engine::EngineError::Cancelled)
        }
        Err(e) => Err(engine::EngineError::other(e.to_string())),
    }
}

pub(crate) async fn timeouts(pool: &SqlitePool) -> (Duration, Duration) {
    let connect = settings::get(pool, settings::settings_keys::CONNECT_TIMEOUT_SECS)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_u64())
        .unwrap_or(15);
    let read = settings::get(pool, settings::settings_keys::READ_TIMEOUT_SECS)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_u64())
        .unwrap_or(60);
    (Duration::from_secs(connect), Duration::from_secs(read))
}

/// Read the `user_agent` setting. Empty strings normalize to `None`,
/// which lets the engine fall back to its compiled-in default.
pub(crate) async fn user_agent_setting(pool: &SqlitePool) -> Option<String> {
    settings::get(pool, settings::settings_keys::USER_AGENT)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty())
}

async fn current_status(pool: &SqlitePool, id: DownloadId) -> crate::error::Result<Status> {
    use sqlx::Row;
    let row = sqlx::query("SELECT status FROM downloads WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let row = row.ok_or(crate::error::CoreError::DownloadNotFound(id))?;
    let s: String = row.get("status");
    s.parse()
}

/// Decide whether an HTTP-engine download the engine reported as
/// "complete" should actually be failed. One-click file hosts (and any
/// link that needs a live browser session/token to resolve) routinely
/// answer the captured bare URL with a 0-byte body or an HTML landing
/// page in place of the real file. The engine cannot tell that apart
/// from a genuine transfer — when no `Content-Length` was advertised it
/// treats whatever arrived (including nothing) as the whole file — so
/// without this gate the row flips to `Completed` at 0 B and a failed
/// download masquerades as a success (silent data loss).
///
/// Returns `Some(reason)` when the download must be marked `Failed`
/// instead of `Completed`, or `None` when it looks like a real file.
/// Scoped to the engine path by the caller — yt-dlp rows always write a
/// real media file to disk.
fn http_completion_rejection(summary: &engine::DownloadSummary) -> Option<String> {
    if summary.bytes == 0 {
        return Some(
            "server returned an empty response (0 bytes); this link may need to be \
             opened in the browser to resolve"
                .to_string(),
        );
    }
    // An HTML body where a file was expected is the other common
    // one-click-host failure: the captured URL resolves to a landing /
    // click-through page, not the bytes. Only treat it as a failure when
    // the target file isn't itself an HTML document.
    let is_html = summary
        .content_type
        .as_deref()
        .map(|ct| {
            ct.split(';')
                .next()
                .unwrap_or(ct)
                .trim()
                .eq_ignore_ascii_case("text/html")
        })
        .unwrap_or(false);
    if is_html {
        let ext = summary
            .output
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        let expects_html = matches!(ext.as_deref(), Some("html") | Some("htm"));
        if !expects_html {
            return Some(
                "server returned an HTML page instead of the requested file; this link \
                 may need to be opened in the browser to resolve"
                    .to_string(),
            );
        }
    }
    None
}

/// Sort an engine failure into the typed reason persisted on the row.
///
/// The interesting case is `TerminalStatus`. `retry::classify` puts every
/// 4xx in `Terminal`, which lumps "your link expired" together with "no such
/// file". 401, 403 and 410 are the three that a fresh URL or fresh cookies can
/// actually fix, so only those become [`ErrorKind::ExpiredAuth`].
///
/// This does not overlap with `looks_like_concurrency_limit` in the engine,
/// which also keys on 403: that runs *inside* the transfer and retries
/// single-stream first. By the time an error reaches here, that fallback has
/// already been tried and lost.
///
/// `RemoteChanged` is deliberately `Other`. The file moved on — a different
/// problem from an expired link, and the queue already restarts those.
fn classify_engine_error(err: &engine::EngineError) -> ErrorKind {
    match err {
        engine::EngineError::TerminalStatus {
            status: 401 | 403 | 410,
        } => ErrorKind::ExpiredAuth,
        engine::EngineError::Io { .. } => ErrorKind::Disk,
        engine::EngineError::TransientStatus { .. }
        | engine::EngineError::RetryExhausted { .. }
        | engine::EngineError::BodyTruncated { .. }
        | engine::EngineError::RangeMismatch { .. } => ErrorKind::Network,
        _ => ErrorKind::Other,
    }
}

async fn mark_worker_failed(
    pool: &SqlitePool,
    events: &broadcast::Sender<CoreEvent>,
    id: DownloadId,
    err: &str,
    kind: ErrorKind,
) {
    // Read the current status before flipping so the StatusChanged
    // event reports the actual transition (Active → Failed or
    // Muxing → Failed). The timeline UI uses `from` for messaging.
    let from = current_status(pool, id).await.unwrap_or(Status::Active);
    let _ = download::mark_failed(pool, id, err, kind).await;
    let _ = events.send(CoreEvent::StatusChanged {
        id,
        from,
        to: Status::Failed,
    });
    let _ = events.send(CoreEvent::Failed {
        id,
        error: err.to_string(),
        error_kind: kind,
    });
}

/// Fixed length the persisted speed series is downsampled to. Matches the
/// renderer's `SPEED_HISTORY_LEN` ring buffer so the rebuilt sparkline has
/// the same resolution as a live one.
const SPEED_SAMPLE_CAP: usize = 60;

/// Trim trailing zero samples (the engine's final tick reports `0`, which
/// would otherwise flatten the sparkline's tail) and bucket-average the
/// remainder down to at most `cap` points. Returns an empty vec when there
/// is no non-zero signal.
fn downsample_speeds(samples: &[u32], cap: usize) -> Vec<u32> {
    // Drop the trailing run of zeros (download finished / stalled at end).
    let end = samples
        .iter()
        .rposition(|&s| s > 0)
        .map(|i| i + 1)
        .unwrap_or(0);
    let trimmed = &samples[..end];
    if trimmed.is_empty() || cap == 0 {
        return Vec::new();
    }
    if trimmed.len() <= cap {
        return trimmed.to_vec();
    }
    // Bucket-average into `cap` points so the shape is preserved.
    let mut out = Vec::with_capacity(cap);
    for bucket in 0..cap {
        let start = bucket * trimmed.len() / cap;
        let stop = ((bucket + 1) * trimmed.len() / cap).max(start + 1);
        let slice = &trimmed[start..stop.min(trimmed.len())];
        let avg = slice.iter().map(|&s| s as u64).sum::<u64>() / slice.len() as u64;
        out.push(avg as u32);
    }
    out
}

/// Per-session cap on how many times a single download may be restarted
/// from scratch in response to `RemoteChanged`. Resets across app restarts
/// (the map is in-memory) so a legitimately-changing remote isn't blocked
/// forever, but a remote that changes on every attempt can't spin endlessly
/// within one session.
const MAX_REMOTE_CHANGED_RESTARTS: u32 = 3;

/// In-memory tally of `RemoteChanged` restarts keyed by download id.
static REMOTE_CHANGED_RESTARTS: std::sync::OnceLock<std::sync::Mutex<HashMap<DownloadId, u32>>> =
    std::sync::OnceLock::new();

/// Increment and return the restart count for `id`.
fn record_remote_changed_restart(id: DownloadId) -> u32 {
    let map = REMOTE_CHANGED_RESTARTS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
    let entry = guard.entry(id).or_insert(0);
    *entry += 1;
    *entry
}

/// Translate an engine [`ProgressEvent::SwarmProgress`] into the swarm
/// snapshot to persist and the [`CoreEvent::SwarmProgress`] to re-emit
/// (design §3.C). Pure mapping, split out so the field correspondence is
/// unit-testable without spinning up a worker. Returns `None` for any other
/// variant (the pump only calls it on `SwarmProgress`).
fn translate_swarm(id: DownloadId, ev: &ProgressEvent) -> Option<(SwarmStats, CoreEvent)> {
    let ProgressEvent::SwarmProgress {
        peers,
        seeds,
        up_bps,
        down_bps,
        ratio_milli,
    } = *ev
    else {
        return None;
    };
    let swarm = SwarmStats {
        peers,
        seeds,
        up_bps,
        down_bps,
        ratio_milli,
    };
    let core_ev = CoreEvent::SwarmProgress {
        id,
        peers,
        seeds,
        up_bps,
        down_bps,
        ratio_milli,
    };
    Some((swarm, core_ev))
}

/// Translate an engine [`ProgressEvent::FileProgress`] into a
/// [`CoreEvent::TorrentFileProgress`] (renamed at the wire boundary, mirroring
/// the `SegmentProgress` rename). Pure mapping; `None` for other variants.
fn translate_file_progress(id: DownloadId, ev: &ProgressEvent) -> Option<CoreEvent> {
    let ProgressEvent::FileProgress {
        index,
        downloaded,
        total,
    } = *ev
    else {
        return None;
    };
    Some(CoreEvent::TorrentFileProgress {
        id,
        index,
        downloaded,
        total,
    })
}

async fn read_sidecar_segments(path: &Path) -> Option<Vec<engine::SegmentState>> {
    if !path.exists() {
        return None;
    }
    let bytes = tokio::fs::read(path).await.ok()?;
    let meta: Meta = serde_json::from_slice(&bytes).ok()?;
    Some(meta.segments)
}

// Used in worker for `record.output_path.clone()` early; keep PathBuf
// available without pulling another module just to thread the type.
#[allow(dead_code)]
fn _path_marker() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_stem_keeps_a_title_that_contains_dots() {
        // The reported bug. `Path::file_stem` cut at the last dot, so the
        // template became `Ep.%(ext)s` and the file landed as `Ep.mp4`.
        assert_eq!(
            ytdlp_output_stem("Ep. 5 - The Finale"),
            "Ep. 5 - The Finale"
        );
        // A short, purely numeric tail — a length heuristic would have
        // eaten this one.
        assert_eq!(ytdlp_output_stem("Part 2.5"), "Part 2.5");
        // Short, alphanumeric, and carrying a letter — an "looks like an
        // extension" heuristic would have eaten this one too.
        assert_eq!(ytdlp_output_stem("Show S01.E05"), "Show S01.E05");
        assert_eq!(ytdlp_output_stem("Vol. 3"), "Vol. 3");
    }

    #[test]
    fn output_stem_drops_a_real_extension() {
        // yt-dlp appends the extension itself, so one already on the name
        // has to go or the file lands as `clip.mp4.mp4`.
        assert_eq!(ytdlp_output_stem("clip.mp4"), "clip");
        assert_eq!(ytdlp_output_stem("song.flac"), "song");
        assert_eq!(ytdlp_output_stem("movie.MKV"), "movie");
    }

    #[test]
    fn output_stem_strips_a_stacked_manifest_extension() {
        // What an HLS capture's name looks like when the URL tail carried
        // both — one pass would leave `name.mp4` and yield `name.mp4.mp4`.
        assert_eq!(ytdlp_output_stem("name.mp4.m3u8"), "name");
    }

    #[test]
    fn output_stem_stops_before_eating_the_whole_name() {
        // A name that is nothing but extensions must not reduce to "".
        assert_eq!(ytdlp_output_stem(".mp4"), ".mp4");
        assert_eq!(ytdlp_output_stem("mp4"), "mp4");
        // A title whose own words happen to be an extension keeps them:
        // stripping stops as soon as the base would be empty.
        assert_eq!(ytdlp_output_stem(".ts.ts"), ".ts");
    }

    #[test]
    fn output_stem_escapes_percent_signs() {
        // yt-dlp reads `%(field)s` in an output template, and
        // `sanitize_filename` does not remove `%`. Unescaped, this title
        // hands yt-dlp a field spec built from the user's video title.
        assert_eq!(ytdlp_output_stem("Save 50%(off) now"), "Save 50%%(off) now");
        assert_eq!(ytdlp_output_stem("100% real"), "100%% real");
        // The escape survives alongside an extension strip.
        assert_eq!(ytdlp_output_stem("50% off.mp4"), "50%% off");
    }

    #[test]
    fn downsample_trims_trailing_zeros() {
        assert_eq!(downsample_speeds(&[10, 20, 0, 0], 60), vec![10, 20]);
        // All-zero (or empty) signal yields nothing to plot.
        assert!(downsample_speeds(&[0, 0, 0], 60).is_empty());
        assert!(downsample_speeds(&[], 60).is_empty());
    }

    #[test]
    fn downsample_caps_length_and_preserves_shape() {
        let samples: Vec<u32> = (1..=300).collect();
        let out = downsample_speeds(&samples, 60);
        assert_eq!(out.len(), 60);
        // Monotonic input stays monotonic after bucket-averaging.
        assert!(out.windows(2).all(|w| w[0] <= w[1]));
        assert!(*out.first().unwrap() < *out.last().unwrap());
    }

    /// An in-memory `QueueManager` with no workers. Enough to exercise
    /// the claim loop, which only touches the pool and the active map.
    async fn test_manager() -> QueueManager {
        // `max_connections(1)` because each connection to
        // `sqlite::memory:` would otherwise get its own empty database.
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory pool");
        crate::db::migrate(&pool).await.expect("migrate");
        let schedules = SchedulesCache::load(&pool).await.expect("schedules");
        let (events, _rx) = broadcast::channel(64);
        QueueManager {
            pool,
            events,
            active: Arc::new(Mutex::new(HashMap::new())),
            shutdown: CancellationToken::new(),
            drain: Mutex::new(DrainTracker::default()),
            schedules: Arc::new(RwLock::new(schedules)),
            torrent_engine: Arc::new(TorrentEngineCell::new()),
            rate_limiter: engine::TokenBucket::new(0),
        }
    }

    /// Insert a bare `queued` row and return its id.
    async fn insert_queued_row(pool: &SqlitePool) -> DownloadId {
        sqlx::query(
            "INSERT INTO downloads (url, filename, output_path, status, priority, segments, \
             created_at) VALUES (?, ?, ?, 'queued', 0, 1, ?)",
        )
        .bind("https://example.com/x.bin")
        .bind("x.bin")
        .bind("C:/tmp/x.bin")
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .expect("insert");
        sqlx::query_scalar("SELECT id FROM downloads ORDER BY id DESC LIMIT 1")
            .fetch_one(pool)
            .await
            .expect("id")
    }

    #[tokio::test]
    async fn fill_capacity_never_spawns_a_second_worker_for_one_download() {
        // The bug this locks down: `fill_capacity` used to claim any
        // `queued` row without checking the active map, and
        // `active.insert` then *replaced* the live handle — dropping the
        // previous `JoinHandle` (detaching the task) and the last clone of
        // its `CancellationToken`. The old worker's yt-dlp kept
        // downloading to the same path forever, unreachable by pause,
        // delete, and shutdown alike. A pause immediately followed by a
        // resume was enough to trigger it, because `reconcile_active`
        // cancels without waiting for the worker to exit.
        let manager = test_manager().await;
        let id = insert_queued_row(&manager.pool).await;

        // Stand in for a worker that was told to stop but hasn't exited.
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        manager.active.lock().await.insert(
            id,
            ActiveHandle {
                cancel,
                join: tokio::spawn(async move { stop.cancelled().await }),
                control: None,
                cancel_ticks: 1,
            },
        );

        manager.fill_capacity().await.expect("fill pass");

        assert_eq!(
            manager.active.lock().await.len(),
            1,
            "a second worker was spawned for a download that already has one"
        );
        // The row must stay `queued` so a later tick can claim it once the
        // old worker is gone — claiming it now would strand it as `active`
        // with nothing running.
        let status: String = sqlx::query_scalar("SELECT status FROM downloads WHERE id = ?")
            .bind(id)
            .fetch_one(&manager.pool)
            .await
            .expect("status");
        assert_eq!(status, "queued");

        // And once the old worker really exits, the reap frees the slot.
        let handle = manager.active.lock().await.remove(&id).expect("handle");
        handle.cancel.cancel();
        let _ = handle.join.await;
    }

    #[tokio::test]
    async fn reconcile_keeps_the_handle_so_the_row_cannot_be_reclaimed() {
        // Cancelling must not remove the handle: while it is in the map,
        // `fill_capacity` skips the id, which is what serializes a
        // pause→resume into one worker at a time.
        let manager = test_manager().await;
        let id = insert_queued_row(&manager.pool).await;
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        manager.active.lock().await.insert(
            id,
            ActiveHandle {
                cancel,
                join: tokio::spawn(async move { stop.cancelled().await }),
                control: None,
                cancel_ticks: 0,
            },
        );

        // The row is `queued`, so it is not in `active_ids` and the
        // reconcile pass should cancel — but keep — the worker.
        manager.reconcile_active().await.expect("reconcile");

        let active = manager.active.lock().await;
        let handle = active.get(&id).expect("handle must survive the cancel");
        assert!(handle.cancel.is_cancelled());
        assert_eq!(handle.cancel_ticks, 1);
    }

    fn summary(bytes: u64, output: &str, content_type: Option<&str>) -> engine::DownloadSummary {
        engine::DownloadSummary {
            url: "https://example.com/x".parse().unwrap(),
            output: std::path::PathBuf::from(output),
            bytes,
            segments: 1,
            resumed: false,
            content_type: content_type.map(|s| s.to_string()),
            filename_hint: None,
        }
    }

    /// Only the three statuses a fresh link can actually fix become
    /// `ExpiredAuth`. 404 and 400 share the `Terminal` retry class but no
    /// amount of re-capturing helps them, so offering "Refresh link" there
    /// would send the user on a pointless trip to the browser.
    #[test]
    fn classify_maps_only_auth_statuses_to_expired_auth() {
        for status in [401u16, 403, 410] {
            assert_eq!(
                classify_engine_error(&engine::EngineError::TerminalStatus { status }),
                ErrorKind::ExpiredAuth,
                "{status} should be expired_auth"
            );
        }
        for status in [400u16, 404, 451] {
            assert_eq!(
                classify_engine_error(&engine::EngineError::TerminalStatus { status }),
                ErrorKind::Other,
                "{status} should not be expired_auth"
            );
        }
    }

    #[test]
    fn classify_maps_transient_failures_to_network() {
        assert_eq!(
            classify_engine_error(&engine::EngineError::TransientStatus { status: 503 }),
            ErrorKind::Network
        );
        // A file that moved on is not an expired link — the queue already
        // restarts those, bounded.
        assert_eq!(
            classify_engine_error(&engine::EngineError::RemoteChanged),
            ErrorKind::Other
        );
    }

    #[test]
    fn completion_gate_rejects_zero_bytes() {
        // a 0-byte "completed" download is silent data loss.
        let r = http_completion_rejection(&summary(0, "file.zip", Some("application/zip")));
        assert!(r.is_some(), "0 bytes must be rejected");
        assert!(r.unwrap().contains("empty"));
    }

    #[test]
    fn completion_gate_rejects_html_when_file_expected() {
        // One-click hosts answer with a landing page; the target isn't HTML.
        let r = http_completion_rejection(&summary(
            5_000,
            "movie.mp4",
            Some("text/html; charset=utf-8"),
        ));
        assert!(
            r.is_some(),
            "HTML body for a non-HTML target must be rejected"
        );
        assert!(r.unwrap().contains("HTML"));
    }

    #[test]
    fn completion_gate_allows_real_file() {
        // A non-empty body with a sensible content-type is a real download.
        assert!(
            http_completion_rejection(&summary(5_000, "movie.mp4", Some("video/mp4"))).is_none()
        );
        // Missing content-type is fine as long as bytes arrived.
        assert!(http_completion_rejection(&summary(5_000, "movie.mp4", None)).is_none());
    }

    #[test]
    fn completion_gate_allows_html_when_html_expected() {
        // The user genuinely wanted the page (filename ends in .html).
        assert!(
            http_completion_rejection(&summary(5_000, "page.html", Some("text/html"))).is_none()
        );
    }

    #[test]
    fn translate_swarm_maps_fields_and_tags_id() {
        // The pump translates the engine's torrent SwarmProgress into the
        // persisted snapshot + the CoreEvent verbatim, stamping the id.
        let ev = ProgressEvent::SwarmProgress {
            peers: 7,
            seeds: 19,
            up_bps: 2_048,
            down_bps: 999_999,
            ratio_milli: 1234,
        };
        let (swarm, core_ev) = translate_swarm(42, &ev).expect("maps a SwarmProgress");
        assert_eq!(
            swarm,
            SwarmStats {
                peers: 7,
                seeds: 19,
                up_bps: 2_048,
                down_bps: 999_999,
                ratio_milli: 1234,
            }
        );
        match core_ev {
            CoreEvent::SwarmProgress {
                id,
                peers,
                seeds,
                up_bps,
                down_bps,
                ratio_milli,
            } => {
                assert_eq!(id, 42);
                assert_eq!(peers, 7);
                assert_eq!(seeds, 19);
                assert_eq!(up_bps, 2_048);
                assert_eq!(down_bps, 999_999);
                assert_eq!(ratio_milli, 1234);
            }
            other => panic!("expected SwarmProgress, got {other:?}"),
        }
    }

    #[test]
    fn translate_file_progress_renames_to_torrent_file_progress() {
        // FileProgress re-emits as TorrentFileProgress (the wire rename) with
        // its byte counters intact and the row id stamped on.
        let ev = ProgressEvent::FileProgress {
            index: 3,
            downloaded: 512,
            total: 4_096,
        };
        match translate_file_progress(9, &ev).expect("maps a FileProgress") {
            CoreEvent::TorrentFileProgress {
                id,
                index,
                downloaded,
                total,
            } => {
                assert_eq!(id, 9);
                assert_eq!(index, 3);
                assert_eq!(downloaded, 512);
                assert_eq!(total, 4_096);
            }
            other => panic!("expected TorrentFileProgress, got {other:?}"),
        }
    }

    #[test]
    fn translate_helpers_ignore_unrelated_variants() {
        // The translators are only meaningful for their own variant; a
        // mismatched event yields None rather than a wrong-shaped CoreEvent.
        let tick = ProgressEvent::Tick {
            downloaded: 1,
            total: Some(2),
            speed_bps: 3.0,
            eta: None,
        };
        assert!(translate_swarm(1, &tick).is_none());
        assert!(translate_file_progress(1, &tick).is_none());
    }

    #[test]
    fn core_swarm_event_serializes_snake_case() {
        // Lock the wire contract Phase 3b binds against: the tag and field
        // names must be snake_case `swarm_progress` / `torrent_file_progress`.
        let swarm = CoreEvent::SwarmProgress {
            id: 1,
            peers: 2,
            seeds: 3,
            up_bps: 4,
            down_bps: 5,
            ratio_milli: 6,
        };
        let v: serde_json::Value = serde_json::to_value(&swarm).unwrap();
        assert_eq!(v["type"], "swarm_progress");
        assert_eq!(v["peers"], 2);
        assert_eq!(v["seeds"], 3);
        assert_eq!(v["up_bps"], 4);
        assert_eq!(v["down_bps"], 5);
        assert_eq!(v["ratio_milli"], 6);

        let file = CoreEvent::TorrentFileProgress {
            id: 1,
            index: 0,
            downloaded: 10,
            total: 20,
        };
        let v: serde_json::Value = serde_json::to_value(&file).unwrap();
        assert_eq!(v["type"], "torrent_file_progress");
        assert_eq!(v["index"], 0);
        assert_eq!(v["downloaded"], 10);
        assert_eq!(v["total"], 20);
    }
}
