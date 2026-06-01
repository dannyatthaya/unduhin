<script setup lang="ts">
import { computed } from "vue";

type Tone = "active" | "muxing" | "paused" | "failed" | "done";

const props = withDefaults(
  defineProps<{ value: number; tone?: Tone; showPercent?: boolean }>(),
  { tone: "active", showPercent: true }
);

const fillClass = computed(() => {
  switch (props.tone) {
    case "active":
      return "bg-primary";
    case "muxing":
      return "bg-info";
    case "paused":
      return "bg-warning";
    case "failed":
      return "bg-danger";
    case "done":
      return "bg-success";
  }
});

const textClass = computed(() => {
  switch (props.tone) {
    case "active":
      return "text-primary";
    case "muxing":
      return "text-info";
    case "paused":
      return "text-warning";
    case "failed":
      return "text-danger";
    case "done":
      return "text-success";
  }
});

const clamped = computed(() => Math.min(100, Math.max(0, props.value)));
</script>

<template>
  <div class="flex items-center gap-2">
    <div class="relative h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
      <div
        class="h-full rounded-full transition-[width] duration-300 ease-out"
        :class="fillClass"
        :style="{ width: `${clamped}%` }"
      />
    </div>
    <span v-if="showPercent" class="w-10 text-right text-xs font-medium" :class="textClass">
      {{ clamped }}%
    </span>
  </div>
</template>
