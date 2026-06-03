//! Worker-queue transfer engine with live re-segmentation.
//!
//! Each persistent worker owns its own `mpsc::Receiver<Segment>`; the
//! initial plan pushes one range per worker. 7b keeps the senders alive
//! in [`SharedState`] so a supervisor task can drive split/join via
//! [`Control::SetSegments`] without restarting the transfer.
//!
//! The ticker is the single source of truth for per-segment telemetry:
//! every [`TICK_INTERVAL`] it samples each worker's cumulative bytes
//! through its own [`SpeedMeter`], computes the median across active
//! workers, and emits one [`ProgressEvent::SegmentProgress`] per
//! worker with the smoothed speed and a [`SegmentRuntimeState`] verdict
//! (Active | Done | Slow | Stalled). The samplers vec grows on the fly
//! when the supervisor adds new workers.
//!
//! Live truncation: split shrinks a donor's `segment.end` in `meta`.
//! The worker's body-read loop reads the live end inside its post-chunk
//! lock and exits early once `bytes_downloaded` reaches it. The TCP
//! body beyond the new end is dropped; the new worker re-issues a Range
//! request for the second half. Byte-exact assembly is preserved because
//! the source bytes are deterministic and any overshoot before the
//! truncation check is overwritten by the new worker with identical
//! content.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use reqwest::header::{IF_RANGE, RANGE};
use reqwest::{Client, StatusCode};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tokio::task::JoinSet;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::download::{DownloadOptions, DownloadSummary};
use crate::error::{EngineError, Result};
use crate::meta::{Meta, SegmentState};
use crate::progress::{emit, eta, ProgressEvent, SegmentRuntimeState, SpeedMeter};
use crate::retry::{classify_reqwest, Backoff, RetryClass};
use crate::segment::Segment;

pub(crate) const TICK_INTERVAL: Duration = Duration::from_millis(250);
pub(crate) const SPEED_ALPHA: f64 = 0.3;

/// Slow-start cadence: when ramping toward the target connection count, add
/// at most one connection per interval. Each add is confirm-before-commit
/// (see [`try_probe_and_split`]), so we never burst — the gap just paces
/// how quickly a healthy host reaches full parallelism (~target × interval).
const RAMP_INTERVAL: Duration = Duration::from_millis(500);
const WRITE_CHUNK_FLUSH_BYTES: u64 = 64 * 1024;

/// A worker that produced zero bytes for at least this long is reported
/// as [`SegmentRuntimeState::Stalled`].
const STALLED_AFTER: Duration = Duration::from_secs(5);

/// Number of consecutive "below median*0.5" ticks before a worker is
/// reported as [`SegmentRuntimeState::Slow`].
const SLOW_TICK_THRESHOLD: u32 = 2;

/// Inclusive bounds on the segment count for live re-segmentation.
pub const MAX_SEGMENTS: usize = 32;
pub const MIN_SEGMENTS: usize = 1;

/// External control messages driving live re-segmentation. Sent through
/// an `mpsc::Sender<Control>` held by the core layer.
#[derive(Debug)]
pub enum Control {
    /// Reshape the active worker pool to `n`. Bounded
    /// [`MIN_SEGMENTS`]`..=`[`MAX_SEGMENTS`]. The `ack` channel reports
    /// validation / dispatch result (not transfer completion).
    SetSegments {
        n: usize,
        ack: oneshot::Sender<Result<()>>,
    },
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_transfer(
    client: Client,
    opts: DownloadOptions,
    meta: Meta,
    meta_path: PathBuf,
    cancel: CancellationToken,
    tx: Option<broadcast::Sender<ProgressEvent>>,
    resumed: bool,
    mut control_rx: Option<mpsc::Receiver<Control>>,
    content_type: Option<String>,
    filename_hint: Option<String>,
    prefetched: Option<reqwest::Response>,
    // When `Some(n)`, slow-start: begin with the segments already in `meta`
    // (normally one) and ramp the connection count up toward `n`, backing
    // off if the host refuses a new connection. `None` keeps the fixed
    // plan in `meta` (resume / single-stream).
    ramp_target: Option<usize>,
) -> Result<DownloadSummary> {
    let total = meta.total_bytes;
    let initial_segment_count = meta.segments.len();
    let resumed_bytes = meta.downloaded_total();
    let ranges_supported = meta.accept_ranges;

    emit(
        tx.as_ref(),
        ProgressEvent::Started {
            total,
            segments: initial_segment_count,
            resumed_bytes,
        },
    );

    // Surface the name learned from the response headers immediately, while
    // the bytes are still flowing, so the UI can replace a random URL slug
    // with the real filename (and re-categorize) mid-download instead of
    // surprising the user at completion. The file on disk is left at its
    // working path — the engine is still writing it; the physical
    // rename/relocate happens once at completion from the same hint carried
    // on `DownloadSummary::filename_hint`.
    if let Some(hint) = filename_hint.as_deref().filter(|h| !h.is_empty()) {
        emit(
            tx.as_ref(),
            ProgressEvent::FilenameLearned {
                hint: hint.to_string(),
            },
        );
    }

    let shared = Arc::new(SharedState {
        meta: Mutex::new(meta),
        meta_path: meta_path.clone(),
        tx: tx.clone(),
        backoff: opts.backoff,
        ranges_supported,
        senders: Mutex::new(Vec::with_capacity(initial_segment_count)),
        prefetched: Mutex::new(
            prefetched
                .map(|r| std::collections::HashMap::from([(0usize, r)]))
                .unwrap_or_default(),
        ),
    });

    let mut workers: JoinSet<Result<()>> = JoinSet::new();

    // Spawn the initial worker pool. Each worker gets one segment and
    // its own sender, which is parked in `shared.senders` so the
    // supervisor can later push more segments (split) or close it (join).
    {
        let initial_plan: Vec<Segment> = {
            let m = shared.meta.lock().await;
            m.segments.iter().map(|s| s.segment).collect()
        };
        let mut senders_guard = shared.senders.lock().await;
        for (index, seg) in initial_plan.into_iter().enumerate() {
            let (tx_seg, rx_seg) = mpsc::channel::<Segment>(16);
            let _ = tx_seg.send(seg).await;
            senders_guard.push(Some(tx_seg));
            spawn_worker(
                &mut workers,
                client.clone(),
                opts.url.clone(),
                index,
                rx_seg,
                shared.clone(),
                cancel.clone(),
            );
        }
    }

    let ticker_shared = shared.clone();
    let ticker_cancel = cancel.clone();
    let ticker = tokio::spawn(async move {
        ticker_loop(ticker_shared, total, ticker_cancel).await;
    });

    let mut first_err: Option<EngineError> = None;
    if let Err(e) = supervisor_loop(
        &shared,
        &client,
        &opts.url,
        &cancel,
        &mut workers,
        &mut control_rx,
        ramp_target,
    )
    .await
    {
        cancel.cancel();
        first_err = Some(e);
    }

    // Drain any straggling workers if supervisor exited early on error.
    while let Some(joined) = workers.join_next().await {
        match joined {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                if first_err.is_none() {
                    cancel.cancel();
                    first_err = Some(e);
                }
            }
            Err(join_err) => {
                cancel.cancel();
                if first_err.is_none() {
                    first_err = Some(EngineError::other(format!("worker panicked: {join_err}")));
                }
            }
        }
    }

    // Stop the ticker AFTER workers complete so we keep emitting progress
    // until the last byte.
    cancel.cancel();
    let _ = ticker.await;

    {
        let m = shared.meta.lock().await;
        m.save(&meta_path).await?;
    }

    if let Some(err) = first_err {
        emit(
            shared.tx.as_ref(),
            ProgressEvent::Failed {
                error: err.to_string(),
            },
        );
        return Err(err);
    }

    // Guarantee a Done verdict for every segment lands in the event
    // stream — the last ticker iteration may have fired before the
    // final bytes were written. Renderers can rely on this.
    let final_snapshot: Vec<(usize, u64, u64)> = {
        let m = shared.meta.lock().await;
        m.segments
            .iter()
            .map(|s| (s.segment.index, s.bytes_downloaded, s.segment.len()))
            .collect()
    };
    let final_segment_count = final_snapshot.len();
    for (index, bytes_downloaded, segment_total) in final_snapshot {
        emit(
            shared.tx.as_ref(),
            ProgressEvent::SegmentProgress {
                index,
                bytes_downloaded,
                segment_total,
                speed_bps: 0.0,
                state: SegmentRuntimeState::Done,
            },
        );
    }

    let bytes = {
        let m = shared.meta.lock().await;
        if m.total_bytes.is_some() && !m.is_complete() {
            return Err(EngineError::other(
                "transfer finished without completing all segments",
            ));
        }
        m.downloaded_total()
    };
    Meta::delete(&meta_path).await.ok();
    emit(shared.tx.as_ref(), ProgressEvent::Completed { bytes });

    Ok(DownloadSummary {
        url: opts.url,
        output: opts.output,
        bytes,
        segments: final_segment_count,
        resumed,
        content_type,
        filename_hint,
    })
}

async fn supervisor_loop(
    shared: &Arc<SharedState>,
    client: &Client,
    url: &Url,
    cancel: &CancellationToken,
    workers: &mut JoinSet<Result<()>>,
    control_rx: &mut Option<mpsc::Receiver<Control>>,
    ramp_target: Option<usize>,
) -> Result<()> {
    // Auto-ramp state. Only active for range-capable fresh downloads that
    // asked for more than one connection; disabled the moment the user
    // drives segmentation manually (they take over).
    let target = ramp_target.unwrap_or(0);
    let mut ramping = shared.ranges_supported && target > 1;
    let mut ramp_timer = tokio::time::interval(RAMP_INTERVAL);
    ramp_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ramp_timer.tick().await; // consume the immediate first tick → first add is delayed

    loop {
        if workers.is_empty() {
            return Ok(());
        }
        tokio::select! {
            joined = workers.join_next() => {
                match joined {
                    Some(Ok(Ok(()))) => continue,
                    Some(Ok(Err(e))) => return Err(e),
                    Some(Err(join_err)) => {
                        return Err(EngineError::other(format!(
                            "worker panicked: {join_err}"
                        )));
                    }
                    None => return Ok(()),
                }
            }
            ctrl = recv_optional(control_rx) => {
                match ctrl {
                    Some(Control::SetSegments { n, ack }) => {
                        ramping = false; // manual override wins
                        let res = apply_set_segments(
                            shared, client, url, cancel, workers, n,
                        ).await;
                        let _ = ack.send(res);
                    }
                    None => {
                        // Control channel closed (or absent); stop polling
                        // it but keep driving workers to completion.
                        *control_rx = None;
                    }
                }
            }
            _ = ramp_timer.tick(), if ramping => {
                if active_worker_count(shared).await >= target
                    || !try_probe_and_split(shared, client, url, cancel, workers).await
                {
                    // Reached the target, or the host refused another
                    // connection (cap) / the probe failed — stop growing
                    // and let the established connections finish.
                    ramping = false;
                }
            }
        }
    }
}

async fn recv_optional(rx: &mut Option<mpsc::Receiver<Control>>) -> Option<Control> {
    match rx.as_mut() {
        Some(r) => r.recv().await,
        None => std::future::pending().await,
    }
}

fn spawn_worker(
    workers: &mut JoinSet<Result<()>>,
    client: Client,
    url: Url,
    worker_index: usize,
    rx: mpsc::Receiver<Segment>,
    shared: Arc<SharedState>,
    cancel: CancellationToken,
) {
    workers.spawn(async move { worker(client, url, worker_index, rx, shared, cancel).await });
}

async fn apply_set_segments(
    shared: &Arc<SharedState>,
    client: &Client,
    url: &Url,
    cancel: &CancellationToken,
    workers: &mut JoinSet<Result<()>>,
    new_n: usize,
) -> Result<()> {
    if !(MIN_SEGMENTS..=MAX_SEGMENTS).contains(&new_n) {
        return Err(EngineError::other(format!(
            "segments out of bounds (got {new_n}, allowed {MIN_SEGMENTS}..={MAX_SEGMENTS})"
        )));
    }
    if !shared.ranges_supported {
        return Err(EngineError::other("download is not resumable"));
    }
    let already_complete = shared.meta.lock().await.is_complete();
    if already_complete {
        return Ok(());
    }

    let current_n = active_worker_count(shared).await;
    if new_n == current_n {
        return Ok(());
    }

    if new_n > current_n {
        let to_add = new_n - current_n;
        for _ in 0..to_add {
            if !try_split_one(shared, client, url, cancel, workers).await? {
                break;
            }
        }
    } else {
        let to_drop = current_n - new_n;
        join_leavers(shared, to_drop).await;
    }
    Ok(())
}

async fn active_worker_count(shared: &Arc<SharedState>) -> usize {
    let senders = shared.senders.lock().await;
    senders.iter().filter(|s| s.is_some()).count()
}

async fn try_split_one(
    shared: &Arc<SharedState>,
    client: &Client,
    url: &Url,
    cancel: &CancellationToken,
    workers: &mut JoinSet<Result<()>>,
) -> Result<bool> {
    let (new_index, new_segment) = {
        let mut m = shared.meta.lock().await;
        let senders = shared.senders.lock().await;
        let Some(donor_idx) = pick_donor(&m, &senders) else {
            return Ok(false);
        };
        drop(senders);

        let donor_state = &mut m.segments[donor_idx];
        let start = donor_state.segment.start;
        let original_end = donor_state.segment.end;
        let bd = donor_state.bytes_downloaded;
        let remaining = original_end.saturating_sub(start + bd);
        if remaining < 2 {
            return Ok(false);
        }
        let midpoint = start + bd + remaining / 2;
        donor_state.segment.end = midpoint;

        let new_index = m.segments.len();
        let new_segment = Segment {
            index: new_index,
            start: midpoint,
            end: original_end,
        };
        m.segments.push(SegmentState {
            segment: new_segment,
            bytes_downloaded: 0,
        });
        (new_index, new_segment)
    };

    let (tx_seg, rx_seg) = mpsc::channel::<Segment>(16);
    if tx_seg.send(new_segment).await.is_err() {
        return Err(EngineError::other("failed to push new segment"));
    }
    {
        let mut senders = shared.senders.lock().await;
        while senders.len() < new_index {
            senders.push(None);
        }
        if senders.len() == new_index {
            senders.push(Some(tx_seg));
        } else {
            senders[new_index] = Some(tx_seg);
        }
    }

    spawn_worker(
        workers,
        client.clone(),
        url.clone(),
        new_index,
        rx_seg,
        shared.clone(),
        cancel.clone(),
    );
    Ok(true)
}

/// Confirm-before-commit split, used by the auto-ramp. Unlike
/// [`try_split_one`] (which shrinks the donor *then* lets the new worker
/// fetch), this opens the new connection FIRST and only commits the split
/// once the server answers `206`. So a connection the host refuses (its
/// concurrency cap) costs nothing — no range is orphaned and the donor
/// keeps going untouched. Returns `true` when a connection was added,
/// `false` when the ramp should stop (no splittable donor, refusal, or a
/// probe error — all non-fatal; the established connections finish).
async fn try_probe_and_split(
    shared: &Arc<SharedState>,
    client: &Client,
    url: &Url,
    cancel: &CancellationToken,
    workers: &mut JoinSet<Result<()>>,
) -> bool {
    // Pick a donor and a split point WITHOUT mutating the plan yet.
    let (donor_idx, midpoint, original_end, validator) = {
        let m = shared.meta.lock().await;
        let senders = shared.senders.lock().await;
        let Some(donor_idx) = pick_donor(&m, &senders) else {
            return false;
        };
        let s = &m.segments[donor_idx];
        let start = s.segment.start;
        let original_end = s.segment.end;
        let bd = s.bytes_downloaded;
        let remaining = original_end.saturating_sub(start + bd);
        if remaining < 2 {
            return false;
        }
        let midpoint = start + bd + remaining / 2;
        let validator = m.etag.clone().or_else(|| m.last_modified.clone());
        (donor_idx, midpoint, original_end, validator)
    };

    // Open the tail range and confirm 206 before touching the plan.
    let mut req = client
        .get(url.clone())
        .header(RANGE, format!("bytes={}-{}", midpoint, original_end - 1));
    if let Some(v) = validator.as_deref() {
        if let Ok(value) = reqwest::header::HeaderValue::from_str(v) {
            req = req.header(IF_RANGE, value);
        }
    }
    let resp = tokio::select! {
        _ = cancel.cancelled() => return false,
        r = req.send() => match r {
            Ok(r) => r,
            Err(_) => return false, // probe failed → stop ramping (non-fatal)
        },
    };
    if resp.status() != StatusCode::PARTIAL_CONTENT {
        // 403/429/… (cap reached) or 200 (range ignored) → stop ramping.
        return false;
    }

    // Confirmed. Commit: shrink the donor (the donor worker may have read
    // past `midpoint` while we probed — the new worker simply rewrites that
    // overlap with identical bytes, as live-split already does), register
    // the new segment, hand it the open 206 body, and spawn its worker.
    let new_index = {
        let mut m = shared.meta.lock().await;
        m.segments[donor_idx].segment.end = midpoint;
        let new_index = m.segments.len();
        m.segments.push(SegmentState {
            segment: Segment {
                index: new_index,
                start: midpoint,
                end: original_end,
            },
            bytes_downloaded: 0,
        });
        new_index
    };
    shared.prefetched.lock().await.insert(new_index, resp);

    let (tx_seg, rx_seg) = mpsc::channel::<Segment>(16);
    let new_segment = Segment {
        index: new_index,
        start: midpoint,
        end: original_end,
    };
    if tx_seg.send(new_segment).await.is_err() {
        shared.prefetched.lock().await.remove(&new_index);
        return false;
    }
    {
        let mut senders = shared.senders.lock().await;
        while senders.len() < new_index {
            senders.push(None);
        }
        if senders.len() == new_index {
            senders.push(Some(tx_seg));
        } else {
            senders[new_index] = Some(tx_seg);
        }
    }
    spawn_worker(
        workers,
        client.clone(),
        url.clone(),
        new_index,
        rx_seg,
        shared.clone(),
        cancel.clone(),
    );
    true
}

fn pick_donor(meta: &Meta, senders: &[Option<mpsc::Sender<Segment>>]) -> Option<usize> {
    let mut best: Option<(usize, u64)> = None;
    for (i, sender) in senders.iter().enumerate() {
        if sender.is_none() {
            continue;
        }
        if i >= meta.segments.len() {
            continue;
        }
        let s = &meta.segments[i];
        let remaining = s
            .segment
            .end
            .saturating_sub(s.segment.start + s.bytes_downloaded);
        if remaining < 2 {
            continue;
        }
        match best {
            None => best = Some((i, remaining)),
            Some((_, br)) if remaining > br => best = Some((i, remaining)),
            _ => {}
        }
    }
    best.map(|(i, _)| i)
}

async fn join_leavers(shared: &Arc<SharedState>, to_drop: usize) {
    let m = shared.meta.lock().await;
    let mut senders = shared.senders.lock().await;
    let mut active: Vec<(usize, u64)> = senders
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            if s.is_some() && i < m.segments.len() {
                let seg = &m.segments[i];
                let remaining = seg
                    .segment
                    .end
                    .saturating_sub(seg.segment.start + seg.bytes_downloaded);
                Some((i, remaining))
            } else {
                None
            }
        })
        .collect();
    active.sort_by_key(|(_, r)| *r);
    drop(m);

    for (i, _) in active.into_iter().take(to_drop) {
        senders[i] = None;
    }
}

pub(crate) struct SharedState {
    pub(crate) meta: Mutex<Meta>,
    pub(crate) meta_path: PathBuf,
    pub(crate) tx: Option<broadcast::Sender<ProgressEvent>>,
    pub(crate) backoff: Backoff,
    pub(crate) ranges_supported: bool,
    pub(crate) senders: Mutex<Vec<Option<mpsc::Sender<Segment>>>>,
    /// Already-open response bodies, keyed by segment index, handed to a
    /// worker so it streams from a request the engine already opened
    /// instead of issuing another GET. Two producers: index 0 for the
    /// single-stream initial no-range GET (so a one-time-token host is
    /// fetched in exactly one request, like a browser), and ramp indices
    /// for the confirmed `206` from a probe-split (see
    /// [`try_probe_and_split`]). Each entry is taken once, on that worker's
    /// first attempt (`already == 0`); a retry finds it gone and opens a
    /// fresh request.
    pub(crate) prefetched: Mutex<std::collections::HashMap<usize, reqwest::Response>>,
}

/// Per-worker state owned by the ticker. The classification rules need
/// memory between ticks (consecutive slow / zero counters), so each
/// worker carries its own sampler.
struct SegmentSampler {
    meter: SpeedMeter,
    prev_state: SegmentRuntimeState,
    consecutive_slow_ticks: u32,
    consecutive_zero_ticks: u32,
}

impl SegmentSampler {
    fn new() -> Self {
        Self {
            meter: SpeedMeter::new(TICK_INTERVAL, SPEED_ALPHA),
            prev_state: SegmentRuntimeState::Active,
            consecutive_slow_ticks: 0,
            consecutive_zero_ticks: 0,
        }
    }

    fn classify(&mut self, speed_bps: f64, median: f64, is_done: bool) -> SegmentRuntimeState {
        if is_done {
            self.consecutive_slow_ticks = 0;
            self.consecutive_zero_ticks = 0;
            self.prev_state = SegmentRuntimeState::Done;
            return SegmentRuntimeState::Done;
        }
        let next = if speed_bps == 0.0 {
            self.consecutive_slow_ticks = 0;
            self.consecutive_zero_ticks = self.consecutive_zero_ticks.saturating_add(1);
            let zero_ms =
                u64::from(self.consecutive_zero_ticks) * (TICK_INTERVAL.as_millis() as u64);
            if zero_ms >= STALLED_AFTER.as_millis() as u64 {
                SegmentRuntimeState::Stalled
            } else {
                // Don't flip away from Slow on a single zero-bps tick;
                // hold the previous verdict until the stall threshold.
                self.prev_state
            }
        } else {
            self.consecutive_zero_ticks = 0;
            if median > 0.0 && speed_bps < median * 0.5 {
                self.consecutive_slow_ticks = self.consecutive_slow_ticks.saturating_add(1);
                if self.consecutive_slow_ticks >= SLOW_TICK_THRESHOLD {
                    SegmentRuntimeState::Slow
                } else {
                    SegmentRuntimeState::Active
                }
            } else {
                self.consecutive_slow_ticks = 0;
                SegmentRuntimeState::Active
            }
        };
        self.prev_state = next;
        next
    }
}

fn median_of_active(speeds: &[f64], done: &[bool]) -> f64 {
    let mut active: Vec<f64> = speeds
        .iter()
        .zip(done.iter())
        .filter_map(|(s, d)| if *d { None } else { Some(*s) })
        .collect();
    if active.is_empty() {
        return 0.0;
    }
    active.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = active.len();
    if n % 2 == 1 {
        active[n / 2]
    } else {
        (active[n / 2 - 1] + active[n / 2]) / 2.0
    }
}

async fn ticker_loop(shared: Arc<SharedState>, total: Option<u64>, cancel: CancellationToken) {
    let mut global = SpeedMeter::new(TICK_INTERVAL, SPEED_ALPHA);
    let mut samplers: Vec<SegmentSampler> = Vec::new();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = sleep(TICK_INTERVAL) => {}
        }

        let snapshot: Vec<(u64, u64)> = {
            let m = shared.meta.lock().await;
            let _ = m.save(&shared.meta_path).await;
            m.segments
                .iter()
                .map(|s| (s.bytes_downloaded, s.segment.len()))
                .collect()
        };
        let segment_count = snapshot.len();
        while samplers.len() < segment_count {
            samplers.push(SegmentSampler::new());
        }

        let mut speeds: Vec<f64> = Vec::with_capacity(segment_count);
        let mut done_flags: Vec<bool> = Vec::with_capacity(segment_count);
        for i in 0..segment_count {
            let (bd, total_seg) = snapshot[i];
            let is_done = total_seg > 0 && bd >= total_seg;
            done_flags.push(is_done);
            if is_done {
                speeds.push(0.0);
            } else {
                let speed = samplers[i].meter.sample(bd).unwrap_or(0.0);
                speeds.push(speed);
            }
        }

        let median = median_of_active(&speeds, &done_flags);

        for i in 0..segment_count {
            let (bd, total_seg) = snapshot[i];
            // For an unknown-length single-stream download the placeholder
            // segment carries `len() == 0` until the body closes, so
            // reporting 0 as the total while bytes stream in would violate
            // the engine's `bytes_downloaded <= segment_total` invariant.
            // Report the bytes seen so far as the provisional total; the
            // final emit carries the true length once `segment.end` is
            // promoted to the actual byte count.
            let segment_total = if total_seg == 0 { bd } else { total_seg };
            let state = samplers[i].classify(speeds[i], median, done_flags[i]);
            emit(
                shared.tx.as_ref(),
                ProgressEvent::SegmentProgress {
                    index: i,
                    bytes_downloaded: bd,
                    segment_total,
                    speed_bps: speeds[i],
                    state,
                },
            );
        }

        let downloaded: u64 = snapshot.iter().map(|(bd, _)| *bd).sum();
        if let Some(speed) = global.sample(downloaded) {
            let remaining = total.map(|t| t.saturating_sub(downloaded)).unwrap_or(0);
            let eta_dur = total.and_then(|_| eta(remaining, speed));
            emit(
                shared.tx.as_ref(),
                ProgressEvent::Tick {
                    downloaded,
                    total,
                    speed_bps: speed,
                    eta: eta_dur,
                },
            );
        }
    }
}

async fn worker(
    client: Client,
    url: Url,
    worker_index: usize,
    mut rx: mpsc::Receiver<Segment>,
    shared: Arc<SharedState>,
    cancel: CancellationToken,
) -> Result<()> {
    while let Some(segment) = rx.recv().await {
        if cancel.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        if let Err(e) = process_segment(&client, &url, segment, &shared, &cancel).await {
            tracing::debug!(worker = worker_index, segment = segment.index, error = %e,
                "worker: segment failed");
            return Err(e);
        }

        // The worker's sender lives in `shared.senders` so the supervisor
        // can route split-added segments to us via push, or close us for
        // a graceful join. Without an explicit signal, we'd block on
        // `rx.recv()` forever after our last assigned segment. After
        // every successful segment, check if the global transfer is
        // done; if so, drop all senders so every peer worker can wake
        // and exit.
        let done = shared.meta.lock().await.is_complete();
        if done {
            let mut senders = shared.senders.lock().await;
            for s in senders.iter_mut() {
                *s = None;
            }
        }
    }
    Ok(())
}

/// Drive one segment to completion, retrying transient errors per
/// [`Backoff`]. Caller is responsible for cancellation propagation.
async fn process_segment(
    client: &Client,
    url: &Url,
    segment: Segment,
    shared: &Arc<SharedState>,
    cancel: &CancellationToken,
) -> Result<()> {
    let backoff = shared.backoff;
    let mut attempt: u32 = 0;
    loop {
        if cancel.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        match try_segment(client, url, segment, shared, cancel).await {
            Ok(()) => return Ok(()),
            Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
            Err(err) => {
                let class = match &err {
                    EngineError::TransientStatus { .. } => RetryClass::Transient,
                    EngineError::TerminalStatus { .. } => RetryClass::Terminal,
                    EngineError::Http(reqwest_err) => classify_reqwest(reqwest_err),
                    EngineError::Io { .. } => RetryClass::Transient,
                    EngineError::BodyTruncated { .. } => RetryClass::Transient,
                    _ => RetryClass::Terminal,
                };
                if class == RetryClass::Terminal {
                    return Err(err);
                }
                attempt += 1;
                if backoff.is_exhausted(attempt) {
                    return Err(EngineError::RetryExhausted {
                        last: err.to_string(),
                    });
                }
                let delay = backoff.delay_for(attempt);
                tracing::warn!(
                    segment = segment.index,
                    attempt,
                    ?delay,
                    error = %err,
                    "transient segment failure; backing off",
                );
                tokio::select! {
                    _ = cancel.cancelled() => return Err(EngineError::Cancelled),
                    _ = sleep(delay) => {}
                }
            }
        }
    }
}

/// Run one attempt at a single segment, starting from its current
/// `bytes_downloaded` high-water mark. Reads `segment.start` and the
/// (possibly live-truncated) `segment.end` from `meta` so a retry after
/// a split honors the new end. Per-write `SegmentProgress` emission is
/// owned by the ticker.
async fn try_segment(
    client: &Client,
    url: &Url,
    segment: Segment,
    shared: &Arc<SharedState>,
    cancel: &CancellationToken,
) -> Result<()> {
    let index = segment.index;
    let (already, segment_start, current_end, output_path, validator) = {
        let m = shared.meta.lock().await;
        let s = &m.segments[index];
        // Only short-circuit if we know the total size up front; for
        // single-stream + unknown-length, `len() == 0` is a placeholder
        // for "haven't started yet," not "done."
        if m.total_bytes.is_some() && s.is_complete() {
            return Ok(());
        }
        // Validator to pin the range to (prefer the strong ETag, fall back
        // to Last-Modified). Used as `If-Range` so a changed resource is
        // detected per-request rather than only at resume start.
        let validator = m.etag.clone().or_else(|| m.last_modified.clone());
        (
            s.bytes_downloaded,
            s.segment.start,
            s.segment.end,
            m.output_path.clone(),
            validator,
        )
    };

    // Live-truncation: if split already pushed our end down to (or
    // past) our high-water mark, there's nothing left to fetch.
    if current_end <= segment_start + already {
        return Ok(());
    }

    let absolute_offset = segment_start + already;
    let mut req = client.get(url.clone());
    if shared.ranges_supported {
        let header = format!("bytes={}-{}", absolute_offset, current_end - 1);
        req = req.header(RANGE, header);
        // Pin the range to the validator we are resuming against. If the
        // remote changed, an honoring server replies `200` (full body)
        // instead of `206`; we catch that below and surface RemoteChanged
        // so the file is never stitched from old + new bytes. Skip a
        // validator that can't be a valid header value (defensive).
        if let Some(v) = validator.as_deref() {
            if let Ok(value) = reqwest::header::HeaderValue::from_str(v) {
                req = req.header(IF_RANGE, value);
            }
        }
    } else if already > 0 {
        // Single-stream and partial progress — we can't seek into the body.
        // Caller will see this and decide to restart from scratch.
        return Err(EngineError::other(
            "cannot resume single-stream download mid-flight",
        ));
    }

    // First attempt for this segment: if the engine already opened a
    // response for it (single-stream initial GET, or a confirmed ramp
    // probe), stream from that instead of issuing another request. A retry
    // finds the slot empty and opens a fresh request.
    let resp = if already == 0 {
        match shared.prefetched.lock().await.remove(&index) {
            Some(prefetched) => prefetched,
            None => req.send().await?,
        }
    } else {
        req.send().await?
    };
    let status = resp.status();
    if shared.ranges_supported {
        // A `200` on a ranged request means the server dropped the Range —
        // because the resource changed (our `If-Range` no longer matches)
        // or it never supported ranges after all. It will never become a
        // `206`, so map it straight to RemoteChanged instead of routing it
        // through `map_status_error`, where `200` falls into the transient
        // bucket and burns the whole retry budget before failing.
        if status == StatusCode::OK {
            return Err(EngineError::RemoteChanged);
        }
        if status != StatusCode::PARTIAL_CONTENT {
            return Err(crate::http::map_status_error(status.as_u16()));
        }
    } else if !status.is_success() {
        return Err(crate::http::map_status_error(status.as_u16()));
    }

    let mut file = open_for_segment(&output_path, absolute_offset).await?;
    consume_into_file(
        resp,
        &mut file,
        shared,
        index,
        absolute_offset,
        segment_start,
        cancel,
    )
    .await?;

    // The body stream may end before the full segment range arrives (server
    // closed the connection mid-flight). Surface that as a transient error
    // so the retry loop in `worker` picks up where we left off — but only
    // if we did NOT exit early due to live truncation.
    if shared.ranges_supported {
        let (live_end, segment_start_now, actual) = {
            let m = shared.meta.lock().await;
            let s = &m.segments[index];
            (s.segment.end, s.segment.start, s.bytes_downloaded)
        };
        let expected = live_end.saturating_sub(segment_start_now);
        if actual < expected {
            return Err(EngineError::BodyTruncated { expected, actual });
        }
    } else {
        // Single-stream + unknown length: the stream ending IS completion.
        // Promote the actual byte count into segment.end so subsequent
        // bookkeeping (is_complete, downloaded_total) reads correctly.
        let mut m = shared.meta.lock().await;
        if m.total_bytes.is_none() {
            let actual = m.segments[index].bytes_downloaded;
            m.segments[index].segment.end = actual;
            m.total_bytes = Some(actual);
        }
    }
    Ok(())
}

async fn open_for_segment(path: &Path, absolute_offset: u64) -> Result<File> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .await
        .map_err(|e| EngineError::io(Some(path.to_path_buf()), e))?;
    file.seek(std::io::SeekFrom::Start(absolute_offset))
        .await
        .map_err(|e| EngineError::io(Some(path.to_path_buf()), e))?;
    Ok(file)
}

async fn consume_into_file(
    resp: reqwest::Response,
    file: &mut File,
    shared: &Arc<SharedState>,
    index: usize,
    absolute_offset: u64,
    segment_start: u64,
    cancel: &CancellationToken,
) -> Result<()> {
    let mut stream = resp.bytes_stream();
    let mut written_since_flush: u64 = 0;
    let mut written_total: u64 = 0;

    loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => return Err(EngineError::Cancelled),
            c = stream.next() => c,
        };
        let chunk = match next {
            Some(c) => c?,
            None => break,
        };
        if chunk.is_empty() {
            continue;
        }
        // If a misbehaving server hands us more bytes than the segment
        // covers, truncate before writing so we never trample the next
        // segment's region. Use the LIVE end so a split-shrunk range
        // doesn't get overwritten.
        let live_end = {
            let m = shared.meta.lock().await;
            m.segments[index].segment.end
        };
        let segment_len = live_end.saturating_sub(segment_start);
        let max_write = if segment_len > 0 {
            segment_len.saturating_sub((absolute_offset - segment_start) + written_total)
        } else {
            chunk.len() as u64
        };
        if max_write == 0 {
            break;
        }
        let slice = if (chunk.len() as u64) > max_write {
            &chunk[..max_write as usize]
        } else {
            &chunk[..]
        };
        file.write_all(slice)
            .await
            .map_err(|e| EngineError::io(None, e))?;
        written_since_flush += slice.len() as u64;
        written_total += slice.len() as u64;

        // Update the segment's high-water mark. The ticker reads this
        // every 250 ms and is the only place SegmentProgress is emitted.
        let truncated = {
            let mut m = shared.meta.lock().await;
            let s = &mut m.segments[index];
            s.bytes_downloaded = (absolute_offset + written_total) - s.segment.start;
            s.segment.end <= s.segment.start + s.bytes_downloaded
        };
        if truncated {
            break;
        }

        if segment_len > 0 && (absolute_offset - segment_start) + written_total >= segment_len {
            break;
        }

        if written_since_flush >= WRITE_CHUNK_FLUSH_BYTES {
            file.flush().await.map_err(|e| EngineError::io(None, e))?;
            written_since_flush = 0;
        }
    }
    file.flush().await.map_err(|e| EngineError::io(None, e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_of_active_skips_done_workers() {
        let speeds = vec![100.0, 200.0, 300.0, 0.0];
        let done = vec![false, false, false, true];
        assert!((median_of_active(&speeds, &done) - 200.0).abs() < 1e-9);
    }

    #[test]
    fn median_of_active_empty_returns_zero() {
        assert_eq!(median_of_active(&[], &[]), 0.0);
        assert_eq!(median_of_active(&[1.0, 2.0], &[true, true]), 0.0);
    }

    #[test]
    fn median_of_active_even_count_averages() {
        let speeds = vec![10.0, 20.0, 30.0, 40.0];
        let done = vec![false, false, false, false];
        assert!((median_of_active(&speeds, &done) - 25.0).abs() < 1e-9);
    }

    #[test]
    fn sampler_emits_done_for_completed_worker() {
        let mut s = SegmentSampler::new();
        let state = s.classify(0.0, 0.0, true);
        assert_eq!(state, SegmentRuntimeState::Done);
    }

    #[test]
    fn sampler_marks_stalled_after_threshold() {
        let mut s = SegmentSampler::new();
        let ticks_to_stall =
            (STALLED_AFTER.as_millis() as u64 / TICK_INTERVAL.as_millis() as u64) as u32;
        for _ in 0..(ticks_to_stall - 1) {
            let st = s.classify(0.0, 100.0, false);
            assert_ne!(st, SegmentRuntimeState::Stalled);
        }
        let st = s.classify(0.0, 100.0, false);
        assert_eq!(st, SegmentRuntimeState::Stalled);
    }

    #[test]
    fn sampler_marks_slow_after_two_consecutive_below_threshold_ticks() {
        let mut s = SegmentSampler::new();
        let st = s.classify(10.0, 100.0, false);
        assert_eq!(st, SegmentRuntimeState::Active);
        let st = s.classify(10.0, 100.0, false);
        assert_eq!(st, SegmentRuntimeState::Slow);
    }

    #[test]
    fn sampler_resets_slow_counter_on_recovery() {
        let mut s = SegmentSampler::new();
        let _ = s.classify(10.0, 100.0, false);
        let _ = s.classify(60.0, 100.0, false);
        let st = s.classify(10.0, 100.0, false);
        assert_eq!(st, SegmentRuntimeState::Active);
    }

    fn make_meta(ranges: Vec<(u64, u64, u64)>) -> Meta {
        let segs: Vec<Segment> = ranges
            .iter()
            .enumerate()
            .map(|(i, (s, e, _))| Segment {
                index: i,
                start: *s,
                end: *e,
            })
            .collect();
        let mut m = Meta::new(
            "https://example.com/x",
            std::path::PathBuf::from("/tmp/x"),
            Some(ranges.iter().map(|(_, e, _)| *e).max().unwrap_or(0)),
            None,
            None,
            true,
            segs,
        );
        for (i, (_, _, bd)) in ranges.iter().enumerate() {
            m.segments[i].bytes_downloaded = *bd;
        }
        m
    }

    fn dummy_senders(active: &[bool]) -> Vec<Option<mpsc::Sender<Segment>>> {
        active
            .iter()
            .map(|a| {
                if *a {
                    let (tx, _rx) = mpsc::channel::<Segment>(1);
                    Some(tx)
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn pick_donor_returns_worker_with_most_remaining() {
        // index 0: 100 remaining, index 1: 500 remaining, index 2: closed.
        let m = make_meta(vec![(0, 100, 0), (100, 1000, 500), (1000, 2000, 0)]);
        let senders = dummy_senders(&[true, true, false]);
        assert_eq!(pick_donor(&m, &senders), Some(1));
    }

    #[test]
    fn pick_donor_skips_workers_with_under_two_bytes() {
        let m = make_meta(vec![(0, 1, 0), (1, 100, 99)]);
        let senders = dummy_senders(&[true, true]);
        // index 0 has remaining=1, index 1 has remaining=1; both skipped.
        assert_eq!(pick_donor(&m, &senders), None);
    }

    #[test]
    fn pick_donor_none_when_all_closed() {
        let m = make_meta(vec![(0, 1000, 0), (1000, 2000, 0)]);
        let senders = dummy_senders(&[false, false]);
        assert_eq!(pick_donor(&m, &senders), None);
    }
}
