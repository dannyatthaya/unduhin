import { describe, it, expect, beforeEach } from "vitest";
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

describe("useGroupedDownloads — completed buckets", () => {
  beforeEach(() => setActivePinia(createPinia()));

  it("splits completed rows into today/yesterday/older and never drops them", () => {
    const store = useDownloadsStore();
    store.records = new Map<number, DownloadRecord>([
      [1, rec({ id: 1, completed_at: ago(2 * HOUR) })], // last 24h
      [2, rec({ id: 2, completed_at: ago(30 * HOUR) })], // yesterday
      [3, rec({ id: 3, completed_at: ago(72 * HOUR) })], // older (was dropped before)
      // null completed_at: previously vanished; now bucketed via created_at.
      [4, rec({ id: 4, completed_at: null, created_at: ago(5 * HOUR) })],
    ]);

    const { groups } = useGroupedDownloads(
      () => "",
      () => null,
    );
    const byKey = Object.fromEntries(
      groups.value.map((g) => [g.key, g.rows.map((r) => r.id).sort()]),
    );

    expect(byKey["completed"]).toEqual([1, 4]); // rolling 24h, incl. null fallback
    expect(byKey["completed-yesterday"]).toEqual([2]);
    expect(byKey["completed-older"]).toEqual([3]);

    // Every completed row appears somewhere — none silently dropped.
    const allIds = groups.value.flatMap((g) => g.rows.map((r) => r.id)).sort();
    expect(allIds).toEqual([1, 2, 3, 4]);
  });
});
