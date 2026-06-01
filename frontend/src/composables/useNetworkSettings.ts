// Typed accessors for the Network section.

import { computed, type WritableComputedRef } from "vue";

import { useSettingsStore } from "@/stores/settings";

function typedNumber(
  key: string,
  fallback: number,
  bounds?: { min?: number; max?: number },
): WritableComputedRef<number> {
  const s = useSettingsStore();
  return computed({
    get() {
      const v = s.values[key];
      if (typeof v === "number" && Number.isFinite(v)) return v;
      return fallback;
    },
    set(next) {
      let n = Math.floor(next);
      if (!Number.isFinite(n)) n = fallback;
      if (bounds?.min != null) n = Math.max(bounds.min, n);
      if (bounds?.max != null) n = Math.min(bounds.max, n);
      void s.set(key, n);
    },
  });
}

function typedString(key: string, fallback = ""): WritableComputedRef<string> {
  const s = useSettingsStore();
  return computed({
    get() {
      const v = s.values[key];
      return typeof v === "string" ? v : fallback;
    },
    set(next) {
      void s.set(key, next);
    },
  });
}

export function useNetworkSettings() {
  return {
    connectTimeoutSecs: typedNumber("connect_timeout_secs", 15, { min: 1, max: 600 }),
    readTimeoutSecs: typedNumber("read_timeout_secs", 60, { min: 1, max: 600 }),
    maxRetries: typedNumber("max_retries", 5, { min: 1, max: 20 }),
    retryBackoffBaseMs: typedNumber("retry_backoff_base_ms", 500, {
      min: 100,
      max: 60_000,
    }),
    userAgent: typedString("user_agent", ""),
  };
}
