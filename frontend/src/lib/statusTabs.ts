// The status tabs above the downloads list. Each tab is a bucket of
// backend statuses; this is the single place that mapping lives, so the
// tab counts, the grouped view and the flat view cannot disagree.
//
// Pure functions (no Vue, no store) so the mapping is unit-testable.

import type { Status } from "@/types/tauri-bindings";

export type StatusTab = "all" | "attention" | "downloading" | "queued" | "done";

/** Display order of the tabs. */
export const STATUS_TABS: readonly StatusTab[] = [
  "all",
  "attention",
  "downloading",
  "queued",
  "done",
];

/**
 * The tab a status belongs to, besides "all". `cancelled` belongs to none:
 * it is a legacy terminal state that only shows under "All".
 */
export function statusTabOf(status: Status): Exclude<StatusTab, "all"> | null {
  switch (status) {
    case "failed":
    case "paused":
      return "attention";
    case "active":
    case "muxing":
      return "downloading";
    case "queued":
      return "queued";
    case "completed":
      return "done";
    default:
      return null;
  }
}

export function matchesTab(status: Status, tab: StatusTab): boolean {
  return tab === "all" || statusTabOf(status) === tab;
}

export function countByTab(
  rows: readonly { status: Status }[],
): Record<StatusTab, number> {
  const counts: Record<StatusTab, number> = {
    all: rows.length,
    attention: 0,
    downloading: 0,
    queued: 0,
    done: 0,
  };
  for (const row of rows) {
    const tab = statusTabOf(row.status);
    if (tab) counts[tab] += 1;
  }
  return counts;
}
