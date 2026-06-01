// Per-rule match counters. Machine-local in `chrome.storage.local` so
// the totals don't sync across devices — the dashboard reflects *this*
// machine's traffic, which is the only honest read of "how often is
// this rule earning its keep here".
//
// Writes are buffered to keep the storage write rate sane: a flush
// fires every 5 events or 2 s, whichever comes first. The service
// worker pushes the snapshot up to Tauri every 6 s via
// `chrome.alarms` (`Inbound::RuleMetrics`).

import { log } from "../shared/log.js";
import type { RuleMetric } from "../shared/types.js";

export const RULE_METRICS_KEY = "ruleMetrics";

/** Wire-symmetric on the JS side too. */
export interface StoredMetric {
  matchCount: number;
  lastMatchAt: number | null;
}

export type RuleMetricsStore = Record<string, StoredMetric>;

const FLUSH_EVENT_THRESHOLD = 5;
const FLUSH_INTERVAL_MS = 2_000;

const pending: Map<string, StoredMetric> = new Map();
let flushTimer: ReturnType<typeof setTimeout> | null = null;
let inFlight: Promise<void> | null = null;

/**
 * Record a hit for `pattern`. Cheap (Map mutation only) — the actual
 * `chrome.storage.local.set` is debounced.
 *
 * Skips empty patterns silently. Buffered writes survive SW idle:
 * `chrome.storage.local` writes from a queued flush still land
 * because the SW only sleeps after `void`-returning callbacks settle.
 */
export function recordRuleHit(pattern: string): void {
  if (!pattern) return;
  const prev = pending.get(pattern);
  const now = Date.now();
  pending.set(pattern, {
    matchCount: (prev?.matchCount ?? 0) + 1,
    lastMatchAt: now,
  });
  if (pending.size >= FLUSH_EVENT_THRESHOLD) {
    void flushMetrics();
    return;
  }
  if (!flushTimer) {
    flushTimer = setTimeout(() => {
      flushTimer = null;
      void flushMetrics();
    }, FLUSH_INTERVAL_MS);
  }
}

/**
 * Flush the pending buffer into `chrome.storage.local`. Merges with
 * what's already there so two SW instances don't lose each other's
 * counts (rare — the bridge SW is the only writer in practice, but
 * the merge is cheap). Returns the merged snapshot for callers that
 * want it without a second read.
 */
export async function flushMetrics(): Promise<void> {
  if (inFlight) return inFlight;
  if (pending.size === 0) return;
  if (flushTimer) {
    clearTimeout(flushTimer);
    flushTimer = null;
  }
  const snapshot = new Map(pending);
  pending.clear();
  inFlight = (async () => {
    const current = await readStore();
    for (const [pattern, metric] of snapshot) {
      const prior = current[pattern];
      current[pattern] = {
        matchCount: (prior?.matchCount ?? 0) + metric.matchCount,
        lastMatchAt: Math.max(prior?.lastMatchAt ?? 0, metric.lastMatchAt ?? 0) || null,
      };
    }
    await writeStore(current);
  })()
    .catch((err) => log.warn("rule-metrics flush failed", err))
    .finally(() => {
      inFlight = null;
    });
  return inFlight;
}

/**
 * Snapshot of the on-disk store, shaped for the wire. Used by the
 * `chrome.alarms` tick that pushes `Inbound::RuleMetrics` to Tauri.
 */
export async function snapshotForWire(): Promise<RuleMetric[]> {
  await flushMetrics();
  const store = await readStore();
  const out: RuleMetric[] = [];
  for (const [pattern, metric] of Object.entries(store)) {
    out.push({
      pattern,
      matchCount: metric.matchCount,
      lastMatchAt: metric.lastMatchAt,
    });
  }
  return out;
}

/**
 * Drop counters for any pattern no longer referenced by the user's
 * rule set. Called whenever the settings store changes — we don't
 * want stale rows accruing forever.
 */
export async function pruneTo(activePatterns: ReadonlySet<string>): Promise<void> {
  const store = await readStore();
  let changed = false;
  for (const pattern of Object.keys(store)) {
    if (!activePatterns.has(pattern)) {
      delete store[pattern];
      changed = true;
    }
  }
  if (changed) await writeStore(store);
}

async function readStore(): Promise<RuleMetricsStore> {
  return new Promise((resolve) => {
    chrome.storage.local.get({ [RULE_METRICS_KEY]: {} }, (items) => {
      const raw = items[RULE_METRICS_KEY];
      if (!raw || typeof raw !== "object") {
        resolve({});
        return;
      }
      const out: RuleMetricsStore = {};
      for (const [pattern, entry] of Object.entries(raw as Record<string, unknown>)) {
        if (!entry || typeof entry !== "object") continue;
        const obj = entry as { matchCount?: unknown; lastMatchAt?: unknown };
        const matchCount =
          typeof obj.matchCount === "number" && Number.isFinite(obj.matchCount)
            ? Math.max(0, Math.trunc(obj.matchCount))
            : 0;
        const lastMatchAt =
          typeof obj.lastMatchAt === "number" && Number.isFinite(obj.lastMatchAt)
            ? obj.lastMatchAt
            : null;
        out[pattern] = { matchCount, lastMatchAt };
      }
      resolve(out);
    });
  });
}

async function writeStore(store: RuleMetricsStore): Promise<void> {
  return new Promise((resolve) => {
    chrome.storage.local.set({ [RULE_METRICS_KEY]: store }, () => resolve());
  });
}
