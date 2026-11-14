// Builds the sections of the downloads list — Needs attention, Downloading,
// Queued, Cancelled, Done today, Done earlier — and the flat (un-grouped)
// row list for the "Flat" layout. Both shapes share the same filter + sort
// logic so switching layouts doesn't surprise the user with reordering.

import { computed } from "vue";

import { useDownloadsStore } from "@/stores/downloads";
import {
  compareRows,
  type SortColumn,
  type SortDir,
} from "@/composables/useDownloadsSort";
import { calendarDaysAgo } from "@/lib/format";
import type { DownloadRecord } from "@/types/tauri-bindings";

export type DownloadGroupKey =
  | "needs-attention"
  | "active"
  | "queued"
  | "cancelled"
  | "completed"
  | "completed-earlier";

export interface DownloadGroup {
  key: DownloadGroupKey;
  rows: DownloadRecord[];
}

/**
 * Whether a completed row finished on today's local calendar day. Prefers
 * `completed_at` and falls back to `created_at`, which keeps rows whose
 * `completed_at` is NULL (legacy rows) visible instead of silently dropped.
 * If neither parses the row counts as earlier rather than vanishing.
 */
function completedToday(r: DownloadRecord, nowMs: number): boolean {
  const iso = r.completed_at ?? r.created_at;
  const t = iso ? Date.parse(iso) : Number.NaN;
  if (Number.isNaN(t)) return false;
  return calendarDaysAgo(t, nowMs) <= 0;
}

function matchesQuery(r: DownloadRecord, q: string): boolean {
  return (
    r.filename.toLowerCase().includes(q) ||
    r.url.toLowerCase().includes(q) ||
    r.output_path.toLowerCase().includes(q)
  );
}

export function useGroupedDownloads(
  search: () => string,
  categoryId: () => number | null,
  sortColumn?: () => SortColumn,
  sortDir?: () => SortDir,
) {
  const store = useDownloadsStore();

  const matching = computed(() => {
    const q = search().trim().toLowerCase();
    const cid = categoryId();
    return store.all.filter((r: DownloadRecord) => {
      if (cid != null && r.category_id !== cid) return false;
      return !q || matchesQuery(r, q);
    });
  });

  const sortedMatching = computed(() => {
    const rows = [...matching.value];
    if (!sortColumn || !sortDir) return rows;
    const col = sortColumn();
    const dir = sortDir();
    rows.sort((a, b) =>
      compareRows(a, b, col, dir, (id) => store.statsFor(id) ?? null),
    );
    return rows;
  });

  const groups = computed<DownloadGroup[]>(() => {
    const rows = sortedMatching.value;
    const now = Date.now();
    const completed = rows.filter((r) => r.status === "completed");

    // Every row lands in exactly one bucket, so the grouped layout shows
    // the same rows as the flat one. Cancelled rows are terminal but not
    // failures; they get their own section rather than vanishing.
    const buckets: DownloadGroup[] = [
      {
        key: "needs-attention",
        rows: rows.filter((r) => r.status === "failed" || r.status === "paused"),
      },
      {
        key: "active",
        rows: rows.filter((r) => r.status === "active" || r.status === "muxing"),
      },
      { key: "queued", rows: rows.filter((r) => r.status === "queued") },
      { key: "cancelled", rows: rows.filter((r) => r.status === "cancelled") },
      { key: "completed", rows: completed.filter((r) => completedToday(r, now)) },
      {
        key: "completed-earlier",
        rows: completed.filter((r) => !completedToday(r, now)),
      },
    ];
    return buckets.filter((g) => g.rows.length > 0);
  });

  return { groups, matching, sortedMatching };
}
