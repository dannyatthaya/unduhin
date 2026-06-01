<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import DownloadRow from "./DownloadRow.vue";
import { useDownloadsStore } from "@/stores/downloads";
import { formatSpeed } from "@/lib/format";
import type { DownloadGroup } from "@/composables/useGroupedDownloads";

const { t } = useI18n();
const props = defineProps<{ group: DownloadGroup }>();
const downloads = useDownloadsStore();

const titleTone = computed(() => {
  if (props.group.key === "needs-attention") return "text-danger";
  return "text-muted-foreground";
});

const groupLabel = computed(() => {
  switch (props.group.key) {
    case "active":
      return t("downloads.groupActive");
    case "paused-queued":
      return t("downloads.groupPausedQueued");
    case "needs-attention":
      return t("downloads.groupNeedsAttention");
    case "completed":
      return t("downloads.groupCompletedToday");
    case "completed-yesterday":
      return t("downloads.groupCompletedYesterday");
    case "completed-older":
      return t("downloads.groupCompletedOlder");
    default:
      return props.group.label;
  }
});

const aggregateSpeed = computed(() => {
  if (props.group.key !== "active") return null;
  let s = 0;
  for (const r of props.group.rows) {
    const st = downloads.statsFor(r.id);
    if (st) s += st.speed_bps;
  }
  return s;
});

// Queue position numbering for queued items only.
function queuePosition(id: number): number | null {
  if (props.group.key !== "paused-queued") return null;
  const queued = props.group.rows.filter((r) => r.status === "queued");
  const idx = queued.findIndex((r) => r.id === id);
  return idx < 0 ? null : idx + 1;
}
</script>

<template>
  <section class="space-y-2">
    <header class="flex items-center justify-between px-1">
      <h2
        class="text-xs font-semibold uppercase tracking-wider"
        :class="titleTone"
      >
        {{ groupLabel }}
        <span class="ml-1 text-muted-foreground/80">· {{ group.rows.length }}</span>
      </h2>
      <span
        v-if="aggregateSpeed != null && aggregateSpeed > 0"
        class="text-xs font-medium text-foreground/80"
      >
        {{ formatSpeed(aggregateSpeed) }} ↓
      </span>
    </header>

    <TransitionGroup
      tag="div"
      class="space-y-2"
      enter-active-class="transition-all duration-200 ease-out"
      leave-active-class="transition-all duration-150 ease-in"
      enter-from-class="opacity-0 -translate-y-1"
      leave-to-class="opacity-0"
    >
      <DownloadRow
        v-for="row in group.rows"
        :key="row.id"
        :download="row"
        :queue-position="queuePosition(row.id)"
      />
    </TransitionGroup>
  </section>
</template>
