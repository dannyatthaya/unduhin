//! Typed events emitted on the core broadcast channel.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use engine::SegmentRuntimeState;

use crate::download::{DownloadId, DownloadRecord, Status};
use crate::tooling::Tool;

/// One event on the core event bus.
///
/// `Box<DownloadRecord>` is used for [`CoreEvent::DownloadAdded`] to keep
/// the enum's stack size reasonable; broadcast channels clone events for
/// every subscriber and `DownloadRecord` is on the larger side.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoreEvent {
    DownloadAdded {
        id: DownloadId,
        snapshot: Box<DownloadRecord>,
    },
    StatusChanged {
        id: DownloadId,
        from: Status,
        to: Status,
    },
    ProgressUpdate {
        id: DownloadId,
        downloaded: u64,
        total: Option<u64>,
        #[serde(rename = "speed_bps")]
        speed_bps: f64,
        #[serde(with = "duration_opt")]
        eta: Option<Duration>,
    },
    /// Per-segment heartbeat for a download. Field names follow the
    /// locked-design wire shape — note the rename vs. the engine
    /// variant: `bytes` (not `bytes_downloaded`), `total` (not
    /// `segment_total`). The translation lives in `queue.rs`.
    SegmentProgress {
        id: DownloadId,
        index: usize,
        bytes: u64,
        total: u64,
        #[serde(rename = "speed_bps")]
        speed_bps: f64,
        state: SegmentRuntimeState,
    },
    Completed {
        id: DownloadId,
        bytes: u64,
    },
    Failed {
        id: DownloadId,
        error: String,
    },
    /// Live swarm snapshot for a torrent download. Translated from the
    /// engine's [`engine::ProgressEvent::SwarmProgress`] in `queue.rs`; the
    /// pump also persists the snapshot into the row's `torrent` JSON so the
    /// peers/seeds survive a relaunch. `up_bps` / `down_bps` are bytes/sec;
    /// `ratio_milli` is the upload/download ratio in thousandths.
    SwarmProgress {
        id: DownloadId,
        peers: u32,
        seeds: u32,
        up_bps: u64,
        down_bps: u64,
        ratio_milli: u32,
    },
    /// Per-file progress for a multi-file torrent. Re-emitted from the
    /// engine's [`engine::ProgressEvent::FileProgress`] (renamed so the wire
    /// shape reads `torrent_file_progress` rather than colliding with the
    /// HTTP segment vocabulary). Not persisted — the detail pane keeps it in
    /// memory like `SegmentProgress`.
    TorrentFileProgress {
        id: DownloadId,
        index: usize,
        downloaded: u64,
        total: u64,
    },
    Removed {
        id: DownloadId,
    },
    CategoryChanged {
        id: DownloadId,
        category_id: Option<i64>,
    },
    /// The user (or the engine) reshaped a download's worker pool to
    /// `n` workers via `set_segments`. The DB's `segments` column tracks
    /// the *logical* intent; live runtime state lives in the engine.
    SegmentsChanged {
        id: DownloadId,
        n: usize,
    },
    /// The on-disk filename and output path of a download row changed
    /// after creation. Fired when yt-dlp learns the real extension /
    /// post-mux filename and updates the DB.
    PathsChanged {
        id: DownloadId,
        filename: String,
        output_path: String,
    },
    SettingChanged {
        key: String,
    },
    /// Fired once when the manager's active worker set drains to zero
    /// after having been non-empty. Debounced 1 s in the queue manager so
    /// a brief gap between two downloads doesn't spuriously fire.
    QueueEmptied,
    /// The in-app named-pipe server bound the well-known
    /// `\\.\pipe\unduhin` path (or the per-test override). Fired once
    /// per app lifetime, immediately after the listener accepts. The
    /// queue manager ignores this; the frontend's
    /// `useBrowserStatus()` composable uses it to refresh the
    /// Settings → Browser status card without a polling loop.
    PipeListening {
        name: String,
    },
    /// New rule-metrics snapshot landed via the pipe (`Inbound::RuleMetrics`).
    /// Frontend re-queries via `get_rule_metrics`; the event has
    /// no payload so a slow consumer can't drift behind a deep snapshot
    /// queue.
    RuleMetricsUpdated,
    /// One of the four schedule mutators (`add_schedule`,
    /// `update_schedule`, `remove_schedule`, or an internal `start_at`
    /// reap after a claim) ran. The frontend re-queries via
    /// `list_schedules` — no payload, mirrors `QueueEmptied`.
    SchedulesChanged,
    ToolInstallProgress {
        tool: Tool,
        downloaded: u64,
        total: Option<u64>,
    },
    ToolInstallCompleted {
        tool: Tool,
        version: Option<String>,
    },
    ToolInstallFailed {
        tool: Tool,
        error: String,
    },
}

/// Serialize `Option<Duration>` as `Option<f64>` seconds. Tauri's frontend
/// will deserialize this into a plain number, which is easier to format
/// than the Serde-default tagged representation.
mod duration_opt {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        d.map(|d| d.as_secs_f64()).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        Option::<f64>::deserialize(d).map(|opt| opt.map(Duration::from_secs_f64))
    }
}
