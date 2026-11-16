<script setup lang="ts">
// The pinned progress block of the details panel. Each fact shows once:
// status and percent on the first line, the bar, then bytes and speed.
// A queued download has no bar — there is nothing to fill yet.

import { computed, type Component } from "vue";
import { useI18n } from "vue-i18n";
import { AlertCircle, CheckCircle2, Clock, Pause } from "lucide-vue-next";

import { useDownloadsStore } from "@/stores/downloads";
import { useElapsedSeconds } from "@/composables/useElapsed";
import {
  approxDuration,
  formatAdded,
  formatBytes,
  formatEta,
  formatSpeed,
  percent,
} from "@/lib/format";
import type { DownloadRecord } from "@/types/tauri-bindings";

const props = defineProps<{ download: DownloadRecord }>();

const { t, locale } = useI18n();
const downloads = useDownloadsStore();

const stats = computed(() => downloads.statsFor(props.download.id));
const pct = computed(() =>
  percent(props.download.downloaded_bytes, props.download.total_bytes),
);
const hasKnownSize = computed(
  () => props.download.total_bytes != null && props.download.total_bytes > 0,
);
const elapsedSeconds = useElapsedSeconds(() => props.download);

interface Tone {
  text: string;
  bar: string;
  icon: Component | null;
}

const tone = computed<Tone>(() => {
  switch (props.download.status) {
    case "active":
      return { text: "text-primary", bar: "bg-primary", icon: null };
    case "muxing":
      return { text: "text-info", bar: "bg-info", icon: null };
    case "queued":
      return { text: "text-info", bar: "bg-info", icon: Clock };
    case "failed":
      return { text: "text-danger", bar: "bg-danger", icon: AlertCircle };
    case "completed":
      return { text: "text-success", bar: "bg-success", icon: CheckCircle2 };
    default:
      return { text: "text-dl-soft", bar: "bg-muted-foreground", icon: Pause };
  }
});

const headline = computed(() => {
  const withPct = hasKnownSize.value;
  switch (props.download.status) {
    case "active":
      return withPct
        ? t("detail.headlineDownloading", { pct: pct.value })
        : t("downloads.groupDownloading");
    case "muxing":
      return t("downloads.statusMuxing");
    case "queued":
      return t("downloads.statusQueued");
    case "paused":
      return withPct
        ? t("detail.headlinePaused", { pct: pct.value })
        : t("downloads.statusPaused");
    case "failed":
      return withPct
        ? t("downloads.rowFailedAt", { pct: pct.value })
        : t("downloads.statusFailed");
    case "completed":
      return t("downloads.statusDone");
    default:
      return t("downloads.statusCancelled");
  }
});

/** Right end of the first line: time left, or when it finished. */
const aside = computed(() => {
  const r = props.download;
  if (r.status === "active") {
    const eta = stats.value?.eta;
    return eta != null && eta > 0 ? t("detail.timeLeft", { time: formatEta(eta) }) : "";
  }
  if (r.status === "completed" && r.completed_at) {
    return formatAdded(r.completed_at, t("downloads.addedYesterday"), locale.value);
  }
  return "";
});

const showBar = computed(() => props.download.status !== "queued");
const barWidth = computed(() =>
  props.download.status === "completed" ? 100 : pct.value,
);

const bytesLine = computed(() => {
  const r = props.download;
  const total = formatBytes(r.total_bytes ?? r.downloaded_bytes);
  if (r.status === "completed") {
    const seconds = elapsedSeconds.value;
    if (seconds == null || seconds <= 0) return total;
    const took = approxDuration(seconds);
    return t("detail.sizeInTime", { size: total, time: t(`downloads.duration_${took.unit}`, { n: took.n }) });
  }
  if (r.status === "queued") {
    return hasKnownSize.value ? total : t("detail.sizeUnknown");
  }
  if (!hasKnownSize.value) return formatBytes(r.downloaded_bytes);
  const of = t("downloads.rowBytesOf", {
    done: formatBytes(r.downloaded_bytes),
    total: formatBytes(r.total_bytes),
  });
  return r.status === "failed" || r.status === "paused"
    ? t("detail.bytesKept", { bytes: of })
    : of;
});

const speedLine = computed(() =>
  props.download.status === "active" || props.download.status === "muxing"
    ? formatSpeed(stats.value?.speed_bps)
    : "",
);
</script>

<template>
  <div class="flex shrink-0 flex-col gap-2 px-5 pb-3.5">
    <div class="flex items-center gap-2 text-xs">
      <component
        :is="tone.icon"
        v-if="tone.icon"
        class="h-3.5 w-3.5 shrink-0"
        :class="tone.text"
        aria-hidden="true"
      />
      <span class="font-semibold" :class="tone.text">{{ headline }}</span>
      <span class="ml-auto whitespace-nowrap font-mono text-muted-foreground">{{ aside }}</span>
    </div>
    <span v-if="showBar" class="block h-1.5 rounded-[3px] bg-dl-track">
      <span
        class="block h-1.5 rounded-[3px] transition-[width] duration-300 ease-out"
        :class="tone.bar"
        :style="{ width: `${barWidth}%` }"
      />
    </span>
    <div class="flex justify-between gap-3 font-mono text-xs text-muted-foreground">
      <span>{{ bytesLine }}</span>
      <span>{{ speedLine }}</span>
    </div>
  </div>
</template>
