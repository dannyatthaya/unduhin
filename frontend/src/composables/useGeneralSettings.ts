// Typed accessors over `useSettingsStore` for the General section. Each
// returned ref is writable; writes go to the store (which fires the
// `setting_changed` event) and also propagate locally so the UI stays in
// sync without a round-trip.

import { computed, type WritableComputedRef } from "vue";

import { useSettingsStore } from "@/stores/settings";
import type { ThemeMode } from "@/composables/useTheme";

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

function typedBoolean(key: string, fallback: boolean): WritableComputedRef<boolean> {
  const s = useSettingsStore();
  return computed({
    get() {
      const v = s.values[key];
      return typeof v === "boolean" ? v : fallback;
    },
    set(next) {
      void s.set(key, next);
    },
  });
}

function typedTheme(key: string): WritableComputedRef<ThemeMode> {
  const s = useSettingsStore();
  return computed({
    get() {
      const v = s.values[key];
      if (v === "light" || v === "dark" || v === "system") return v;
      return "system";
    },
    set(next) {
      void s.set(key, next);
    },
  });
}

export type DeleteAction = "ask" | "row_only" | "row_and_data";

function typedDeleteAction(key: string): WritableComputedRef<DeleteAction> {
  const s = useSettingsStore();
  return computed({
    get() {
      const v = s.values[key];
      if (v === "ask" || v === "row_only" || v === "row_and_data") return v;
      return "ask";
    },
    set(next) {
      void s.set(key, next);
    },
  });
}

export function useGeneralSettings() {
  return {
    defaultOutputPath: typedString("default_output_path", ""),
    defaultSegments: typedNumber("default_segments", 8, { min: 1, max: 32 }),
    maxConcurrent: typedNumber("max_concurrent_downloads", 4, { min: 1, max: 16 }),
    globalSpeedBps: typedNumber("global_speed_limit_bps", 0, { min: 0 }),
    themeMode: typedTheme("theme_mode"),
    deleteDefaultAction: typedDeleteAction("delete_default_action"),
    alwaysAskFilename: typedBoolean("always_ask_filename", false),
  };
}
