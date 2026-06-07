// Typed contract between the Vue frontend and the `unduhin-app` Tauri
// shell. Mirrors the serde representations in:
//   crates/core/src/{download,category,settings,event}.rs
//   src-tauri/src/commands.rs
// Keep this file in sync by hand. Anything serialized by `unduhin-core`
// uses snake_case keys (the Serde defaults) — that's reflected below.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ---------------------------------------------------------------------------
// Primitive shapes (snake_case to match Serde defaults)
// ---------------------------------------------------------------------------

export type DownloadId = number;
export type CategoryId = number;

export type Status =
  | "queued"
  | "active"
  | "muxing"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled";

export const ALL_STATUSES: Status[] = [
  "queued",
  "active",
  "muxing",
  "paused",
  "completed",
  "failed",
  "cancelled",
];

/** One half-open byte range `[start, end)` owned by a worker. Mirrors
 * `crates/engine/src/segment.rs`. */
export interface Segment {
  index: number;
  start: number;
  end: number;
}

/** Persisted per-segment state from the sidecar. Mirrors the *nested*
 * serde shape of `crates/engine/src/meta.rs::SegmentState` —
 * `{ segment: { index, start, end }, bytes_downloaded }` — NOT a flat
 * record. Reading it as flat yielded `NaN` ranges/percentages. */
export interface SegmentState {
  segment: Segment;
  bytes_downloaded: number;
}

/**
 * Live verdict for one worker, shipped with every `segment_progress`
 * event. Distinct from `SegmentState` above, which mirrors the
 * persisted sidecar row (byte range + bytes already written).
 */
export type SegmentRuntimeState = "active" | "done" | "slow" | "stalled";

/** One in-flight worker's tick snapshot from the engine. */
export interface SegmentLive {
  index: number;
  bytes: number;
  total: number;
  speed_bps: number;
  state: SegmentRuntimeState;
}

// ---------------------------------------------------------------------------
// Torrent shapes — mirror crates/core/src/download.rs
//   (DownloadKind, TorrentMeta, TorrentSource, TorrentFile, SwarmStats)
// ---------------------------------------------------------------------------

/** Which backend runs a download. Mirrors `DownloadKind` (snake_case serde).
 *  `http` → engine, `media` → yt-dlp, `torrent` → librqbit facade. */
export type DownloadKind = "http" | "media" | "torrent";

/** Where a torrent came from. Serde-tagged with `kind` (snake_case) — mirrors
 *  `crates/core/src/download.rs::TorrentSource`. `.torrent` bytes are copied
 *  into the managed dir and referenced by `path`; magnets store the URI. */
export type TorrentSource =
  | { kind: "magnet"; uri: string }
  | { kind: "file"; path: string }
  | { kind: "info_hash"; hash: string };

/** One file inside a torrent — feeds the add-time picker and the detail-pane
 *  per-file progress list. Mirrors `download.rs::TorrentFile`. */
export interface TorrentFile {
  index: number;
  path: string;
  length: number;
  selected: boolean;
}

/** Last swarm snapshot persisted into the row's `torrent` JSON. `up_bps` /
 *  `down_bps` are bytes/sec; `ratio_milli` is the upload/download ratio in
 *  thousandths (1500 = 1.5x). Mirrors `download.rs::SwarmStats`. */
export interface SwarmStats {
  peers: number;
  seeds: number;
  up_bps: number;
  down_bps: number;
  ratio_milli: number;
}

/** Persisted torrent state for a `kind === "torrent"` row. Stored as one
 *  nullable JSON column on `downloads.torrent`. Mirrors
 *  `crates/core/src/download.rs::TorrentMeta`. */
export interface TorrentMeta {
  /** Lowercase hex info-hash — the stable de-dup key. */
  info_hash: string;
  source: TorrentSource;
  /** `null` = download all files; otherwise the selected file indices into
   *  `files` (librqbit `only_files`). */
  selected_files: number[] | null;
  /** Filled once librqbit resolves metadata. */
  files: TorrentFile[] | null;
  /** Last swarm snapshot; survives relaunch so the UI can render peers/seeds
   *  before the session re-attaches. */
  swarm: SwarmStats | null;
}

export interface DownloadRecord {
  id: DownloadId;
  url: string;
  filename: string;
  output_path: string;
  total_bytes: number | null;
  downloaded_bytes: number;
  status: Status;
  error: string | null;
  category_id: CategoryId | null;
  priority: number;
  segments: number;
  created_at: string; // RFC 3339
  completed_at: string | null;
  etag: string | null;
  last_modified: string | null;
  segments_meta: SegmentState[] | null;
  /** Present on yt-dlp-driven rows. Null for plain direct-file downloads. */
  media_info: MediaInfo | null;
  /** Downsampled bytes-per-second series persisted on completion, used to
   * rebuild the detail-pane sparkline for downloads that finished before
   * this session. Null while in flight or for legacy rows. */
  speed_samples: number[] | null;
  /** Which backend runs this download. Defaults to `"http"` on legacy rows
   * (the migration backfills yt-dlp rows to `"media"`). */
  kind: DownloadKind;
  /** Persisted torrent state when `kind === "torrent"`. Null for HTTP/media
   * rows and rows predating the torrent migration. */
  torrent: TorrentMeta | null;
}

// ---------------------------------------------------------------------------
// Media (yt-dlp) shapes — mirror crates/core/src/ytdlp/mod.rs
// ---------------------------------------------------------------------------

export interface ProbeResult {
  url: string;
  extractor: string;
  title: string;
  uploader: string | null;
  duration_secs: number | null;
  thumbnail_url: string | null;
  is_live: boolean;
  age_limit: number | null;
  formats: Format[];
  recommended_video_audio: string | null;
  recommended_audio_only: string | null;
}

export interface Format {
  format_id: string;
  ext: string;
  resolution: string | null;
  fps: number | null;
  vcodec: string | null;
  acodec: string | null;
  filesize_bytes: number | null;
  tbr_kbps: number | null;
  note: string | null;
}

export interface MediaInfo {
  extractor: string;
  format_selector: string;
  title: string;
  original_url: string;
  needs_ffmpeg: boolean;
}

// ---------------------------------------------------------------------------
// Schedules — mirror crates/core/src/schedule.rs
// ---------------------------------------------------------------------------

export type ScheduleId = number;

export type ScheduleKind = "start_at" | "after_queue" | "quiet_hours";

export interface Schedule {
  id: ScheduleId;
  kind: ScheduleKind;
  download_id: DownloadId | null;
  /** RFC3339 for `start_at`; `"HH:MM"` local for `quiet_hours`. */
  start_iso: string | null;
  /** `"HH:MM"` local for `quiet_hours`; null otherwise. */
  end_iso: string | null;
  /** Bit 0 = Mon … bit 6 = Sun. */
  days_mask: number;
  active: boolean;
  created_at: string; // RFC3339
}

export interface NewSchedule {
  kind: ScheduleKind;
  download_id?: DownloadId | null;
  start_iso?: string | null;
  end_iso?: string | null;
  days_mask?: number | null;
  active?: boolean | null;
}

export interface QuietHoursState {
  active: boolean;
  /** RFC3339 when the active window ends; null when not currently active. */
  until: string | null;
}

export type Tool = "yt_dlp" | "ffmpeg";

export interface ToolStatus {
  tool: Tool;
  installed: boolean;
  path: string | null;
  version: string | null;
  latest_known: string | null;
}

export interface Category {
  id: CategoryId;
  name: string;
  icon: string | null;
  default_output_path: string | null;
  extension_rules: string[];
}

export type SettingValue = string | number | boolean | null | Record<string, unknown> | unknown[];

export interface DownloadFilter {
  status?: Status;
  category_id?: CategoryId;
}

// ---------------------------------------------------------------------------
// Event payloads — Serde-tagged with `type` (snake_case)
// ---------------------------------------------------------------------------

export type CoreEvent =
  | {
      type: "download_added";
      id: DownloadId;
      snapshot: DownloadRecord;
    }
  | {
      type: "status_changed";
      id: DownloadId;
      from: Status;
      to: Status;
    }
  | {
      type: "progress_update";
      id: DownloadId;
      downloaded: number;
      total: number | null;
      speed_bps: number;
      eta: number | null; // seconds
    }
  | {
      type: "segment_progress";
      id: DownloadId;
      index: number;
      bytes: number;
      total: number;
      speed_bps: number;
      state: SegmentRuntimeState;
    }
  | {
      type: "completed";
      id: DownloadId;
      bytes: number;
    }
  | {
      type: "failed";
      id: DownloadId;
      error: string;
    }
  | {
      /** Live swarm snapshot for a torrent download. The queue pump also
       *  persists this into the row's `torrent` JSON so peers/seeds survive
       *  a relaunch. `up_bps` / `down_bps` are bytes/sec; `ratio_milli` is
       *  the upload/download ratio in thousandths (1500 = 1.5x). Mirrors
       *  `crates/core/src/event.rs::CoreEvent::SwarmProgress`. */
      type: "swarm_progress";
      id: DownloadId;
      peers: number;
      seeds: number;
      up_bps: number;
      down_bps: number;
      ratio_milli: number;
    }
  | {
      /** Per-file progress for a multi-file torrent. `index` is the file's
       *  position in the torrent's file list; `downloaded` / `total` are
       *  bytes for that one file. Not persisted (in-memory like
       *  `segment_progress`). Mirrors
       *  `crates/core/src/event.rs::CoreEvent::TorrentFileProgress`. */
      type: "torrent_file_progress";
      id: DownloadId;
      index: number;
      downloaded: number;
      total: number;
    }
  | {
      type: "removed";
      id: DownloadId;
    }
  | {
      type: "category_changed";
      id: DownloadId;
      category_id: CategoryId | null;
    }
  | {
      type: "segments_changed";
      id: DownloadId;
      n: number;
    }
  | {
      type: "paths_changed";
      id: DownloadId;
      filename: string;
      output_path: string;
    }
  | {
      type: "setting_changed";
      key: string;
    }
  | {
      /** Fired once when the manager's active worker set drains to zero
       *  after having been non-empty (debounced 1s in the queue). The
       *  `useNotifications` composable surfaces this as the Queue empty
       *  toast when `notify_queue_empty` is on. */
      type: "queue_emptied";
    }
  | {
      /** Fired whenever a schedule row is added/updated/removed, or when
       *  the queue manager reaps a fired `start_at` row. Carries no
       *  payload — the schedules store re-queries via
       *  `api.listSchedules()` on receipt. */
      type: "schedules_changed";
    }
  | {
      /** Fired once per app lifetime, immediately after the in-app
       *  named-pipe server (`\\.\pipe\unduhin`) accepts. The
       *  Settings → Browser status card uses this as a refresh trigger
       *  so it can flip from "starting" to "connected" without a
       *  polling loop. */
      type: "pipe_listening";
      name: string;
    }
  | {
      /** Fired whenever the pipe server caches a fresh `Inbound::RuleMetrics`
       *  snapshot from the extension. The Settings → Browser domain-rules
       *  card uses this as a refresh trigger for `get_rule_metrics`. */
      type: "rule_metrics_updated";
    }
  | {
      type: "tool_install_progress";
      tool: Tool;
      downloaded: number;
      total: number | null;
    }
  | {
      type: "tool_install_completed";
      tool: Tool;
      version: string | null;
    }
  | {
      type: "tool_install_failed";
      tool: Tool;
      error: string;
    };

export const EVENT_CHANNEL = "unduhin:event";

// ---------------------------------------------------------------------------
// Command-input shapes
// ---------------------------------------------------------------------------

export interface AddDownloadInput {
  url: string;
  filename?: string | null;
  output_path?: string | null;
  category_id?: CategoryId | null;
  category_name?: string | null;
  priority?: number | null;
  segments?: number | null;
  media_info?: MediaInfo | null;
  /** Which backend should run the download. Omit (or `"http"`) for direct
   *  files; the media dialog leaves this unset and supplies `media_info`;
   *  the torrent path (P4) sends `"torrent"` plus `torrent`. */
  kind?: DownloadKind | null;
  /** Torrent state when `kind === "torrent"` — the resolved `info_hash`,
   *  source, and the user's file selection from the add-time picker. */
  torrent?: TorrentMeta | null;
}

/** Input to `fetch_torrent_metadata` — probe a torrent's file list WITHOUT
 *  downloading (librqbit `list_only`), backing the add-time file picker.
 *  Mirrors the facade's `TorrentInput` / `TorrentSource` discriminant. P4
 *  wires the Tauri command; this types the surface it will expose. */
export type TorrentMetadataInput = TorrentSource;

/** Result of `fetch_torrent_metadata`. Mirrors the facade's
 *  `crates/torrent::TorrentMetadata` (info_hash + display name + every file
 *  in torrent order). `index` is each file's position in the list, used to
 *  build the `selected_files` selection sent back in `TorrentMeta`. */
export interface TorrentMetadataResult {
  info_hash: string;
  name: string;
  files: TorrentMetadataFile[];
}

export interface TorrentMetadataFile {
  index: number;
  path: string;
  length: number;
}

export interface NewCategoryInput {
  name: string;
  icon?: string | null;
  default_output_path?: string | null;
  extension_rules?: string[] | null;
}

export interface SetSettingInput {
  key: string;
  value: SettingValue;
}

export interface AppInfo {
  version: string;
  name: string;
  git_sha: string;
  build_timestamp: string;
  /** "stable" or "beta". Reflects the persisted setting; live updates
   *  when the user switches channel via Settings → About. */
  channel: string;
  /** e.g. "Windows 11 · x64 · 22631.3593". */
  os: string;
}

export type UpdateCheckStatus = "up_to_date" | "update_available" | "error";

export interface DiskInfo {
  drive: string;
  free_bytes: number;
  total_bytes: number;
}

// ---------------------------------------------------------------------------
// Typed `invoke` wrappers
// ---------------------------------------------------------------------------

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  getDiskInfo: () => invoke<DiskInfo>("get_disk_info"),

  addDownload: (input: AddDownloadInput) =>
    invoke<DownloadId>("add_download", { input }),
  listDownloads: (filter?: DownloadFilter) =>
    invoke<DownloadRecord[]>("list_downloads", { filter: filter ?? null }),
  getDownload: (id: DownloadId) =>
    invoke<DownloadRecord>("get_download", { id }),
  pauseDownload: (id: DownloadId) => invoke<void>("pause_download", { id }),
  resumeDownload: (id: DownloadId) => invoke<void>("resume_download", { id }),
  cancelDownload: (id: DownloadId) => invoke<void>("cancel_download", { id }),
  retryDownload: (id: DownloadId) => invoke<void>("retry_download", { id }),
  removeDownload: (id: DownloadId, deleteData = false) =>
    invoke<void>("remove_download", { id, deleteData }),
  setPriority: (id: DownloadId, priority: number) =>
    invoke<void>("set_priority", { id, priority }),
  /**
   * Reshape a download's worker pool to `n` workers (1..=32). For an
   * actively transferring download, this triggers a live split or
   * graceful join inside the engine; for queued / paused rows it just
   * updates the persisted intent. Throws on non-resumable downloads.
   */
  setSegments: (id: DownloadId, n: number) =>
    invoke<void>("set_segments", { id, n }),
  /**
   * Reassign a download to a different category. Pass `null` to clear
   * the assignment. Emits a `category_changed` event so the sidebar
   * counts and any open detail pane update live.
   */
  setCategory: (id: DownloadId, categoryId: CategoryId | null) =>
    invoke<void>("set_category", { id, categoryId }),
  /**
   * HEAD-probe `url` and return the engine's best-effort derived
   * filename — Content-Disposition wins, then the final-redirect URL
   * path, then a MIME-based fallback, then the user-supplied URL path
   * tail. Returns `null` when nothing usable could be derived; the
   * caller should treat that as "ask the user to type a name".
   */
  previewFilename: (url: string) =>
    invoke<string | null>("preview_filename", { url }),
  pauseAll: () => invoke<number>("pause_all"),
  resumeAll: () => invoke<number>("resume_all"),

  listCategories: () => invoke<Category[]>("list_categories"),
  addCategory: (input: NewCategoryInput) =>
    invoke<CategoryId>("add_category", { input }),
  updateCategory: (id: CategoryId, input: NewCategoryInput) =>
    invoke<void>("update_category", { id, input }),
  removeCategory: (id: CategoryId) =>
    invoke<void>("remove_category", { id }),
  setCategoryOrder: (ids: CategoryId[]) =>
    invoke<void>("set_category_order", { ids }),

  getSettings: () => invoke<Record<string, SettingValue>>("get_settings"),
  getSetting: (key: string) =>
    invoke<SettingValue | null>("get_setting", { key }),
  setSetting: (input: SetSettingInput) =>
    invoke<void>("set_setting", { input }),

  /**
   * Returns `null` when yt-dlp doesn't recognize the URL (or the probe
   * times out) — the caller should then fall through to the regular
   * `addDownload` path so the engine handles it as a direct file.
   * Returns the rich result when yt-dlp recognized the URL.
   * Throws when yt-dlp is not installed or hit a fatal error (DRM, auth).
   */
  probeMediaUrl: (url: string) =>
    invoke<ProbeResult | null>("probe_media_url", { url }),
  /**
   * Probe a magnet / `.torrent` / infohash for its file list WITHOUT
   * downloading (librqbit `list_only`) — backs the add-time file picker.
   * Returns the resolved `info_hash`, display name, and every file in
   * torrent order. Throws when metadata can't be fetched within the
   * backend timeout (no peers / DHT off) or the input is malformed.
   *
   * The backing Tauri command is wired in P4; this typed wrapper is the
   * frozen surface that the AddUrlDialog torrent path will call.
   */
  fetchTorrentMetadata: (input: TorrentMetadataInput) =>
    invoke<TorrentMetadataResult>("fetch_torrent_metadata", { input }),
  toolStatus: (tool: Tool) => invoke<ToolStatus>("tool_status", { tool }),
  installTool: (tool: Tool) => invoke<ToolStatus>("install_tool", { tool }),

  recordUpdateCheck: (
    status: UpdateCheckStatus,
    availableVersion: string | null = null,
    notes: string | null = null,
  ) =>
    invoke<void>("record_update_check", {
      status,
      availableVersion,
      notes,
    }),
  getLogsDir: () => invoke<string | null>("get_logs_dir"),

  /**
   * Resolve an outstanding `unduhin:confirm-quit` prompt with the user's
   * answer. Calling with a stale `request_id` (already answered or
   * timed-out) is a no-op on the Rust side.
   */
  confirmQuitResponse: (requestId: number, allow: boolean) =>
    invoke<void>("confirm_quit_response", { requestId, allow }),
  /**
   * Exit the app immediately. Bypasses the close-behavior policy —
   * appropriate for the tray's *Quit* item where the user has already
   * committed to leaving.
   */
  quitApp: () => invoke<void>("quit_app"),

  // -- Schedules -----------------------------------------------------------
  listSchedules: () => invoke<Schedule[]>("list_schedules"),
  addSchedule: (input: NewSchedule) =>
    invoke<ScheduleId>("add_schedule", { input }),
  updateSchedule: (id: ScheduleId, input: NewSchedule) =>
    invoke<void>("update_schedule", { id, input }),
  removeSchedule: (id: ScheduleId) =>
    invoke<void>("remove_schedule", { id }),
  /**
   * Snapshot of the global quiet-hours window. `{ active: false, until:
   * null }` when no `quiet_hours` schedule currently covers the user's
   * local clock. Read by `useNotifications` as the suppression gate.
   */
  getQuietHoursState: () => invoke<QuietHoursState>("get_quiet_hours_state"),
};

// ---------------------------------------------------------------------------
// One-off event channels (typed listeners)
// ---------------------------------------------------------------------------

/**
 * Confirmation prompt the Rust close handler emits when
 * `close_behavior = "ask"` or
 * `confirm_on_quit = true + close_behavior = "exit" + inflight downloads`.
 * The frontend renders `ConfirmOnQuitDialog` against this payload and
 * calls `api.confirmQuitResponse(request_id, allow)` to resolve.
 */
export interface ConfirmQuitRequest {
  request_id: number;
  active_count: number;
  /** `true` = framed as "close window?", `false` = framed as "quit with
   *  active downloads?". The branch picks user-facing wording. */
  ask_close: boolean;
}

export const CONFIRM_QUIT_EVENT = "unduhin:confirm-quit";

export function onConfirmQuit(
  handler: (req: ConfirmQuitRequest) => void,
): Promise<UnlistenFn> {
  return listen<ConfirmQuitRequest>(CONFIRM_QUIT_EVENT, (e) =>
    handler(e.payload),
  );
}

export const OPEN_ADD_URL_EVENT = "unduhin:open-add-url";

export function onOpenAddUrl(handler: () => void): Promise<UnlistenFn> {
  return listen(OPEN_ADD_URL_EVENT, () => handler());
}

/** Emitted by the tray's "Check for updates…" item. */
export const CHECK_UPDATES_EVENT = "unduhin:check-updates";

export function onCheckUpdates(handler: () => void): Promise<UnlistenFn> {
  return listen(CHECK_UPDATES_EVENT, () => handler());
}

export function onCoreEvent(
  handler: (event: CoreEvent) => void,
): Promise<UnlistenFn> {
  return listen<CoreEvent>(EVENT_CHANNEL, (e) => handler(e.payload));
}
