// A download's parts as the details panel shows them: one row per part
// with its byte range, how much is done, its speed and whether it is slow.
// Pure, so the Parts map and table are unit-testable.
//
// Words: a connection is one link to the server; a part is the piece of
// the file it fetches.

import type { SegmentLive, SegmentState } from "@/types/tauri-bindings";

export interface PartRow {
  /** Zero-based index; shown to the user as `index + 1`. */
  index: number;
  /** First byte of the part, or null when the range is not known yet. */
  start: number | null;
  /** One past the last byte of the part. */
  end: number | null;
  /** Bytes fetched so far. */
  done: number;
  /** Whole percent, 0..=100. */
  pct: number;
  /** Current speed in bytes/sec; null when finished or not reporting. */
  speedBps: number | null;
  isDone: boolean;
  /** The engine's verdict: slow or stalled. Never guessed here. */
  isSlow: boolean;
}

const FULL = 100;

function percentOf(done: number, total: number): number {
  if (total <= 0) return 0;
  return Math.min(FULL, Math.round((done / total) * FULL));
}

function isSlowState(live: SegmentLive | undefined): boolean {
  return live?.state === "slow" || live?.state === "stalled";
}

/**
 * Rows from the persisted part ranges, overlaid with the live per-part
 * telemetry when there is any. Before the first range is saved (a download
 * that has just started) there are only `count` placeholder rows carrying
 * whatever the live data says.
 */
export function buildPartRows(
  parts: readonly SegmentState[] | null | undefined,
  live: ReadonlyMap<number, SegmentLive> | undefined,
  count: number,
): PartRow[] {
  if (!parts || parts.length === 0) {
    return Array.from({ length: Math.max(0, count) }, (_, index) => {
      const l = live?.get(index);
      const isDone = l?.state === "done";
      return {
        index,
        start: null,
        end: null,
        done: l?.bytes ?? 0,
        pct: l ? percentOf(l.bytes, l.total) : 0,
        speedBps: isDone ? null : (l?.speed_bps ?? null),
        isDone,
        isSlow: isSlowState(l),
      };
    });
  }
  return parts.map((part) => {
    const { index, start, end } = part.segment;
    const total = Math.max(0, end - start);
    const l = live?.get(index);
    // Live bytes are fresher than the sidecar, which is saved once a second.
    const done = Math.max(part.bytes_downloaded, l?.bytes ?? 0);
    const pct = percentOf(done, total);
    const isDone = l ? l.state === "done" : pct >= FULL;
    return {
      index,
      start,
      end,
      done,
      pct: isDone ? FULL : pct,
      speedBps: isDone ? null : (l?.speed_bps ?? null),
      isDone,
      isSlow: !isDone && isSlowState(l),
    };
  });
}

/** The first slow part, if any. */
export function firstSlowPart(rows: readonly PartRow[]): PartRow | null {
  return rows.find((row) => row.isSlow) ?? null;
}

/** Each part's share of the whole file, as a flex weight for the map. */
export function partWeight(row: PartRow): number {
  if (row.start == null || row.end == null) return 1;
  return Math.max(1, row.end - row.start);
}
