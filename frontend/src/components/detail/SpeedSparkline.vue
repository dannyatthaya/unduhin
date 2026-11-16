<script setup lang="ts">
// Speed chart for the details panel: an area under a 2px line, a mid
// grid line and a dashed average. `fill` pads the left with zeros so a
// live chart starts at the left edge and grows rightward; a finished
// download's saved series is drawn edge to edge instead.

import { computed } from "vue";

const VIEW_WIDTH = 150;
const VIEW_HEIGHT = 40;
/** Head-room above the peak so the line never touches the top edge. */
const PEAK_HEADROOM = 1.12;

const props = withDefaults(
  defineProps<{
    samples: number[];
    /** Samples a live chart spans (one per second). */
    capacity?: number;
    height?: number;
    /** Pad to `capacity` with leading zeros (live) or stretch (finished). */
    fill?: boolean;
    tone?: "accent" | "success";
    /** Text alternative describing the chart. */
    label: string;
    /** Caption for the dashed average line. */
    averageLabel?: string;
  }>(),
  { capacity: 60, height: 88, fill: true, tone: "accent" },
);

const series = computed(() => {
  if (!props.fill) return props.samples;
  const recent = props.samples.slice(-props.capacity);
  const padding = Math.max(0, props.capacity - recent.length);
  return [...Array.from({ length: padding }, () => 0), ...recent];
});

const peak = computed(() => series.value.reduce((max, v) => Math.max(max, v), 0));

const average = computed(() => {
  const moving = series.value.filter((v) => v > 0);
  if (moving.length === 0) return 0;
  return moving.reduce((sum, v) => sum + v, 0) / moving.length;
});

const scaleMax = computed(() => Math.max(peak.value * PEAK_HEADROOM, 1));

function yOf(value: number): number {
  return VIEW_HEIGHT - (value / scaleMax.value) * VIEW_HEIGHT;
}

const paths = computed(() => {
  const data = series.value;
  if (data.length === 0) return { area: "", line: "" };
  const stepX = data.length > 1 ? VIEW_WIDTH / (data.length - 1) : VIEW_WIDTH;
  const line = data
    .map((v, i) => `${i === 0 ? "M" : "L"}${(i * stepX).toFixed(2)} ${yOf(v).toFixed(2)}`)
    .join(" ");
  return { line, area: `${line} L${VIEW_WIDTH} ${VIEW_HEIGHT} L0 ${VIEW_HEIGHT} Z` };
});

const averageY = computed(() => yOf(average.value));
/** Where the "avg" caption sits, as a share of the chart's height. */
const averageTop = computed(() => `${(averageY.value / VIEW_HEIGHT) * 100}%`);

const toneClass = computed(() =>
  props.tone === "success" ? "text-success" : "text-primary",
);
</script>

<template>
  <div class="relative">
    <svg
      :viewBox="`0 0 ${VIEW_WIDTH} ${VIEW_HEIGHT}`"
      preserveAspectRatio="none"
      class="block w-full"
      :class="toneClass"
      :style="{ height: `${height}px` }"
      role="img"
      :aria-label="label"
    >
      <path
        :d="`M0 ${VIEW_HEIGHT / 2} L${VIEW_WIDTH} ${VIEW_HEIGHT / 2}`"
        fill="none"
        stroke="var(--dl-line)"
        stroke-width="1"
        vector-effect="non-scaling-stroke"
      />
      <path
        v-if="average > 0"
        :d="`M0 ${averageY.toFixed(2)} L${VIEW_WIDTH} ${averageY.toFixed(2)}`"
        fill="none"
        stroke="hsl(var(--muted-foreground))"
        stroke-width="1"
        stroke-dasharray="3 3"
        vector-effect="non-scaling-stroke"
      />
      <path :d="paths.area" fill="currentColor" fill-opacity="0.16" />
      <path
        :d="paths.line"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linejoin="round"
        vector-effect="non-scaling-stroke"
      />
    </svg>
    <span
      v-if="average > 0 && averageLabel"
      aria-hidden="true"
      class="absolute right-0 -translate-y-1/2 bg-dl-chrome px-[3px] font-mono text-xs text-muted-foreground"
      :style="{ top: averageTop }"
    >
      {{ averageLabel }}
    </span>
  </div>
</template>
