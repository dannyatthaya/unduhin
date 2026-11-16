<script setup lang="ts">
// Overview tab. Top to bottom: anything the download is waiting on, its
// speed, the settings it can override, then the facts. The headline
// progress is pinned above the tabs (DetailProgress), so nothing here
// repeats it.

import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import DetailAttention from "./DetailAttention.vue";
import DetailFacts from "./DetailFacts.vue";
import DetailTuning from "./DetailTuning.vue";
import SpeedSparkline from "./SpeedSparkline.vue";

import { useDownloadsStore } from "@/stores/downloads";
import { formatSpeed, splitSpeed } from "@/lib/format";
import type { DownloadRecord } from "@/types/tauri-bindings";

const props = defineProps<{ download: DownloadRecord }>();
defineEmits<{ "edit-schedule": [] }>();

const { t, locale } = useI18n();
const store = useDownloadsStore();

const isCompleted = computed(() => props.download.status === "completed");
const isLive = computed(
  () => props.download.status === "active" || props.download.status === "muxing",
);

/** Live samples while it runs; the saved series once it has finished. */
const samples = computed(() => {
  const live = store.speedHistoryFor(props.download.id);
  if (isCompleted.value) {
    return props.download.speed_samples?.length ? props.download.speed_samples : live;
  }
  return live;
});
const showSpeed = computed(
  () => (isLive.value || isCompleted.value) && samples.value.some((v) => v > 0),
);

const peak = computed(() => samples.value.reduce((max, v) => Math.max(max, v), 0));
const average = computed(() => {
  const moving = samples.value.filter((v) => v > 0);
  return moving.length === 0 ? 0 : moving.reduce((sum, v) => sum + v, 0) / moving.length;
});

/** "Avg 38.2 MB/s, peak 46.0": the unit is said once when both share it. */
const speedSummary = computed(() => {
  const avg = splitSpeed(average.value);
  const top = splitSpeed(peak.value);
  return t("detail.avgPeak", {
    avg: formatSpeed(average.value),
    peak: avg.unit === top.unit ? top.value : formatSpeed(peak.value),
  });
});

const chartLabel = computed(() =>
  t(isCompleted.value ? "detail.speedChartAriaDone" : "detail.speedChartAria", {
    avg: formatSpeed(average.value),
    peak: formatSpeed(peak.value),
  }),
);

function clock(iso: string | null): string {
  const ms = iso ? Date.parse(iso) : Number.NaN;
  if (Number.isNaN(ms)) return "";
  return new Date(ms).toLocaleTimeString(locale.value, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

/** A torrent has no settings of its own; a finished file has none left. */
const showTuning = computed(
  () => props.download.kind !== "torrent" && !isCompleted.value,
);

const attentionRef = ref<InstanceType<typeof DetailAttention> | null>(null);
defineExpose({ focusFreshLink: () => attentionRef.value?.focusFreshLink() });
</script>

<template>
  <div class="flex flex-col gap-5">
    <DetailAttention
      ref="attentionRef"
      :download="download"
      @edit-schedule="$emit('edit-schedule')"
    />

    <section v-if="showSpeed" :aria-label="t('detail.metricSpeed')" class="flex flex-col gap-2">
      <div class="flex min-h-6 items-center justify-between gap-2">
        <span class="text-xs font-semibold text-dl-soft">
          {{ isCompleted ? t("detail.speedWhole") : t("detail.speedLast60s") }}
        </span>
        <span class="whitespace-nowrap font-mono text-xs text-muted-foreground">
          {{ speedSummary }}
        </span>
      </div>
      <SpeedSparkline
        :samples="samples"
        :fill="!isCompleted"
        :height="isCompleted ? 72 : 88"
        :tone="isCompleted ? 'success' : 'accent'"
        :label="chartLabel"
        :average-label="isCompleted ? undefined : t('detail.avgShort')"
      />
      <div class="flex justify-between font-mono text-xs text-muted-foreground">
        <template v-if="isCompleted">
          <span>{{ clock(download.created_at) }}</span>
          <span>{{ clock(download.completed_at) }}</span>
        </template>
        <template v-else>
          <span>{{ t("detail.axisMinus60") }}</span>
          <span>{{ t("detail.axisMinus30") }}</span>
          <span>{{ t("detail.now") }}</span>
        </template>
      </div>
    </section>

    <DetailTuning v-if="showTuning" :download="download" />

    <DetailFacts :download="download" />
  </div>
</template>
