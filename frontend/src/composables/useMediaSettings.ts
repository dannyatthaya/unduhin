// Typed accessors for the Media section (yt-dlp + ffmpeg).

import { computed, type WritableComputedRef } from "vue";

import { useSettingsStore } from "@/stores/settings";

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

export function useMediaSettings() {
  return {
    ytdlpBinaryPath: typedString("ytdlp_binary_path", ""),
    ffmpegBinaryPath: typedString("ffmpeg_binary_path", ""),
    defaultFormat: typedString("ytdlp_default_format", "bv*+ba/b"),
    probeTimeoutMs: typedNumber("ytdlp_probe_timeout_ms", 3000, {
      min: 500,
      max: 30_000,
    }),
    consentAcceptedAt: typedString("ytdlp_consent_accepted_at", ""),
  };
}
