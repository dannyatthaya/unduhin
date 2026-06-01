//! Progress events broadcast to subscribers (CLI, UI, tests).
//!
//! The downloader owns one [`tokio::sync::broadcast::Sender`] and emits
//! events as work happens. Consumers subscribe and consume at their own
//! pace; if a consumer is slow, broadcast will lag them rather than block
//! the downloader.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Default channel capacity. Generous so progress consumers can fall a bit
/// behind without losing events.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// Live verdict for one worker, computed at every ticker sample and
/// shipped with every [`ProgressEvent::SegmentProgress`].
///
/// This is distinct from [`crate::meta::SegmentState`], which is the
/// *persisted* sidecar struct describing the byte range and bytes
/// already written. The two would collide in name if we let them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentRuntimeState {
    /// Mid-flight worker producing bytes at or above the slow threshold.
    Active,
    /// Worker has reached its assigned range end.
    Done,
    /// `speed_bps` < `median(active_speeds) * 0.5` for two consecutive ticks.
    Slow,
    /// `speed_bps == 0` for >= 5 s of continuous ticks.
    Stalled,
}

/// One event in the lifecycle of a download.
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// Fired once at the start of the actual transfer. `total` is `None`
    /// for chunked / unknown-length responses.
    Started {
        total: Option<u64>,
        segments: usize,
        resumed_bytes: u64,
    },
    /// Per-segment heartbeat from the ticker. `bytes_downloaded` is
    /// cumulative for the segment; `speed_bps` is the worker's own
    /// smoothed rate; `state` is the median-relative verdict.
    SegmentProgress {
        index: usize,
        bytes_downloaded: u64,
        segment_total: u64,
        speed_bps: f64,
        state: SegmentRuntimeState,
    },
    /// Periodic snapshot of overall transfer state. Fires roughly every
    /// 250 ms while bytes are flowing.
    Tick {
        downloaded: u64,
        total: Option<u64>,
        speed_bps: f64,
        eta: Option<Duration>,
    },
    /// Final success — `bytes` is the total written.
    Completed { bytes: u64 },
    /// Final failure. Stringified because broadcast requires `Clone`.
    Failed { error: String },
}

/// Helper for building a broadcast channel of `ProgressEvent`.
pub fn channel(capacity: usize) -> broadcast::Sender<ProgressEvent> {
    let (tx, _) = broadcast::channel(capacity);
    tx
}

/// Send-or-drop: progress events are advisory. If no one is subscribed,
/// `send` returns Err and we move on.
pub(crate) fn emit(tx: Option<&broadcast::Sender<ProgressEvent>>, ev: ProgressEvent) {
    if let Some(tx) = tx {
        let _ = tx.send(ev);
    }
}

/// Exponentially-weighted moving average of transfer rate.
///
/// Sampled at `update_interval`; produces a speed estimate that smooths
/// over short bursts while still responding to sustained changes.
#[derive(Debug, Clone)]
pub(crate) struct SpeedMeter {
    alpha: f64,
    speed_bps: Option<f64>,
    last_tick: Instant,
    last_bytes: u64,
    pub update_interval: Duration,
}

impl SpeedMeter {
    pub fn new(update_interval: Duration, alpha: f64) -> Self {
        Self {
            alpha,
            speed_bps: None,
            last_tick: Instant::now(),
            last_bytes: 0,
            update_interval,
        }
    }

    /// Feed total-bytes-downloaded so far. Returns `Some` when a tick
    /// boundary has been crossed since the last sample.
    pub fn sample(&mut self, downloaded: u64) -> Option<f64> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_tick);
        if elapsed < self.update_interval {
            return None;
        }
        let dt = elapsed.as_secs_f64();
        if dt <= 0.0 {
            return None;
        }
        let delta = downloaded.saturating_sub(self.last_bytes) as f64;
        let instant = delta / dt;
        let smoothed = match self.speed_bps {
            None => instant,
            Some(prev) => self.alpha * instant + (1.0 - self.alpha) * prev,
        };
        self.speed_bps = Some(smoothed);
        self.last_tick = now;
        self.last_bytes = downloaded;
        Some(smoothed)
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn current(&self) -> Option<f64> {
        self.speed_bps
    }
}

pub(crate) fn eta(remaining: u64, speed_bps: f64) -> Option<Duration> {
    if !speed_bps.is_finite() || speed_bps <= 0.0 || remaining == 0 {
        return None;
    }
    let secs = remaining as f64 / speed_bps;
    if !secs.is_finite() || secs < 0.0 {
        return None;
    }
    Some(Duration::from_secs_f64(secs.min(u64::MAX as f64 / 2.0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn meter_returns_none_before_first_interval() {
        let mut m = SpeedMeter::new(Duration::from_millis(50), 0.3);
        // Immediately after construction, no tick has elapsed.
        assert!(m.sample(100).is_none());
    }

    #[test]
    fn meter_smooths_over_samples() {
        let mut m = SpeedMeter::new(Duration::from_millis(10), 0.5);
        sleep(Duration::from_millis(15));
        let s1 = m.sample(1000).unwrap();
        assert!(s1 > 0.0);
        sleep(Duration::from_millis(15));
        let s2 = m.sample(2000);
        assert!(s2.is_some());
    }

    #[test]
    fn eta_handles_zero_speed_and_remaining() {
        assert!(eta(100, 0.0).is_none());
        assert!(eta(100, f64::NAN).is_none());
        assert!(eta(0, 1000.0).is_none());
        let d = eta(2000, 1000.0).unwrap();
        assert!((d.as_secs_f64() - 2.0).abs() < 1e-3);
    }
}
