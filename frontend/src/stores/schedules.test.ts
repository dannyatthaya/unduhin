import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createPinia, setActivePinia } from "pinia";

import { useSchedulesStore } from "./schedules";
import type { Schedule } from "@/types/tauri-bindings";

function row(over: Partial<Schedule> = {}): Schedule {
  return {
    id: 1,
    kind: "start_at",
    download_id: 42,
    start_iso: "2026-06-01T22:00:00Z",
    end_iso: null,
    days_mask: 127,
    active: true,
    created_at: "2026-05-25T00:00:00Z",
    ...over,
  };
}

describe("useSchedulesStore selectors", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  afterEach(() => {
    setActivePinia(undefined);
  });

  it("byDownload groups per-download rows and skips globals", () => {
    const store = useSchedulesStore();
    store.list = [
      row({ id: 1, kind: "start_at", download_id: 42 }),
      row({ id: 2, kind: "after_queue", download_id: 42, start_iso: null }),
      row({ id: 3, kind: "start_at", download_id: 7 }),
      row({
        id: 4,
        kind: "quiet_hours",
        download_id: null,
        start_iso: "22:00",
        end_iso: "07:00",
      }),
    ];
    expect(store.byDownload.get(42)?.length).toBe(2);
    expect(store.byDownload.get(7)?.length).toBe(1);
    expect(store.byDownload.size).toBe(2); // global row excluded
  });

  it("globalQuietHours returns the singleton global row", () => {
    const store = useSchedulesStore();
    store.list = [
      row({ id: 1, kind: "start_at", download_id: 42 }),
      row({
        id: 2,
        kind: "quiet_hours",
        download_id: null,
        start_iso: "22:00",
        end_iso: "07:00",
      }),
    ];
    expect(store.globalQuietHours?.id).toBe(2);
  });

  it("globalQuietHours is null when no global row exists", () => {
    const store = useSchedulesStore();
    store.list = [row({ id: 1, kind: "start_at", download_id: 42 })];
    expect(store.globalQuietHours).toBeNull();
  });
});
