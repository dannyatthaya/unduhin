<script setup lang="ts">
// Overview tab of the details panel. The headline progress and the stat
// tiles live in the panel header (DetailPane); this tab carries the speed
// chart, the segments preview, and where the file comes from and goes to.

import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Copy } from "lucide-vue-next";

import SpeedSparkline from "./SpeedSparkline.vue";
import SegmentBars from "./SegmentBars.vue";

import { useCategoriesStore } from "@/stores/categories";
import { useDownloadsStore } from "@/stores/downloads";
import { useToast } from "@/composables/useToast";
import { formatBytes, formatSpeed, relativeTime } from "@/lib/format";
import type { DownloadRecord } from "@/types/tauri-bindings";

const { t, locale } = useI18n();
const props = defineProps<{ download: DownloadRecord }>();
const emit = defineEmits<{ "view-segments": [] }>();

const store = useDownloadsStore();
const categories = useCategoriesStore();
const toast = useToast();

const samples = computed(() => store.speedHistoryFor(props.download.id));
const live = computed(() => store.liveSegmentsFor(props.download.id));

const isMedia = computed(() => props.download.media_info != null);

/** Torrent rows have no HTTP segments — the per-segment preview + "view all
 *  segments" affordance don't apply. The detail pane offers a Torrent tab
 *  instead, so the overview just hides the segments preview here. */
const isTorrent = computed(() => props.download.kind === "torrent");
const showSegmentsPreview = computed(() => !isMedia.value && !isTorrent.value);

const peakSpeed = computed(() => {
  let m = 0;
  for (const v of samples.value) if (v > m) m = v;
  return m;
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

const categoryName = computed(() => categories.nameOf(props.download.category_id));

const addedAt = computed(() => {
  const ms = Date.parse(props.download.created_at);
  if (Number.isNaN(ms)) return "—";
  return new Date(ms).toLocaleString(locale.value, {
    day: "numeric",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
});

const sizeLine = computed(() => {
  const bytes = props.download.total_bytes;
  if (bytes == null) return formatBytes(bytes);
  const exact = t("detail.bytesSuffix", {
    bytes: Number(bytes).toLocaleString("en-US"),
  });
  return `${formatBytes(bytes)} · ${exact}`;
});

async function copyLink() {
  try {
    await navigator.clipboard.writeText(props.download.url);
    toast.push(t("downloads.linksCopied", { n: 1 }, 1), "success");
  } catch {
    toast.push(t("detail.copyFailed"), "error");
  }
}
</script>

<template>
  <div class="flex flex-col gap-[18px]">
    <section class="flex flex-col gap-2">
      <div class="flex items-baseline justify-between text-xs text-muted-foreground">
        <h3>{{ t("detail.speedLast60s") }}</h3>
        <span class="font-mono">
          {{ t("detail.peakShort") }}
          <span class="text-foreground">{{ formatSpeed(peakSpeed) }}</span>
        </span>
      </div>
      <SpeedSparkline :samples="samples" :height="64" />
    </section>

    <!-- Segments preview — hidden for yt-dlp (single-stream) and torrent
         downloads (no HTTP segments; use the Torrent tab instead). -->
    <section v-if="showSegmentsPreview" class="flex flex-col gap-2">
      <header class="flex items-baseline justify-between text-xs">
        <h3 class="text-muted-foreground">{{ t("detail.tabSegments") }}</h3>
        <button
          type="button"
          class="font-medium text-primary hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
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
      <p v-if="slowLabel" class="flex items-start gap-1.5 text-xs text-warning">
        <span aria-hidden="true">⚠</span>
        <span>{{ slowLabel }}</span>
      </p>
    </section>

    <section class="flex flex-col gap-1.5">
      <h3 class="text-xs text-muted-foreground">{{ t("detail.sourceLink") }}</h3>
      <div
        class="flex items-center gap-2 rounded-[9px] border border-input bg-dl-field py-2 pl-2.5 pr-2"
      >
        <span class="min-w-0 flex-1 truncate font-mono text-xs" :title="download.url">
          {{ download.url }}
        </span>
        <button
          type="button"
          class="flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          :aria-label="t('detail.copyLink')"
          :title="t('detail.copyLink')"
          @click="copyLink"
        >
          <Copy class="h-3.5 w-3.5" aria-hidden="true" />
        </button>
      </div>
    </section>

    <section class="flex flex-col gap-1.5">
      <h3 class="text-xs text-muted-foreground">{{ t("detail.savingTo") }}</h3>
      <span class="break-all font-mono text-xs">{{ download.output_path }}</span>
    </section>

    <dl class="grid grid-cols-2 gap-x-4 gap-y-[18px]">
      <div class="flex flex-col gap-1.5">
        <dt class="text-xs text-muted-foreground">{{ t("detail.metricSize") }}</dt>
        <dd>{{ sizeLine }}</dd>
      </div>
      <div class="flex flex-col gap-1.5">
        <dt class="text-xs text-muted-foreground">{{ t("detail.metricCategory") }}</dt>
        <dd>{{ categoryName }}</dd>
      </div>
      <div class="col-span-2 flex flex-col gap-1.5">
        <dt class="text-xs text-muted-foreground">{{ t("detail.added") }}</dt>
        <dd>
          {{ addedAt }}
          <span class="text-muted-foreground">· {{ relativeTime(download.created_at) }}</span>
        </dd>
      </div>
    </dl>
  </div>
</template>
