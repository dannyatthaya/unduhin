<script setup lang="ts">
// 60-second speed sparkline. SVG area chart, themed via currentColor.
// Zero-fills the prefix so the chart starts at the left edge and grows
// rightward as samples accumulate.

import { computed } from "vue";

const props = withDefaults(
  defineProps<{
    samples: number[];
    capacity?: number;
    height?: number;
  }>(),
  { capacity: 60, height: 80 }
);

const padded = computed(() => {
  const n = props.capacity;
  const s = props.samples.slice(-n);
  const pad = Math.max(0, n - s.length);
  const out: number[] = [];
  for (let i = 0; i < pad; i++) out.push(0);
  for (const v of s) out.push(v);
  return out;
});

const peak = computed(() => {
  let m = 0;
  for (const v of padded.value) if (v > m) m = v;
  return m;
});

const avg = computed(() => {
  let total = 0;
  let count = 0;
  for (const v of padded.value) {
    if (v > 0) {
      total += v;
      count += 1;
    }
  }
  return count > 0 ? total / count : 0;
});

const path = computed(() => {
  const data = padded.value;
  if (data.length === 0) return { area: "", line: "" };
  const w = 1000;
  const h = 100;
  const max = Math.max(peak.value, 1);
  const stepX = data.length > 1 ? w / (data.length - 1) : w;

  const points = data.map((v, i) => {
    const x = i * stepX;
    const y = h - (v / max) * h;
    return [x, y] as const;
  });

  const lineD = points
    .map(([x, y], i) => `${i === 0 ? "M" : "L"}${x.toFixed(1)} ${y.toFixed(1)}`)
    .join(" ");

  const areaD = `${lineD} L${w} ${h} L0 ${h} Z`;
  return { area: areaD, line: lineD };
});

defineExpose({ peak, avg });
</script>

<template>
  <svg
    viewBox="0 0 1000 100"
    preserveAspectRatio="none"
    class="block w-full text-primary"
    :style="{ height: `${height}px` }"
    aria-hidden="true"
  >
    <defs>
      <linearGradient :id="`sparkline-fill-${(samples.length || 0)}`" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0%" stop-color="currentColor" stop-opacity="0.28" />
        <stop offset="100%" stop-color="currentColor" stop-opacity="0.02" />
      </linearGradient>
    </defs>
    <path :d="path.area" :fill="`url(#sparkline-fill-${(samples.length || 0)})`" />
    <path
      :d="path.line"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      stroke-linejoin="round"
      stroke-linecap="round"
      vector-effect="non-scaling-stroke"
    />
  </svg>
</template>
