<script setup lang="ts">
// The one toolbar row above the list: page title, status tabs, and the
// total-speed block (readout, sparkline, Pause all, speed limit). The
// bulk bar replaces it at the same height while rows are selected.

import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Pause } from "lucide-vue-next";

import SpeedSparkline from "@/components/detail/SpeedSparkline.vue";
import SpeedLimitPopover from "./SpeedLimitPopover.vue";
import { splitSpeed } from "@/lib/format";
import type { StatusTab } from "@/lib/statusTabs";

export interface ToolbarTab {
  id: StatusTab;
  label: string;
  count: number;
}

const props = defineProps<{
  title: string;
  tabs: ToolbarTab[];
  activeTab: StatusTab;
  /** Bytes per second across every running download. */
  speedBps: number;
  /** One aggregate-speed sample per second, oldest first. */
  speedSamples: number[];
  /** Nothing is running or queued — Pause all has nothing to do. */
  idle: boolean;
  /** Narrow window: the page title is kept for screen readers only. */
  hideTitle?: boolean;
  /** Short on width (narrow window or details open): icon-only speed block. */
  dense?: boolean;
}>();

const emit = defineEmits<{
  "update:activeTab": [tab: StatusTab];
  "pause-all": [];
}>();

const { t } = useI18n();

const speed = computed(() => splitSpeed(props.speedBps));

function tabClass(tab: ToolbarTab): string {
  return tab.id === props.activeTab
    ? "text-foreground shadow-[inset_0_-2px_0_hsl(var(--primary))]"
    : "text-muted-foreground hover:text-foreground";
}

function countClass(tab: ToolbarTab): string {
  if (tab.id === "attention") return "bg-dl-fail-soft font-medium text-danger";
  if (tab.id === props.activeTab) return "bg-muted text-foreground";
  return "text-muted-foreground";
}

function pauseAll() {
  if (!props.idle) emit("pause-all");
}
</script>

<template>
  <div
    class="flex h-14 shrink-0 items-center gap-4 border-b border-dl-line pl-7 pr-5"
  >
    <h1
      class="mr-1 shrink-0 whitespace-nowrap text-base font-semibold tracking-[-0.01em]"
      :class="{ 'sr-only': hideTitle }"
    >
      {{ title }}
    </h1>

    <div
      role="tablist"
      :aria-label="t('downloads.tabsAria')"
      class="flex min-w-0 gap-0.5 self-stretch overflow-x-auto overflow-y-hidden"
    >
      <button
        v-for="tab in tabs"
        :key="tab.id"
        type="button"
        role="tab"
        :aria-selected="tab.id === activeTab"
        class="flex h-full shrink-0 items-center gap-1.5 whitespace-nowrap px-2.5 font-medium transition-colors focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
        :class="tabClass(tab)"
        @click="emit('update:activeTab', tab.id)"
      >
        {{ tab.label }}
        <span class="rounded-md px-1.5 py-px font-mono text-[11px]" :class="countClass(tab)">
          {{ tab.count }}
        </span>
      </button>
    </div>

    <section
      :aria-label="t('downloads.totalSpeed')"
      class="ml-auto flex shrink-0 items-center gap-3"
    >
      <span v-if="!dense" class="text-xs text-muted-foreground">
        {{ t("downloads.totalSpeed") }}
      </span>
      <span class="whitespace-nowrap font-mono text-[17px] font-medium tracking-[-0.02em]">
        {{ speed.value }}<span class="text-xs text-muted-foreground">{{ speed.unit }}</span>
      </span>
      <div
        v-if="!dense && !idle"
        role="img"
        :aria-label="t('downloads.sparkAria')"
        class="w-[72px]"
      >
        <SpeedSparkline :samples="speedSamples" :height="24" />
      </div>
      <button
        type="button"
        class="flex h-9 items-center gap-1.5 rounded-[9px] border border-border bg-dl-raised font-medium text-foreground transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
        :class="[
          dense ? 'w-9 justify-center' : 'px-3',
          idle ? 'cursor-default opacity-45' : 'hover:bg-accent',
        ]"
        :aria-label="t('downloads.pauseAll')"
        :aria-disabled="idle"
        :title="t('downloads.pauseAll')"
        @click="pauseAll"
      >
        <Pause class="h-3.5 w-3.5" aria-hidden="true" />
        <span v-if="!dense">{{ t("downloads.pauseAll") }}</span>
      </button>
      <SpeedLimitPopover :icon-only="dense" />
    </section>
  </div>
</template>
