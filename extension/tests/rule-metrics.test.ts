// `recordRuleHit` buffers writes (every 5 hits or 2s),
// merges with the existing on-disk store, and the wire snapshot is a
// flat list keyed by pattern.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

interface FakeStorage {
  local: Record<string, unknown>;
  localSetSpy: ReturnType<typeof vi.fn>;
}

function installFakeChrome(): FakeStorage {
  const fake: FakeStorage = {
    local: {},
    localSetSpy: vi.fn((items: Record<string, unknown>, cb: () => void) => {
      Object.assign(fake.local, items);
      cb();
    }),
  };
  (globalThis as unknown as { chrome: unknown }).chrome = {
    storage: {
      local: {
        get: (
          defaults: Record<string, unknown>,
          cb: (items: Record<string, unknown>) => void,
        ) => {
          const result: Record<string, unknown> = { ...defaults };
          for (const key of Object.keys(defaults)) {
            if (key in fake.local) result[key] = fake.local[key];
          }
          cb(result);
        },
        set: fake.localSetSpy,
      },
      onChanged: {
        addListener: () => {},
        removeListener: () => {},
      },
    },
  };
  return fake;
}

installFakeChrome();

const mod = await import("../src/background/rule-metrics.js");
const { recordRuleHit, flushMetrics, snapshotForWire, pruneTo, RULE_METRICS_KEY } = mod;

describe("rule-metrics", () => {
  let fake: FakeStorage;

  beforeEach(() => {
    vi.useFakeTimers();
    fake = installFakeChrome();
  });

  afterEach(() => {
    vi.useRealTimers();
    delete (globalThis as unknown as { chrome?: unknown }).chrome;
  });

  it("buffers under the threshold and flushes after the interval", async () => {
    recordRuleHit("*.example.com");
    recordRuleHit("*.example.com");
    expect(fake.localSetSpy).not.toHaveBeenCalled();
    // Fire the debounce window.
    await vi.advanceTimersByTimeAsync(2_500);
    expect(fake.localSetSpy).toHaveBeenCalledTimes(1);
    const stored = fake.local[RULE_METRICS_KEY] as Record<string, { matchCount: number }>;
    expect(stored["*.example.com"]?.matchCount).toBe(2);
  });

  it("flushes immediately at the 5-event threshold", async () => {
    for (let i = 0; i < 5; i++) recordRuleHit("files.example.com");
    // The 5th hit triggers an in-flight flush.
    await Promise.resolve();
    await vi.runAllTimersAsync();
    expect(fake.localSetSpy).toHaveBeenCalled();
    const stored = fake.local[RULE_METRICS_KEY] as Record<string, { matchCount: number }>;
    expect(stored["files.example.com"]?.matchCount).toBe(5);
  });

  it("merges with the existing on-disk counters", async () => {
    fake.local[RULE_METRICS_KEY] = {
      "drive.example.com": { matchCount: 3, lastMatchAt: 1_000 },
    };
    recordRuleHit("drive.example.com");
    recordRuleHit("drive.example.com");
    await flushMetrics();
    const stored = fake.local[RULE_METRICS_KEY] as Record<string, { matchCount: number }>;
    expect(stored["drive.example.com"]?.matchCount).toBe(5);
  });

  it("snapshotForWire emits one entry per pattern", async () => {
    recordRuleHit("a.example.com");
    recordRuleHit("b.example.com");
    recordRuleHit("a.example.com");
    const snapshot = await snapshotForWire();
    const byPattern = Object.fromEntries(snapshot.map((m) => [m.pattern, m.matchCount]));
    expect(byPattern["a.example.com"]).toBe(2);
    expect(byPattern["b.example.com"]).toBe(1);
  });

  it("pruneTo drops counters for patterns no longer in the active set", async () => {
    fake.local[RULE_METRICS_KEY] = {
      keep: { matchCount: 4, lastMatchAt: 1_000 },
      drop: { matchCount: 2, lastMatchAt: 999 },
    };
    await pruneTo(new Set(["keep"]));
    const stored = fake.local[RULE_METRICS_KEY] as Record<string, unknown>;
    expect(stored.keep).toBeDefined();
    expect(stored.drop).toBeUndefined();
  });

  it("ignores empty patterns", () => {
    recordRuleHit("");
    // No debounce timer should have been scheduled, so no flush ever fires.
    expect(fake.localSetSpy).not.toHaveBeenCalled();
  });
});
