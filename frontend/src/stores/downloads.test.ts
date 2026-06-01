import { describe, it, expect } from "vitest";

import { applyEvent, type DownloadsState } from "./downloads";
import type { DownloadRecord, CoreEvent } from "@/types/tauri-bindings";

function emptyState(): DownloadsState {
  return { records: new Map(), stats: new Map(), liveSegments: new Map() };
}

function sampleRecord(overrides: Partial<DownloadRecord> = {}): DownloadRecord {
  return {
    id: 1,
    url: "https://example.com/file.bin",
    filename: "file.bin",
    output_path: "C:\\downloads\\file.bin",
    total_bytes: 1000,
    downloaded_bytes: 0,
    status: "queued",
    error: null,
    category_id: null,
    priority: 0,
    segments: 8,
    created_at: "2026-05-20T00:00:00Z",
    completed_at: null,
    etag: null,
    last_modified: null,
    segments_meta: null,
    media_info: null,
    speed_samples: null,
    ...overrides,
  };
}

describe("applyEvent", () => {
  it("inserts a record on download_added", () => {
    const state = emptyState();
    const ev: CoreEvent = {
      type: "download_added",
      id: 1,
      snapshot: sampleRecord(),
    };
    applyEvent(state, ev);
    expect(state.records.size).toBe(1);
    expect(state.records.get(1)?.filename).toBe("file.bin");
  });

  it("updates status on status_changed and clears stats when leaving active", () => {
    const state = emptyState();
    state.records.set(1, sampleRecord({ status: "active" }));
    state.stats.set(1, { speed_bps: 10_000, eta: 12 });

    applyEvent(state, { type: "status_changed", id: 1, from: "active", to: "paused" });
    expect(state.records.get(1)?.status).toBe("paused");
    expect(state.stats.has(1)).toBe(false);
  });

  it("keeps stats when transitioning to active", () => {
    const state = emptyState();
    state.records.set(1, sampleRecord({ status: "queued" }));
    state.stats.set(1, { speed_bps: 10_000, eta: 12 });

    applyEvent(state, { type: "status_changed", id: 1, from: "queued", to: "active" });
    expect(state.records.get(1)?.status).toBe("active");
    expect(state.stats.get(1)?.speed_bps).toBe(10_000);
  });

  it("updates downloaded/total on progress_update and writes stats", () => {
    const state = emptyState();
    state.records.set(1, sampleRecord({ status: "active" }));

    applyEvent(state, {
      type: "progress_update",
      id: 1,
      downloaded: 512,
      total: 2048,
      speed_bps: 1024,
      eta: 1.5,
    });

    expect(state.records.get(1)?.downloaded_bytes).toBe(512);
    expect(state.records.get(1)?.total_bytes).toBe(2048);
    expect(state.stats.get(1)).toEqual({ speed_bps: 1024, eta: 1.5 });
  });

  it("marks completed and stamps completed_at", () => {
    const state = emptyState();
    state.records.set(1, sampleRecord({ status: "active", total_bytes: null }));
    applyEvent(state, { type: "completed", id: 1, bytes: 5000 });
    const rec = state.records.get(1);
    expect(rec?.status).toBe("completed");
    expect(rec?.downloaded_bytes).toBe(5000);
    expect(rec?.total_bytes).toBe(5000);
    expect(rec?.completed_at).toBeTruthy();
    expect(state.stats.has(1)).toBe(false);
  });

  it("records an error message on failed", () => {
    const state = emptyState();
    state.records.set(1, sampleRecord({ status: "active" }));
    state.stats.set(1, { speed_bps: 1024, eta: 1 });
    applyEvent(state, { type: "failed", id: 1, error: "Connection reset" });
    expect(state.records.get(1)?.status).toBe("failed");
    expect(state.records.get(1)?.error).toBe("Connection reset");
    expect(state.stats.has(1)).toBe(false);
  });

  it("removes both record and stats on removed", () => {
    const state = emptyState();
    state.records.set(1, sampleRecord());
    state.stats.set(1, { speed_bps: 1, eta: null });
    state.liveSegments.set(
      1,
      new Map([[0, { index: 0, bytes: 5, total: 10, speed_bps: 1, state: "active" }]]),
    );
    applyEvent(state, { type: "removed", id: 1 });
    expect(state.records.size).toBe(0);
    expect(state.stats.size).toBe(0);
    expect(state.liveSegments.size).toBe(0);
  });

  it("writes segment_progress into liveSegments per download/index", () => {
    const state = emptyState();
    state.records.set(1, sampleRecord({ status: "active" }));

    applyEvent(state, {
      type: "segment_progress",
      id: 1,
      index: 0,
      bytes: 100,
      total: 1000,
      speed_bps: 4096,
      state: "active",
    });
    applyEvent(state, {
      type: "segment_progress",
      id: 1,
      index: 1,
      bytes: 50,
      total: 1000,
      speed_bps: 1024,
      state: "slow",
    });
    // Second tick for index 0 — should overwrite the previous entry.
    applyEvent(state, {
      type: "segment_progress",
      id: 1,
      index: 0,
      bytes: 1000,
      total: 1000,
      speed_bps: 0,
      state: "done",
    });

    const map = state.liveSegments.get(1);
    expect(map?.size).toBe(2);
    expect(map?.get(0)).toEqual({
      index: 0,
      bytes: 1000,
      total: 1000,
      speed_bps: 0,
      state: "done",
    });
    expect(map?.get(1)?.state).toBe("slow");
  });

  it("changes category on category_changed", () => {
    const state = emptyState();
    state.records.set(1, sampleRecord({ category_id: 2 }));
    applyEvent(state, { type: "category_changed", id: 1, category_id: 5 });
    expect(state.records.get(1)?.category_id).toBe(5);
  });

  it("updates segment count on segments_changed", () => {
    const state = emptyState();
    state.records.set(1, sampleRecord({ segments: 4 }));
    applyEvent(state, { type: "segments_changed", id: 1, n: 12 });
    expect(state.records.get(1)?.segments).toBe(12);
  });

  it("ignores progress for unknown ids without crashing", () => {
    const state = emptyState();
    applyEvent(state, {
      type: "progress_update",
      id: 99,
      downloaded: 100,
      total: 1000,
      speed_bps: 50,
      eta: 1,
    });
    expect(state.records.size).toBe(0);
    // Stats are still written — they're keyed by id even without a record,
    // useful if `download_added` arrives slightly after the first tick.
    expect(state.stats.get(99)).toEqual({ speed_bps: 50, eta: 1 });
  });

  it("ignores setting_changed entirely (handled by settings store)", () => {
    const state = emptyState();
    state.records.set(1, sampleRecord());
    applyEvent(state, { type: "setting_changed", key: "default_segments" });
    expect(state.records.size).toBe(1);
  });
});
