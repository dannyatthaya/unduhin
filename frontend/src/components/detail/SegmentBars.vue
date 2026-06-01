<script setup lang="ts">
// Stacked mini-bar visualization of segment progress, matching the
// "SEGMENT HEALTH" strip in the screenshots. One vertical column per
// segment; green for done, amber for slow, blue otherwise. The text
// label inside each bar shows the percentage (omitted for done bars).

import { computed } from "vue";

import type { SegmentState } from "@/types/tauri-bindings";

const props = withDefaults(
  defineProps<{
    segments: SegmentState[];
    slowSegment?: number | null;
    bars?: number;
    height?: number;
  }>(),
  { bars: 8, height: 56 }
);

interface Bar {
  index: number;
  pct: number;
  status: "done" | "slow" | "active";
  label: string;
}

const computedBars = computed<Bar[]>(() => {
  if (props.segments.length > 0) {
    return props.segments.map((s) => {
      const seg = s.segment;
      const total = Math.max(1, seg.end - seg.start);
      const pct = Math.min(100, Math.round((s.bytes_downloaded / total) * 100));
      const status: Bar["status"] =
        pct >= 100
          ? "done"
          : props.slowSegment != null && seg.index === props.slowSegment
          ? "slow"
          : "active";
      return {
        index: seg.index,
        pct,
        status,
        label: pct >= 100 ? "" : `${pct}`,
      };
    });
  }
  // Placeholder bars when we don't have real segments_meta yet.
  return Array.from({ length: props.bars }, (_, i) => ({
    index: i,
    pct: 0,
    status: "active" as const,
    label: "",
  }));
});

function fillClass(b: Bar): string {
  switch (b.status) {
    case "done":
      return "bg-success";
    case "slow":
      return "bg-warning";
    default:
      return "bg-primary";
  }
}

function trackClass(b: Bar): string {
  switch (b.status) {
    case "done":
      return "bg-success/15";
    case "slow":
      return "bg-warning/15";
    default:
      return "bg-primary/15";
  }
}
</script>

<template>
  <div class="grid grid-flow-col gap-1" :style="{ gridAutoColumns: '1fr' }">
    <div
      v-for="bar in computedBars"
      :key="bar.index"
      class="relative overflow-hidden rounded-md"
      :class="trackClass(bar)"
      :style="{ height: `${height}px` }"
      :title="`Segment ${bar.index + 1} · ${bar.pct}%`"
    >
      <div
        class="absolute inset-x-0 bottom-0 transition-all duration-300"
        :class="fillClass(bar)"
        :style="{ height: `${bar.pct}%` }"
      />
      <div
        v-if="bar.label"
        class="absolute inset-0 flex items-center justify-center font-mono text-[11px] font-semibold text-white mix-blend-luminosity"
      >
        {{ bar.label }}
      </div>
    </div>
  </div>
</template>
