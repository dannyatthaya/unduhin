<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import ProgressBar from "@/components/ProgressBar.vue";
import SpeedSparkline from "./SpeedSparkline.vue";
import SegmentBars from "./SegmentBars.vue";

import { useCategoriesStore } from "@/stores/categories";
import { useDownloadsStore } from "@/stores/downloads";
import { useElapsedSeconds } from "@/composables/useElapsed";
import {
  formatBytes,
  formatEta,
  formatSpeed,
  percent,
  relativeTime,
} from "@/lib/format";
import type { DownloadRecord } from "@/types/tauri-bindings";

const { t } = useI18n();
const props = defineProps<{ download: DownloadRecord }>();
const emit = defineEmits<{ "view-segments": [] }>();

const store = useDownloadsStore();
const categories = useCategoriesStore();

const stats = computed(() => store.statsFor(props.download.id));
const samples = computed(() => store.speedHistoryFor(props.download.id));
const live = computed(() => store.liveSegmentsFor(props.download.id));

const isMedia = computed(() => props.download.media_info != null);

const pct = computed(() =>
  percent(props.download.downloaded_bytes, props.download.total_bytes)
);

const tone = computed(() => {
  switch (props.download.status) {
    case "paused":
      return "paused" as const;
    case "failed":
      return "failed" as const;
    case "completed":
      return "done" as const;
    default:
      return "active" as const;
  }
});

const elapsedSeconds = useElapsedSeconds(() => props.download);
const elapsedLabel = computed(() => {
  const s = elapsedSeconds.value;
  return s == null ? "—" : formatEta(s);
});

const peakSpeed = computed(() => {
  let m = 0;
  for (const v of samples.value) if (v > m) m = v;
  return m;
});
const avgSpeed = computed(() => {
  let total = 0;
  let count = 0;
  for (const v of samples.value) if (v > 0) { total += v; count += 1; }
  return count > 0 ? total / count : 0;
});

/** First segment the engine reports as slow or stalled. The engine
 *  owns this verdict — see `transfer.rs::SegmentSampler::classify`. */
const slowSegmentIndex = computed(() => {
  const liveMap = live.value;
  if (!liveMap) return null;
  for (const seg of liveMap.values()) {
    if (seg.state === "slow" || seg.state === "stalled") {
      return seg.index;
    }
  }
  return null;
});

const slowLabel = computed(() => {
  if (slowSegmentIndex.value == null) return null;
  return t("detail.overviewSlowWarning", { index: slowSegmentIndex.value + 1 });
});

const categoryLine = computed(() => {
  const name = categories.nameOf(props.download.category_id);
  return name;
});

const serverHost = computed(() => {
  try {
    return new URL(props.download.url).hostname;
  } catch {
    return props.download.url;
  }
});

const addedAt = computed(() => {
  const t = Date.parse(props.download.created_at);
  if (Number.isNaN(t)) return "—";
  const d = new Date(t);
  const month = d.toLocaleString("en-US", { month: "short" });
  const day = d.getDate();
  const year = d.getFullYear();
  const hh = d.getHours().toString().padStart(2, "0");
  const mm = d.getMinutes().toString().padStart(2, "0");
  return `${month} ${day}, ${year} · ${hh}:${mm}`;
});
</script>

<template>
  <div class="space-y-6">
    <!-- Headline progress -->
    <section>
      <div class="flex items-baseline justify-between gap-2">
        <span class="text-3xl font-bold tracking-tight text-foreground">
          {{ pct }}%
        </span>
        <span class="text-sm text-muted-foreground">
          <span class="font-medium text-foreground">
            {{ formatBytes(download.downloaded_bytes) }}
          </span>
          /
          {{ formatBytes(download.total_bytes) }}
        </span>
      </div>
      <ProgressBar :value="pct" :tone="tone" class="mt-3" />
      <div class="mt-3 flex items-center justify-between text-xs text-muted-foreground">
        <span class="flex items-center gap-1">
          <span aria-hidden>↓</span>
          <span class="font-medium text-foreground">
            {{ stats ? formatSpeed(stats.speed_bps) : "—" }}
          </span>
        </span>
        <span>
          {{ t("detail.metricEta") }}
          <span class="ml-1 font-medium text-foreground">
            {{ stats ? formatEta(stats.eta) : "—" }}
          </span>
        </span>
        <span>
          {{ t("detail.metricElapsed") }}
          <span class="ml-1 font-medium text-foreground">
            {{ elapsedLabel }}
          </span>
        </span>
      </div>
    </section>

    <!-- Speed sparkline -->
    <section>
      <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
        {{ t("detail.speedLast60s") }}
      </h3>
      <SpeedSparkline :samples="samples" />
      <div class="mt-1 flex items-center justify-between font-mono text-[11px] text-muted-foreground">
        <span>-60s</span>
        <span>
          {{ t("detail.avgShort") }}
          <span class="text-foreground">{{ formatSpeed(avgSpeed) }}</span>
          · {{ t("detail.peakShort") }}
          <span class="text-foreground">{{ formatSpeed(peakSpeed) }}</span>
        </span>
        <span>{{ t("detail.now") }}</span>
      </div>
    </section>

    <!-- Segments preview — hidden for yt-dlp (single-stream) downloads. -->
    <section v-if="!isMedia">
      <header class="mb-2 flex items-center justify-between">
        <h3 class="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
          {{ t("detail.tabSegments") }}
        </h3>
        <button
          type="button"
          class="text-xs font-medium text-primary hover:underline"
          @click="emit('view-segments')"
        >
          {{ t("detail.viewAllSegments", { n: download.segments }) }}
        </button>
      </header>
      <SegmentBars
        :segments="download.segments_meta ?? []"
        :bars="download.segments"
        :slow-segment="slowSegmentIndex"
        :height="44"
      />
      <p v-if="slowLabel" class="mt-3 flex items-start gap-1.5 text-xs text-warning">
        <span aria-hidden>⚠</span>
        <span>{{ slowLabel }}</span>
      </p>
    </section>

    <!-- Metadata -->
    <section>
      <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
        {{ t("detail.metadata") }}
      </h3>
      <dl class="space-y-1.5 text-xs">
        <div class="grid grid-cols-[88px_1fr] items-baseline gap-3">
          <dt class="text-muted-foreground">{{ t("detail.metricUrl") }}</dt>
          <dd class="break-all font-mono text-foreground">{{ download.url }}</dd>
        </div>
        <div class="grid grid-cols-[88px_1fr] items-baseline gap-3">
          <dt class="text-muted-foreground">{{ t("detail.saveTo") }}</dt>
          <dd class="break-all font-mono text-foreground">{{ download.output_path }}</dd>
        </div>
        <div class="grid grid-cols-[88px_1fr] items-baseline gap-3">
          <dt class="text-muted-foreground">{{ t("detail.metricSize") }}</dt>
          <dd class="text-foreground">
            <span class="font-medium">{{ formatBytes(download.total_bytes) }}</span>
            <span
              v-if="download.total_bytes != null"
              class="ml-1 text-muted-foreground"
            >
              · {{ t("detail.bytesSuffix", { bytes: Number(download.total_bytes).toLocaleString("en-US") }) }}
            </span>
          </dd>
        </div>
        <div class="grid grid-cols-[88px_1fr] items-baseline gap-3">
          <dt class="text-muted-foreground">{{ t("detail.metricCategory") }}</dt>
          <dd class="text-primary">{{ categoryLine }}</dd>
        </div>
        <div class="grid grid-cols-[88px_1fr] items-baseline gap-3">
          <dt class="text-muted-foreground">{{ t("detail.added") }}</dt>
          <dd class="text-foreground">
            {{ addedAt }}
            <span class="ml-1 text-muted-foreground">· {{ relativeTime(download.created_at) }}</span>
          </dd>
        </div>
        <div class="grid grid-cols-[88px_1fr] items-baseline gap-3">
          <dt class="text-muted-foreground">{{ t("detail.server") }}</dt>
          <dd class="text-foreground">
            {{ serverHost }}
            <span class="ml-1 text-muted-foreground">
              ({{ download.etag ? t("detail.resumableOk") : t("detail.singleStream") }})
            </span>
          </dd>
        </div>
      </dl>
    </section>
  </div>
</template>
