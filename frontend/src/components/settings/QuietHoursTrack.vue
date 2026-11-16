<script setup lang="ts">
// The 24-hour track of the quiet-hours editor: the window drawn as a
// block (two when it runs past midnight), a marker for now, and a handle
// at each end. Handles drag, or move a quarter hour with ← / →.

import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useNow } from "@vueuse/core";

import {
  MINUTES_PER_DAY,
  QUIET_STEP_MINUTES,
  formatTime,
  snapMinutes,
  trackBlocks,
} from "@/lib/quietHours";

type End = "start" | "end";

const props = defineProps<{
  /** Minutes since midnight. */
  start: number;
  end: number;
  disabled?: boolean;
}>();
const emit = defineEmits<{
  /** Fired while dragging, for a live preview. */
  preview: [which: End, minutes: number];
  /** Fired when a handle is released or moved by key. */
  commit: [which: End, minutes: number];
}>();

const { t } = useI18n();

const trackRef = ref<HTMLElement | null>(null);
const dragging = ref<End | null>(null);

const NOW_TICK_MS = 30_000;
const now = useNow({ interval: NOW_TICK_MS });
const nowMinutes = computed(() => now.value.getHours() * 60 + now.value.getMinutes());

const percent = (minutes: number) => `${(minutes / MINUTES_PER_DAY) * 100}%`;

const blocks = computed(() =>
  trackBlocks(props.start, props.end).map((block) => ({
    key: block.left,
    style: { left: `${block.left * 100}%`, width: `${block.width * 100}%` },
    // Square the edge that continues on the other side of midnight.
    rounding: block.left === 0 ? "rounded-l-[7px]" : block.left + block.width >= 1 ? "rounded-r-[7px]" : "",
  })),
);

const HOUR_TICKS = [6, 12, 18];

const handles = computed(() => [
  { which: "start" as End, minutes: props.start, label: t("settings.quietStartHandle") },
  { which: "end" as End, minutes: props.end, label: t("settings.quietEndHandle") },
]);

function minutesAt(clientX: number): number {
  const rect = trackRef.value?.getBoundingClientRect();
  if (!rect || rect.width === 0) return 0;
  const fraction = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
  return snapMinutes(fraction * MINUTES_PER_DAY);
}

function onPointerDown(which: End, e: PointerEvent) {
  if (props.disabled) return;
  dragging.value = which;
  (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
}

function onPointerMove(e: PointerEvent) {
  if (dragging.value) emit("preview", dragging.value, minutesAt(e.clientX));
}

function onPointerUp(e: PointerEvent) {
  if (!dragging.value) return;
  emit("commit", dragging.value, minutesAt(e.clientX));
  dragging.value = null;
}

function onKeydown(which: End, minutes: number, e: KeyboardEvent) {
  if (props.disabled) return;
  const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
  if (step === 0) return;
  e.preventDefault();
  emit("commit", which, snapMinutes(minutes + step * QUIET_STEP_MINUTES));
}
</script>

<template>
  <div class="flex flex-col gap-2">
    <div ref="trackRef" class="relative mx-2 h-7 rounded-[7px] bg-dl-track">
      <span
        v-for="block in blocks"
        :key="block.key"
        aria-hidden="true"
        class="absolute inset-y-0 bg-dl-accent-soft shadow-[inset_0_0_0_1.5px_hsl(var(--primary))]"
        :class="block.rounding"
        :style="block.style"
      />
      <span
        v-for="hour in HOUR_TICKS"
        :key="hour"
        aria-hidden="true"
        class="absolute inset-y-2 w-px bg-input"
        :style="{ left: percent(hour * 60) }"
      />
      <span
        aria-hidden="true"
        class="absolute -inset-y-1 w-0.5 rounded-[1px] bg-foreground"
        :style="{ left: percent(nowMinutes) }"
      />
      <span
        aria-hidden="true"
        class="pointer-events-none absolute top-1.5 whitespace-nowrap font-mono text-xs text-foreground"
        :style="
          nowMinutes > MINUTES_PER_DAY * 0.8
            ? { right: `calc(${100 - (nowMinutes / MINUTES_PER_DAY) * 100}% + 8px)` }
            : { left: `calc(${percent(nowMinutes)} + 8px)` }
        "
      >
        {{ t("settings.quietNow", { time: formatTime(nowMinutes) }) }}
      </span>
      <button
        v-for="handle in handles"
        :key="handle.which"
        type="button"
        role="slider"
        :aria-label="handle.label"
        aria-valuemin="0"
        :aria-valuemax="MINUTES_PER_DAY - QUIET_STEP_MINUTES"
        :aria-valuenow="handle.minutes"
        :aria-valuetext="formatTime(handle.minutes)"
        :aria-disabled="disabled || undefined"
        :title="t('settings.quietHandleHint')"
        class="absolute -top-1.5 flex h-10 w-4 touch-none items-center justify-center rounded-md border-2 border-primary bg-card shadow-[0_1px_3px_rgba(0,0,0,0.25)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
        :class="disabled ? 'cursor-not-allowed' : 'cursor-ew-resize'"
        :style="{ left: `calc(${percent(handle.minutes)} - 8px)` }"
        @pointerdown="onPointerDown(handle.which, $event)"
        @pointermove="onPointerMove"
        @pointerup="onPointerUp"
        @pointercancel="dragging = null"
        @keydown="onKeydown(handle.which, handle.minutes, $event)"
      >
        <span
          aria-hidden="true"
          class="h-3.5 w-0.5 -translate-x-0.5 rounded-[1px] bg-primary shadow-[4px_0_0_hsl(var(--primary))]"
        />
      </button>
    </div>
    <div class="flex justify-between font-mono text-xs text-muted-foreground" aria-hidden="true">
      <span>00</span><span>06</span><span>12</span><span>18</span><span>24</span>
    </div>
  </div>
</template>
