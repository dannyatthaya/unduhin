<script setup lang="ts">
// Parts tab: the part map, then a plain table. A slow part is striped in
// the map, carries a warning icon and uses the warning colour — not the
// failure red. Connections and "Help slow parts" live on Overview.

import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { AlertTriangle, Info } from "lucide-vue-next";

import PartMap from "./PartMap.vue";

import { useDownloadsStore } from "@/stores/downloads";
import { formatBytes, formatSpeed, hostOf } from "@/lib/format";
import { buildPartRows, firstSlowPart, type PartRow } from "@/lib/partRows";
import type { DownloadRecord } from "@/types/tauri-bindings";

const { t } = useI18n();
const props = defineProps<{ download: DownloadRecord }>();

const store = useDownloadsStore();
const live = computed(() => store.liveSegmentsFor(props.download.id));

/** yt-dlp downloads are one stream from our side: no parts to show. */
const isMedia = computed(() => props.download.media_info != null);
const isFinished = computed(() => props.download.status === "completed");

const rows = computed<PartRow[]>(() =>
  buildPartRows(props.download.segments_meta, live.value, props.download.segments),
);
const slowPart = computed(() => (isFinished.value ? null : firstSlowPart(rows.value)));

/** "8 parts × 150 MB" when the parts are even; otherwise just the count. */
const mapSummary = computed(() => {
  const n = rows.value.length;
  const total = props.download.total_bytes;
  if (total == null || n === 0) return t("detail.partsCount", { n }, n);
  return t("detail.partsBySize", { n, size: formatBytes(Math.round(total / n)) }, n);
});

function rangeLabel(row: PartRow): string {
  if (row.start == null || row.end == null) return t("common.dash");
  return t("detail.partRange", {
    from: formatBytes(row.start, 0),
    to: formatBytes(row.end, 0),
  });
}

function speedLabel(row: PartRow): string {
  if (row.isDone) return t("common.dash");
  return row.speedBps != null && row.speedBps > 0
    ? formatSpeed(row.speedBps)
    : t("common.dash");
}

const GRID =
  "grid grid-cols-[40px_minmax(0,1fr)_56px_92px] items-center gap-x-2.5 px-3";
const SECTION_LABEL = "text-xs font-semibold text-dl-soft";
</script>

<template>
  <!-- Media (yt-dlp): single stream, nothing to split. -->
  <div v-if="isMedia" class="flex flex-col gap-5">
    <div class="flex items-start gap-2.5 rounded-xl border border-dl-line bg-dl-group px-3.5 py-3">
      <Info class="mt-px h-[18px] w-[18px] shrink-0 text-muted-foreground" aria-hidden="true" />
      <span class="flex flex-col gap-1">
        <span class="font-semibold">{{ t("detail.singleStreamTitle") }}</span>
        <span class="text-xs leading-[1.45] text-muted-foreground">
          {{ t("detail.singleStreamBody") }}
        </span>
      </span>
    </div>
    <dl v-if="download.media_info" class="flex flex-col gap-3.5">
      <div class="flex flex-col gap-1">
        <dt class="text-xs text-muted-foreground">{{ t("detail.mediaSource") }}</dt>
        <dd class="font-mono text-xs">{{ download.media_info.extractor }}</dd>
      </div>
      <div class="flex flex-col gap-1">
        <dt class="text-xs text-muted-foreground">{{ t("detail.mediaFormat") }}</dt>
        <dd class="break-all font-mono text-xs">{{ download.media_info.format_selector }}</dd>
      </div>
      <div class="flex flex-col gap-1">
        <dt class="text-xs text-muted-foreground">{{ t("detail.mediaMuxed") }}</dt>
        <dd>
          {{ download.media_info.needs_ffmpeg ? t("detail.mediaMuxedYes") : t("common.no") }}
        </dd>
      </div>
    </dl>
  </div>

  <div v-else class="flex flex-col gap-5">
    <div
      v-if="isFinished && rows.length === 1"
      class="flex items-start gap-2.5 rounded-xl border border-dl-line bg-dl-group px-3.5 py-3"
    >
      <Info class="mt-px h-[18px] w-[18px] shrink-0 text-muted-foreground" aria-hidden="true" />
      <span class="flex flex-col gap-1">
        <span class="font-semibold">{{ t("detail.onePartTitle") }}</span>
        <span class="text-xs leading-[1.45] text-muted-foreground">
          {{ t("detail.onePartBody", { host: hostOf(download.url) }) }}
        </span>
      </span>
    </div>

    <section :aria-label="t('detail.partMap')" class="flex flex-col gap-2.5">
      <div class="flex min-h-6 items-center justify-between gap-2">
        <span :class="SECTION_LABEL">{{ t("detail.partMap") }}</span>
        <span class="font-mono text-xs text-muted-foreground">{{ mapSummary }}</span>
      </div>
      <PartMap :rows="rows" :finished="isFinished" />
    </section>

    <div
      role="table"
      :aria-label="t('detail.tabSegments')"
      class="flex flex-col overflow-hidden rounded-xl border border-dl-line"
    >
      <div
        role="row"
        :class="GRID"
        class="h-8 border-b border-dl-line bg-dl-group text-xs font-semibold text-muted-foreground"
      >
        <span role="columnheader">{{ t("detail.partColumn") }}</span>
        <span role="columnheader">{{ t("detail.segmentTableBytes") }}</span>
        <span role="columnheader" class="text-right">{{ t("detail.partDone") }}</span>
        <span role="columnheader" class="text-right">{{ t("detail.segmentTableSpeed") }}</span>
      </div>
      <div
        v-for="row in rows"
        :key="row.index"
        role="row"
        :class="[GRID, row.isSlow && !isFinished ? 'bg-dl-warn-soft' : '']"
        class="h-8 border-b border-dl-line-soft last:border-b-0"
      >
        <span role="cell" class="font-mono text-xs text-muted-foreground">{{ row.index + 1 }}</span>
        <span role="cell" class="truncate font-mono text-xs">{{ rangeLabel(row) }}</span>
        <span role="cell" class="text-right font-mono text-xs text-dl-soft">{{ row.pct }}%</span>
        <span
          role="cell"
          class="flex items-center justify-end gap-[5px] font-mono text-xs"
          :class="row.isSlow && !isFinished ? 'font-medium text-warning' : ''"
        >
          <AlertTriangle
            v-if="row.isSlow && !isFinished"
            class="h-3 w-3 shrink-0"
            :aria-label="t('detail.legendSlow')"
          />
          {{ speedLabel(row) }}
        </span>
      </div>
    </div>

    <p v-if="slowPart" class="text-xs leading-[1.45] text-muted-foreground">
      {{
        download.help_slow_parts
          ? t("detail.slowPartHelped", { n: slowPart.index + 1 })
          : t("detail.slowPartNotHelped", { n: slowPart.index + 1 })
      }}
    </p>
  </div>
</template>
