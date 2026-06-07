// Builds the four UI groups in the screenshots: Active, Paused &
// Queued, Needs Attention (failed), Completed Today. Also builds the
// flat (un-grouped) row list when the user picks the "Flat" view in
// the sort toolbar — both shapes share the same filter + sort logic so
// switching views doesn't surprise the user with reordering.

import { computed } from "vue";

import { useDownloadsStore } from "@/stores/downloads";
import {
  compareRows,
  type SortColumn,
  type SortDir,
} from "@/composables/useDownloadsSort";
import type { DownloadRecord } from "@/types/tauri-bindings";

export interface DownloadGroup {
  key:
    | "active"
    | "paused-queued"
    | "needs-attention"
    | "cancelled"
    | "completed"
    | "completed-yesterday"
    | "completed-older";
  label: string;
  rows: DownloadRecord[];
}

const DAY_MS = 24 * 3600 * 1000;

/**
 * Age (ms) of a completed row, preferring `completed_at` and falling back
 * to `created_at`. The fallback is what keeps rows whose `completed_at` is
 * NULL (loaded from the DB / legacy rows) visible instead of silently
 * dropped. If neither parses we return +Infinity so the row still lands in
 * the "Older" bucket rather than vanishing.
 */
function completedAgeMs(r: DownloadRecord): number {
  const iso = r.completed_at ?? r.created_at;
  const t = iso ? Date.parse(iso) : Number.NaN;
  if (Number.isNaN(t)) return Number.POSITIVE_INFINITY;
  return Date.now() - t;
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
      if (!q) return true;
      return (
        r.filename.toLowerCase().includes(q) ||
        r.url.toLowerCase().includes(q)
      );
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
    const active = rows.filter(
      (r) => r.status === "active" || r.status === "muxing",
    );
    const pq = rows.filter((r) => r.status === "paused" || r.status === "queued");
    const attention = rows.filter((r) => r.status === "failed");
    // Cancelled rows are terminal but not failures. They previously matched no
    // bucket and silently vanished from the Grouped view, even though the Flat
    // view and the sidebar category / "All downloads" counts both include them
    // — so a cancelled download read as a phantom +1. Give them their own group.
    const cancelled = rows.filter((r) => r.status === "cancelled");

    // Bucket *every* completed row by age so none disappear: rows older
    // than 24h (previously dropped entirely) fall into Yesterday/Older, and
    // rows with a missing completed_at use created_at as a fallback.
    const completed = rows.filter((r) => r.status === "completed");
    const today = completed.filter((r) => completedAgeMs(r) < DAY_MS);
    const yesterday = completed.filter((r) => {
      const age = completedAgeMs(r);
      return age >= DAY_MS && age < 2 * DAY_MS;
    });
    const older = completed.filter((r) => completedAgeMs(r) >= 2 * DAY_MS);

    const out: DownloadGroup[] = [];
    if (active.length) out.push({ key: "active", label: "Active", rows: active });
    if (pq.length) out.push({ key: "paused-queued", label: "Paused & Queued", rows: pq });
    if (attention.length) out.push({ key: "needs-attention", label: "Needs attention", rows: attention });
    if (cancelled.length) out.push({ key: "cancelled", label: "Cancelled", rows: cancelled });
    // Label reflects the rolling-24h window (not a calendar day); the real
    // label is resolved by `DownloadGroup.vue` from the key via i18n.
    if (today.length) out.push({ key: "completed", label: "Completed in the last 24h", rows: today });
    if (yesterday.length) out.push({ key: "completed-yesterday", label: "Completed yesterday", rows: yesterday });
    if (older.length) out.push({ key: "completed-older", label: "Completed earlier", rows: older });
    return out;
  });

  return { groups, matching, sortedMatching };
}
