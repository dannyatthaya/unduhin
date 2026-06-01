// Sort + view-mode preferences for the downloads list. Persists as a
// single JSON object under the `downloads_sort` setting key so a single
// round-trip carries all three fields. Reads are tolerant — anything
// that doesn't match the expected shape falls back to the default —
// since the same key may have been written by a different app version.

import { computed, type WritableComputedRef } from "vue";

import { useSettingsStore } from "@/stores/settings";
import type { DownloadRecord } from "@/types/tauri-bindings";

export type SortColumn =
  | "filename"
  | "size"
  | "speed"
  | "eta"
  | "status"
  | "added_at";

export type SortDir = "asc" | "desc";

export type ViewMode = "grouped" | "flat";

export interface DownloadsSort {
  view: ViewMode;
  column: SortColumn;
  dir: SortDir;
}

export const DEFAULT_SORT: DownloadsSort = {
  view: "grouped",
  column: "added_at",
  dir: "desc",
};

const COLUMNS: ReadonlyArray<SortColumn> = [
  "filename",
  "size",
  "speed",
  "eta",
  "status",
  "added_at",
];

const VIEWS: ReadonlyArray<ViewMode> = ["grouped", "flat"];

// Stable status ordinal — what feels like the natural top-to-bottom
// reading order in the UI. Used by the comparator when `column ===
// "status"`. Active first, completed last; cancelled trails (visually
// it's the least interesting end state).
const STATUS_ORDER: Record<string, number> = {
  active: 0,
  muxing: 1,
  queued: 2,
  paused: 3,
  failed: 4,
  cancelled: 5,
  completed: 6,
};

export function parseSort(raw: unknown): DownloadsSort {
  if (!raw || typeof raw !== "object") return { ...DEFAULT_SORT };
  const obj = raw as Partial<DownloadsSort>;
  const view: ViewMode = VIEWS.includes(obj.view as ViewMode)
    ? (obj.view as ViewMode)
    : DEFAULT_SORT.view;
  const column: SortColumn = COLUMNS.includes(obj.column as SortColumn)
    ? (obj.column as SortColumn)
    : DEFAULT_SORT.column;
  const dir: SortDir = obj.dir === "asc" || obj.dir === "desc"
    ? obj.dir
    : DEFAULT_SORT.dir;
  return { view, column, dir };
}

/**
 * Key extractor for the sort comparator. Exported as a pure function so
 * Vitest can exercise it without spinning up a store. `stats` is the
 * runtime speed snapshot (returned by `useDownloadsStore.statsFor`); we
 * accept the un-typed bag here to keep the import surface tight.
 */
export function sortKey(
  row: DownloadRecord,
  column: SortColumn,
  stats: { speed_bps: number; eta: number | null } | null,
): number | string {
  switch (column) {
    case "filename":
      return row.filename.toLowerCase();
    case "size":
      return row.total_bytes ?? 0;
    case "speed":
      // Rows with no live stats are sorted to the end regardless of
      // direction. We achieve that by returning -Infinity for desc-
      // friendly ordering; the comparator inverts for asc.
      return stats?.speed_bps ?? -1;
    case "eta":
      // Same sentinel idea: rows with no ETA go to the end.
      return stats?.eta ?? Number.POSITIVE_INFINITY;
    case "status":
      return STATUS_ORDER[row.status] ?? 99;
    case "added_at":
      return Date.parse(row.created_at) || 0;
  }
}

export function compareRows(
  a: DownloadRecord,
  b: DownloadRecord,
  column: SortColumn,
  dir: SortDir,
  statsFor: (id: number) => { speed_bps: number; eta: number | null } | null,
): number {
  const av = sortKey(a, column, statsFor(a.id));
  const bv = sortKey(b, column, statsFor(b.id));
  let cmp: number;
  if (typeof av === "number" && typeof bv === "number") {
    cmp = av - bv;
  } else {
    cmp = String(av).localeCompare(String(bv));
  }
  if (cmp === 0) {
    // Secondary stable key: created_at descending (newest first).
    const ta = Date.parse(a.created_at) || 0;
    const tb = Date.parse(b.created_at) || 0;
    cmp = tb - ta;
  }
  return dir === "asc" ? cmp : -cmp;
}

export function useDownloadsSort(): {
  state: WritableComputedRef<DownloadsSort>;
  view: WritableComputedRef<ViewMode>;
  column: WritableComputedRef<SortColumn>;
  dir: WritableComputedRef<SortDir>;
  toggleColumn: (next: SortColumn) => void;
  toggleView: () => void;
} {
  const settings = useSettingsStore();

  const state = computed<DownloadsSort>({
    get: () => parseSort(settings.values["downloads_sort"]),
    set(next) {
      void settings.set("downloads_sort", {
        view: next.view,
        column: next.column,
        dir: next.dir,
      });
    },
  });

  function patch(partial: Partial<DownloadsSort>) {
    state.value = { ...state.value, ...partial };
  }

  return {
    state,
    view: computed({
      get: () => state.value.view,
      set: (v) => patch({ view: v }),
    }),
    column: computed({
      get: () => state.value.column,
      set: (c) => patch({ column: c }),
    }),
    dir: computed({
      get: () => state.value.dir,
      set: (d) => patch({ dir: d }),
    }),
    /**
     * Clicking the active column flips direction; clicking a different
     * column switches to it with `desc` as the default (most useful for
     * size/speed/added-at; less intuitive for filename, but a single
     * direction default keeps the toolbar predictable).
     */
    toggleColumn(next: SortColumn) {
      const cur = state.value;
      if (cur.column === next) {
        patch({ dir: cur.dir === "asc" ? "desc" : "asc" });
      } else {
        patch({ column: next, dir: "desc" });
      }
    },
    toggleView() {
      patch({ view: state.value.view === "grouped" ? "flat" : "grouped" });
    },
  };
}
