import { describe, expect, test } from "vitest";

import { buildPartRows, firstSlowPart, partWeight } from "./partRows";
import type { SegmentLive, SegmentState } from "@/types/tauri-bindings";

function part(index: number, start: number, end: number, done: number): SegmentState {
  return { segment: { index, start, end }, bytes_downloaded: done };
}

function live(
  index: number,
  bytes: number,
  total: number,
  state: SegmentLive["state"],
  speed = 100,
): SegmentLive {
  return { index, bytes, total, speed_bps: speed, state };
}

describe("buildPartRows", () => {
  test("derives range and percent from the persisted parts", () => {
    const rows = buildPartRows([part(0, 0, 1000, 250)], undefined, 1);

    expect(rows).toEqual([
      {
        index: 0,
        start: 0,
        end: 1000,
        done: 250,
        pct: 25,
        speedBps: null,
        isDone: false,
        isSlow: false,
      },
    ]);
  });

  test("prefers live bytes when they are ahead of the saved ones", () => {
    const liveMap = new Map([[0, live(0, 600, 1000, "active", 42)]]);

    const [row] = buildPartRows([part(0, 0, 1000, 250)], liveMap, 1);

    expect(row.done).toBe(600);
    expect(row.pct).toBe(60);
    expect(row.speedBps).toBe(42);
  });

  test("marks a part slow only on the engine's verdict", () => {
    const liveMap = new Map([
      [0, live(0, 100, 1000, "slow")],
      [1, live(1, 100, 1000, "stalled")],
      [2, live(2, 100, 1000, "active")],
    ]);
    const parts = [part(0, 0, 1000, 100), part(1, 1000, 2000, 100), part(2, 2000, 3000, 100)];

    const rows = buildPartRows(parts, liveMap, 3);

    expect(rows.map((r) => r.isSlow)).toEqual([true, true, false]);
  });

  test("a finished part reports no speed and reads 100%", () => {
    const liveMap = new Map([[0, live(0, 1000, 1000, "done", 77)]]);

    const [row] = buildPartRows([part(0, 0, 1000, 990)], liveMap, 1);

    expect(row.isDone).toBe(true);
    expect(row.pct).toBe(100);
    expect(row.speedBps).toBeNull();
  });

  test("treats a fully saved part as done when there is no live data", () => {
    const [row] = buildPartRows([part(0, 0, 1000, 1000)], undefined, 1);

    expect(row.isDone).toBe(true);
  });

  test("falls back to placeholder rows before any range is saved", () => {
    const rows = buildPartRows(null, undefined, 3);

    expect(rows).toHaveLength(3);
    expect(rows[0]).toMatchObject({ start: null, end: null, pct: 0, isDone: false });
  });

  test("placeholder rows still carry live progress", () => {
    const liveMap = new Map([[1, live(1, 500, 1000, "active", 9)]]);

    const rows = buildPartRows([], liveMap, 2);

    expect(rows[1]).toMatchObject({ pct: 50, speedBps: 9 });
  });

  test("a zero-length part is 0%, not NaN", () => {
    const [row] = buildPartRows([part(0, 0, 0, 0)], undefined, 1);

    expect(row.pct).toBe(0);
  });
});

describe("firstSlowPart", () => {
  test("returns the first slow row", () => {
    const liveMap = new Map([[1, live(1, 1, 1000, "slow")]]);
    const rows = buildPartRows([part(0, 0, 1000, 1), part(1, 1000, 2000, 1)], liveMap, 2);

    expect(firstSlowPart(rows)?.index).toBe(1);
  });

  test("returns null when nothing is slow", () => {
    expect(firstSlowPart(buildPartRows([part(0, 0, 1000, 1)], undefined, 1))).toBeNull();
  });
});

describe("partWeight", () => {
  test("is the part's byte length", () => {
    const [row] = buildPartRows([part(0, 100, 400, 0)], undefined, 1);

    expect(partWeight(row)).toBe(300);
  });

  test("is 1 for a placeholder row", () => {
    const [row] = buildPartRows(null, undefined, 1);

    expect(partWeight(row)).toBe(1);
  });
});
