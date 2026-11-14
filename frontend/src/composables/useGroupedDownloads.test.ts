import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

import { useDownloadsStore } from "@/stores/downloads";
import { useGroupedDownloads } from "@/composables/useGroupedDownloads";
import type { DownloadRecord } from "@/types/tauri-bindings";

function rec(over: Partial<DownloadRecord>): DownloadRecord {
  return {
    id: 1,
    url: "https://example.com/y.bin",
    filename: "y.bin",
    output_path: "/tmp/y.bin",
    total_bytes: 1,
    downloaded_bytes: 1,
    status: "completed",
    error: null,
    error_kind: null,
    category_id: null,
    priority: 0,
    segments: 1,
    created_at: new Date(Date.now()).toISOString(),
    completed_at: null,
    etag: null,
    last_modified: null,
    segments_meta: null,
    media_info: null,
    speed_samples: null,
    kind: "http",
    torrent: null,
    ...over,
  };
}

const HOUR = 3600 * 1000;
const ago = (ms: number) => new Date(Date.now() - ms).toISOString();

function idsByKey(groups: { key: string; rows: DownloadRecord[] }[]) {
  return Object.fromEntries(
    groups.map((g) => [g.key, g.rows.map((r) => r.id).sort()]),
  );
}

describe("useGroupedDownloads", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    // Mid-afternoon, so "a few hours ago" is unambiguously today and the
    // calendar-day buckets don't depend on when the suite runs.
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 10, 14, 15, 0, 0));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("splits completed rows into today and earlier by calendar day, never dropping them", () => {
    const store = useDownloadsStore();
    store.records = new Map<number, DownloadRecord>([
      [1, rec({ id: 1, completed_at: ago(2 * HOUR) })], // today
      // 18 hours ago is inside 24h but on yesterday's calendar day.
      [2, rec({ id: 2, completed_at: ago(18 * HOUR) })],
      [3, rec({ id: 3, completed_at: ago(72 * HOUR) })],
      // null completed_at: bucketed via created_at instead of vanishing.
      [4, rec({ id: 4, completed_at: null, created_at: ago(5 * HOUR) })],
      // Unparseable dates fall into "earlier" rather than disappearing.
      [5, rec({ id: 5, completed_at: "not-a-date" })],
    ]);

    const { groups } = useGroupedDownloads(
      () => "",
      () => null,
    );
    const byKey = idsByKey(groups.value);

    expect(byKey["completed"]).toEqual([1, 4]);
    expect(byKey["completed-earlier"]).toEqual([2, 3, 5]);
  });

  it("puts failed and paused rows under needs-attention, apart from the queue", () => {
    const store = useDownloadsStore();
    store.records = new Map<number, DownloadRecord>([
      [1, rec({ id: 1, status: "failed" })],
      [2, rec({ id: 2, status: "paused" })],
      [3, rec({ id: 3, status: "queued" })],
      [4, rec({ id: 4, status: "active" })],
      [5, rec({ id: 5, status: "muxing" })],
    ]);

    const { groups } = useGroupedDownloads(
      () => "",
      () => null,
    );

    expect(groups.value.map((g) => g.key)).toEqual([
      "needs-attention",
      "active",
      "queued",
    ]);
    const byKey = idsByKey(groups.value);
    expect(byKey["needs-attention"]).toEqual([1, 2]);
    expect(byKey["active"]).toEqual([4, 5]);
    expect(byKey["queued"]).toEqual([3]);
  });

  it("groups cancelled rows instead of dropping them (count/Grouped mismatch fix)", () => {
    const store = useDownloadsStore();
    store.records = new Map<number, DownloadRecord>([
      [1, rec({ id: 1, status: "cancelled" })],
      [2, rec({ id: 2, status: "active" })],
      [3, rec({ id: 3, status: "completed", completed_at: ago(HOUR) })],
    ]);

    const { groups, sortedMatching } = useGroupedDownloads(
      () => "",
      () => null,
    );

    // Cancelled rows get their own group rather than vanishing.
    expect(idsByKey(groups.value)["cancelled"]).toEqual([1]);

    // The Grouped view must show exactly the same rows as the Flat view
    // (`sortedMatching`) — which is what the sidebar category / "All
    // downloads" counts are derived from. No row is counted-but-hidden.
    const groupedIds = groups.value
      .flatMap((g) => g.rows.map((r) => r.id))
      .sort();
    const flatIds = sortedMatching.value.map((r) => r.id).sort();
    expect(groupedIds).toEqual(flatIds);
    expect(groupedIds).toEqual([1, 2, 3]);
  });

  it("searches file names, links and folders", () => {
    const store = useDownloadsStore();
    store.records = new Map<number, DownloadRecord>([
      [1, rec({ id: 1, filename: "Invoice.pdf" })],
      [2, rec({ id: 2, url: "https://files.example.net/invoice/a.bin" })],
      [3, rec({ id: 3, output_path: "C:\\Docs\\Invoices\\b.bin" })],
      [4, rec({ id: 4, filename: "clip.mp4" })],
    ]);

    const { matching } = useGroupedDownloads(
      () => "  INVOICE ",
      () => null,
    );

    expect(matching.value.map((r) => r.id).sort()).toEqual([1, 2, 3]);
  });

  it("filters by category before searching", () => {
    const store = useDownloadsStore();
    store.records = new Map<number, DownloadRecord>([
      [1, rec({ id: 1, category_id: 7 })],
      [2, rec({ id: 2, category_id: 8 })],
    ]);

    const { matching } = useGroupedDownloads(
      () => "",
      () => 7,
    );

    expect(matching.value.map((r) => r.id)).toEqual([1]);
  });
});
