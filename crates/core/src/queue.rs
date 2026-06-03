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
use tokio::sync::{broadcast, mpsc, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use url::Url;

use crate::download::{self, DownloadId, DownloadRecord, Status};
use crate::event::CoreEvent;
use crate::schedule::{self, SchedulesCache};
use crate::settings;
use crate::tooling::{self, Tool};
use crate::ytdlp::{self, YtdlpJob};

/// Period of the manager's reconciliation loop when nothing pokes it.
const TICK_PERIOD: Duration = Duration::from_millis(500);

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
}

impl QueueManager {
    pub(crate) async fn spawn(
        pool: SqlitePool,
        events: broadcast::Sender<CoreEvent>,
        schedules: Arc<RwLock<SchedulesCache>>,
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
    async fn reconcile_active(&self) -> crate::error::Result<()> {
        let db_active = download::active_ids(&self.pool).await?;
        let active = self.active.lock().await;
        let to_cancel: Vec<DownloadId> = active
            .keys()
            .copied()
            .filter(|id| !db_active.contains(id))
            .collect();
        for id in to_cancel {
            if let Some(h) = active.get(&id) {
                tracing::debug!(id, "queue: cancelling worker (db status changed)");
                h.cancel.cancel();
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
            // Only engine-driven downloads use the control channel.
            // yt-dlp paths ignore it.
            let (control_tx, control_rx) = if record.media_info.is_none() {
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
            );
            active.insert(
                id,
                ActiveHandle {
                    cancel,
                    join,
                    control: control_tx,
                },
            );
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
) -> JoinHandle<()> {
    tokio::spawn(async move {
        run_worker(pool, events, record, cancel, control_rx).await;
    })
}

async fn run_worker(
    pool: SqlitePool,
    events: broadcast::Sender<CoreEvent>,
    record: DownloadRecord,
    cancel: CancellationToken,
    control_rx: Option<mpsc::Receiver<Control>>,
) {
    let id = record.id;
    let output_path = record.output_path.clone();
    let meta_path = Meta::sidecar_path(&output_path);
    let pump_meta_path = meta_path.clone();
    let url = match record.url.parse::<Url>() {
        Ok(u) => u,
        Err(e) => {
            mark_worker_failed(&pool, &events, id, &format!("invalid url: {e}")).await;
            return;
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
    let pump_url = url.clone();
    let pump = tokio::spawn(async move {
        // yt-dlp downloads multi-stream formats (video + audio) one
        // stream at a time. Each stream's progress restarts from byte 0,
        // which would otherwise look like the bar snapping back to 0 %.
        // We detect the regression and flip the row to `Muxing` so the
        // UI can show a distinct state instead of confusing the user.
        let mut last_downloaded: u64 = 0;
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
                    // Phase transition: a meaningful drop in the byte
                    // counter means yt-dlp moved to a new stream. We
                    // only fire this once per run; subsequent stream
                    // boundaries are still treated as `Muxing`.
                    if !muxing_emitted
                        && downloaded + 1024 < last_downloaded
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
                    last_downloaded = downloaded;

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
                    // The engine learned the real filename from the response
                    // headers while the bytes are still flowing. Update the
                    // row's display name + category now so the UI stops showing
                    // the random URL slug — the on-disk file is relocated to
                    // match at completion (`apply_engine_filename`).
                    match download::mark_learned_filename(&pump_pool, id, &pump_url, &hint).await {
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
    let result: Result<engine::DownloadSummary, engine::EngineError> =
        if let Some(info) = record.media_info.clone() {
            // yt-dlp-driven downloads bypass the engine entirely. The pump
            // still receives `ProgressEvent`s — yt-dlp produces them via
            // `ytdlp::download` parsing `--progress-template` lines.
            run_ytdlp(
                &pool,
                &events,
                info,
                &record,
                user_agent.clone(),
                cancel.clone(),
                tx.clone(),
            )
            .await
        } else if meta_path.exists() {
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
            )
            .await
        } else {
            let mut opts = DownloadOptions::new(url.clone(), output_path.clone());
            opts.segments = segments;
            opts.connect_timeout = connect_timeout;
            opts.read_timeout = read_timeout;
            opts.backoff = backoff;
            opts.user_agent = user_agent.clone();
            opts.headers = record.headers.clone().unwrap_or_default();
            engine::download_with_control(opts, cancel.clone(), Some(tx.clone()), control_rx).await
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
            // engine path: yt-dlp rows always produce a real media file.
            if record.media_info.is_none() {
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
                        mark_worker_failed(&pool, &events, id, &reason).await;
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
                    match download::apply_engine_filename(&pool, id, &url, hint).await {
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
                mark_worker_failed(&pool, &events, id, &err.to_string()).await;
            }
        }
    }
}

/// Drive a yt-dlp child process for a media-info-tagged download. Wraps
/// the result in [`engine::EngineError`] so the caller's match arms work
/// uniformly across engine and yt-dlp paths.
async fn run_ytdlp(
    pool: &SqlitePool,
    events: &broadcast::Sender<CoreEvent>,
    info: ytdlp::MediaInfo,
    record: &DownloadRecord,
    user_agent: Option<String>,
    cancel: CancellationToken,
    tx: tokio::sync::broadcast::Sender<engine::ProgressEvent>,
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
        .map(|n| {
            // The frontend supplies a stem; let yt-dlp pick the final
            // extension based on the chosen format.
            let stem = std::path::Path::new(n)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "download".to_string());
            format!("{stem}.%(ext)s")
        })
        .unwrap_or_else(|| "%(title)s.%(ext)s".to_string());

    let job = YtdlpJob {
        url: info.original_url.clone(),
        format_selector: info.format_selector.clone(),
        output_dir,
        output_template,
        binary_path: binary,
        ffmpeg_path: ffmpeg,
        user_agent,
        extra_headers: record.headers.clone().unwrap_or_default(),
    };

    let parsed_url = record
        .url
        .parse::<url::Url>()
        .unwrap_or_else(|_| url::Url::parse("about:blank").expect("about:blank parses"));

    match ytdlp::download(job, cancel.clone(), Some(tx)).await {
        Ok(outcome) => {
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

async fn timeouts(pool: &SqlitePool) -> (Duration, Duration) {
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
async fn user_agent_setting(pool: &SqlitePool) -> Option<String> {
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

async fn mark_worker_failed(
    pool: &SqlitePool,
    events: &broadcast::Sender<CoreEvent>,
    id: DownloadId,
    err: &str,
) {
    // Read the current status before flipping so the StatusChanged
    // event reports the actual transition (Active → Failed or
    // Muxing → Failed). The timeline UI uses `from` for messaging.
    let from = current_status(pool, id).await.unwrap_or(Status::Active);
    let _ = download::mark_failed(pool, id, err).await;
    let _ = events.send(CoreEvent::StatusChanged {
        id,
        from,
        to: Status::Failed,
    });
    let _ = events.send(CoreEvent::Failed {
        id,
        error: err.to_string(),
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
        assert!(r.is_some(), "HTML body for a non-HTML target must be rejected");
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
}
