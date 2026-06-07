// useDownloadsStore — the heart of the UI.
//
// Three responsibilities:
//   1. Hold the canonical map of downloads, keyed by id, plus the
//      "transient" per-id stats (current speed, ETA) the engine emits.
//      These are stored separately from `DownloadRecord` because they
//      don't live in the database — they're recomputed each tick and
//      we don't want to bloat the record.
//   2. Maintain UI-only derived state the backend doesn't author:
//      a rolling 60s speed sparkline per download (`speedHistory`),
//      and a synthesized timeline of notable events per download
//      (`timeline`). Both are populated from the same CoreEvent stream
//      and live entirely in the renderer.
//   3. Expose `applyEvent(event)` — a pure-ish reducer that maps one
//      CoreEvent onto the *core* state (records + stats). Lifted out
//      as its own export so it can be unit-tested without spinning up
//      Pinia. Speed history and timeline live outside the reducer so
//      its tests stay focused on the data the backend authors.

import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { api } from "@/types/tauri-bindings";
import type {
  CoreEvent,
  DownloadId,
  DownloadRecord,
  SegmentLive,
  Status,
} from "@/types/tauri-bindings";

export interface DownloadStats {
  speed_bps: number;
  eta: number | null;
}

/** One torrent file's live download progress, keyed by file index.
 *  Carried in-memory like `SegmentLive` (the backend re-emits it but does
 *  not persist it — only the swarm snapshot lands in the row's JSON). */
export interface TorrentFileLive {
  index: number;
  downloaded: number;
  total: number;
}

export interface DownloadsState {
  records: Map<DownloadId, DownloadRecord>;
  stats: Map<DownloadId, DownloadStats>;
  /** Engine-authored per-segment telemetry, keyed by download then by
   *  segment index. Updated from `segment_progress` events; never
   *  written by the frontend itself. */
  liveSegments: Map<DownloadId, Map<number, SegmentLive>>;
  /** Torrent per-file progress, keyed by download then by file index.
   *  Updated from `torrent_file_progress` events. In-memory only, like
   *  `liveSegments` — the backend does not persist it. */
  liveTorrentFiles: Map<DownloadId, Map<number, TorrentFileLive>>;
}

export type TimelineKind =
  | "started"
  | "status"
  | "completed"
  | "failed"
  | "retry";

export interface TimelineEntry {
  kind: TimelineKind;
  at: string; // ISO timestamp
  title: string;
  detail?: string;
  meta?: string;
}

const SPEED_HISTORY_LEN = 60;

/** Pure event reducer. Mutates the provided state in place. Lifted so
 *  the Vitest suite can exercise it without Pinia. */
export function applyEvent(state: DownloadsState, event: CoreEvent): void {
  switch (event.type) {
    case "download_added":
      state.records.set(event.id, event.snapshot);
      break;
    case "status_changed": {
      const rec = state.records.get(event.id);
      if (rec) {
        rec.status = event.to;
        // Clear stale stats whenever a transfer stops being mid-flight.
        // Muxing also produces fresh stats (it's a real download of the
        // second stream), so it counts as "in flight" for stats purposes.
        if (event.to !== "active" && event.to !== "muxing") {
          state.stats.delete(event.id);
        }
      }
      break;
    }
    case "progress_update": {
      const rec = state.records.get(event.id);
      if (rec) {
        rec.downloaded_bytes = event.downloaded;
        if (event.total != null) rec.total_bytes = event.total;
      }
      state.stats.set(event.id, {
        speed_bps: event.speed_bps,
        eta: event.eta,
      });
      break;
    }
    case "segment_progress": {
      let map = state.liveSegments.get(event.id);
      if (!map) {
        map = new Map();
        state.liveSegments.set(event.id, map);
      }
      map.set(event.index, {
        index: event.index,
        bytes: event.bytes,
        total: event.total,
        speed_bps: event.speed_bps,
        state: event.state,
      });
      break;
    }
    case "swarm_progress": {
      // Mirror the backend pump: persist the latest swarm snapshot onto the
      // record's `torrent.swarm` so peers/seeds/ratio render live and survive
      // the next `refresh()` (which reloads the persisted blob). Only torrent
      // rows carry `torrent`; ignore the event for non-torrent records.
      const rec = state.records.get(event.id);
      if (rec?.torrent) {
        rec.torrent.swarm = {
          peers: event.peers,
          seeds: event.seeds,
          up_bps: event.up_bps,
          down_bps: event.down_bps,
          ratio_milli: event.ratio_milli,
        };
      }
      break;
    }
    case "torrent_file_progress": {
      let map = state.liveTorrentFiles.get(event.id);
      if (!map) {
        map = new Map();
        state.liveTorrentFiles.set(event.id, map);
      }
      map.set(event.index, {
        index: event.index,
        downloaded: event.downloaded,
        total: event.total,
      });
      break;
    }
    case "completed": {
      const rec = state.records.get(event.id);
      if (rec) {
        rec.status = "completed";
        rec.downloaded_bytes = event.bytes;
        if (rec.total_bytes == null) rec.total_bytes = event.bytes;
        rec.completed_at = new Date().toISOString();
      }
      state.stats.delete(event.id);
      break;
    }
    case "failed": {
      const rec = state.records.get(event.id);
      if (rec) {
        rec.status = "failed";
        rec.error = event.error;
      }
      state.stats.delete(event.id);
      break;
    }
    case "removed":
      state.records.delete(event.id);
      state.stats.delete(event.id);
      state.liveSegments.delete(event.id);
      state.liveTorrentFiles.delete(event.id);
      break;
    case "category_changed": {
      const rec = state.records.get(event.id);
      if (rec) rec.category_id = event.category_id;
      break;
    }
    case "segments_changed": {
      const rec = state.records.get(event.id);
      if (rec) rec.segments = event.n;
      break;
    }
    case "paths_changed": {
      const rec = state.records.get(event.id);
      if (rec) {
        rec.filename = event.filename;
        rec.output_path = event.output_path;
      }
      break;
    }
    case "tool_install_progress":
    case "tool_install_completed":
    case "tool_install_failed":
      // Tooling events are handled by useToolingStatus; ignore here.
      break;
    case "setting_changed":
      // Settings are handled by useSettingsStore; ignore here.
      break;
    case "schedules_changed":
      // Handled by useSchedulesStore — wired in App.vue's event mount so
      // the suppression gate and any open ScheduleDialog see fresh data.
      break;
  }
}

export const useDownloadsStore = defineStore("downloads", () => {
  // Pinia + Maps + reactivity: we wrap the maps in `ref` so accessing
  // `.records` from a template still triggers re-renders.
  const records = ref(new Map<DownloadId, DownloadRecord>());
  const stats = ref(new Map<DownloadId, DownloadStats>());
  const liveSegments = ref(new Map<DownloadId, Map<number, SegmentLive>>());
  const liveTorrentFiles = ref(
    new Map<DownloadId, Map<number, TorrentFileLive>>(),
  );
  const speedHistory = ref(new Map<DownloadId, number[]>());
  const timeline = ref(new Map<DownloadId, TimelineEntry[]>());

  // First-load skeleton flag. Flips to false the first time `refresh`
  // resolves, even if the list is empty.
  const loading = ref(true);

  const all = computed(() => Array.from(records.value.values()));

  const byStatus = (s: Status) =>
    computed(() => all.value.filter((r) => r.status === s));

  const active = byStatus("active");
  const muxing = byStatus("muxing");
  const queued = byStatus("queued");
  const paused = byStatus("paused");
  const failed = byStatus("failed");
  const completed = byStatus("completed");

  const totals = computed(() => ({
    // "active" is exposed to the UI as "anything mid-flight" — that
    // includes the brief Muxing phase after yt-dlp finishes pulling
    // streams, since from the user's POV the download is still working.
    active: active.value.length + muxing.value.length,
    queued: queued.value.length,
    paused: paused.value.length,
    failed: failed.value.length,
    completed: completed.value.length,
    all: all.value.length,
  }));

  const aggregateSpeedBps = computed(() => {
    let s = 0;
    for (const st of stats.value.values()) s += st.speed_bps;
    return s;
  });

  function statsFor(id: DownloadId): DownloadStats | undefined {
    return stats.value.get(id);
  }

  function liveSegmentsFor(id: DownloadId): Map<number, SegmentLive> | undefined {
    return liveSegments.value.get(id);
  }

  function liveTorrentFilesFor(
    id: DownloadId,
  ): Map<number, TorrentFileLive> | undefined {
    return liveTorrentFiles.value.get(id);
  }

  function speedHistoryFor(id: DownloadId): number[] {
    return speedHistory.value.get(id) ?? [];
  }

  function timelineFor(id: DownloadId): TimelineEntry[] {
    return timeline.value.get(id) ?? [];
  }

  function pushTimeline(id: DownloadId, entry: TimelineEntry) {
    const list = timeline.value.get(id) ?? [];
    list.push(entry);
    timeline.value.set(id, list);
  }

  function pushSpeedSample(id: DownloadId, speed: number) {
    const buf = speedHistory.value.get(id) ?? [];
    buf.push(speed);
    while (buf.length > SPEED_HISTORY_LEN) buf.shift();
    speedHistory.value.set(id, buf);
  }

  async function refresh() {
    try {
      const rows = await api.listDownloads();
      const next = new Map<DownloadId, DownloadRecord>();
      for (const r of rows) {
        next.set(r.id, r);
        // Rebuild the sparkline series for rows that finished before this
        // session: `speedHistory` is otherwise populated only by in-session
        // `progress_update` events, so it would be empty after a relaunch.
        // Don't clobber a live series we're already tracking.
        const existing = speedHistory.value.get(r.id);
        if (
          (!existing || existing.length === 0) &&
          r.speed_samples &&
          r.speed_samples.length > 0
        ) {
          speedHistory.value.set(r.id, r.speed_samples.slice(-SPEED_HISTORY_LEN));
        }
      }
      records.value = next;
    } finally {
      loading.value = false;
    }
  }

  function handleEvent(event: CoreEvent) {
    // Synthesize timeline + speed history *before* the reducer so the
    // pre-event status is still readable when we need it (e.g. a
    // failed→active transition that we want to label as "retry").
    const prev = "id" in event ? records.value.get(event.id) : undefined;

    applyEvent(
      {
        records: records.value,
        stats: stats.value,
        liveSegments: liveSegments.value,
        liveTorrentFiles: liveTorrentFiles.value,
      },
      event,
    );

    switch (event.type) {
      case "download_added":
        pushTimeline(event.id, {
          kind: "started",
          at: new Date().toISOString(),
          title: "Download started",
          detail: `${event.snapshot.segments} segment${
            event.snapshot.segments === 1 ? "" : "s"
          } allocated`,
        });
        break;
      case "status_changed":
        if (event.from === "failed" && event.to === "active") {
          pushTimeline(event.id, {
            kind: "retry",
            at: new Date().toISOString(),
            title: "Retry succeeded",
            detail: "Transfer resumed after a previous failure",
          });
        } else if (event.from !== event.to) {
          pushTimeline(event.id, {
            kind: "status",
            at: new Date().toISOString(),
            title: `Status changed to ${event.to}`,
            detail: `Previous: ${event.from}`,
          });
        }
        break;
      case "progress_update":
        pushSpeedSample(event.id, event.speed_bps);
        break;
      case "completed":
        pushTimeline(event.id, {
          kind: "completed",
          at: new Date().toISOString(),
          title: "Download completed",
          detail: prev?.filename,
        });
        break;
      case "failed":
        pushTimeline(event.id, {
          kind: "failed",
          at: new Date().toISOString(),
          title: "Download failed",
          detail: event.error,
        });
        break;
      case "removed":
        speedHistory.value.delete(event.id);
        timeline.value.delete(event.id);
        liveSegments.value.delete(event.id);
        liveTorrentFiles.value.delete(event.id);
        break;
    }

    // Pinia tracks `.value` re-assignment, not internal Map mutation —
    // re-set the refs to trigger reactivity on every event.
    records.value = new Map(records.value);
    stats.value = new Map(stats.value);
    liveSegments.value = new Map(liveSegments.value);
    liveTorrentFiles.value = new Map(liveTorrentFiles.value);
    speedHistory.value = new Map(speedHistory.value);
    timeline.value = new Map(timeline.value);
  }

  async function add(input: Parameters<typeof api.addDownload>[0]) {
    return api.addDownload(input);
  }

  return {
    records,
    stats,
    liveSegments,
    liveTorrentFiles,
    speedHistory,
    timeline,
    loading,
    all,
    active,
    queued,
    paused,
    failed,
    completed,
    totals,
    aggregateSpeedBps,
    statsFor,
    liveSegmentsFor,
    liveTorrentFilesFor,
    speedHistoryFor,
    timelineFor,
    refresh,
    handleEvent,
    add,
    pause: api.pauseDownload,
    resume: api.resumeDownload,
    cancel: api.cancelDownload,
    retry: api.retryDownload,
    /**
     * Direct passthrough to the backend's `remove_download` command.
     * Most UI surfaces should call `useDeleteConfirm().requestDelete(ids)`
     * instead — that path honours the user's `delete_default_action`
     * preference and shows the shared confirmation dialog when needed.
     */
    remove: (id: DownloadId, deleteData = false) =>
      api.removeDownload(id, deleteData),
    setPriority: api.setPriority,
    setSegments: api.setSegments,
    setCategory: api.setCategory,
    pauseAll: api.pauseAll,
    resumeAll: api.resumeAll,
  };
});
