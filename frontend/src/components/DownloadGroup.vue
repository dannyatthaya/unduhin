<script setup lang="ts">
// One collapsible section of the grouped downloads list: a header band
// with the section's count and a one-line summary on the right, then its
// rows. The parent owns which rows are shown (`visibleRows`) so range
// selection and keyboard navigation only ever see rendered rows.

import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { ChevronDown, ChevronRight, RotateCw, Timer } from "lucide-vue-next";

import DownloadRow from "./DownloadRow.vue";
import { useDownloadsStore } from "@/stores/downloads";
import { useSettingsStore } from "@/stores/settings";
import { useToast } from "@/composables/useToast";
import { approxDuration } from "@/lib/format";
import type { DownloadGroup } from "@/composables/useGroupedDownloads";
import type { DownloadRecord } from "@/types/tauri-bindings";

const props = defineProps<{
  group: DownloadGroup;
  /** Rows to render — a prefix of `group.rows` while a long section is cut. */
  visibleRows: DownloadRecord[];
  collapsed: boolean;
  label: string;
  /** Size and Added columns are hidden; keep the summary short. */
  compact: boolean;
}>();

const emit = defineEmits<{ toggle: []; "show-all": [] }>();

const DEFAULT_MAX_CONCURRENT = 4;

const { t } = useI18n();
const downloads = useDownloadsStore();
const settings = useSettingsStore();
const toast = useToast();

const hiddenCount = computed(
  () => props.group.rows.length - props.visibleRows.length,
);

const dotClass = computed(() => {
  switch (props.group.key) {
    case "needs-attention":
      return "bg-danger";
    case "active":
      return "bg-primary";
    case "queued":
      return "bg-info";
    default:
      return "bg-dl-dim";
  }
});

// Downloading: how many of the concurrency slots are in use.
const slots = computed<{ busy: number; max: number } | null>(() => {
  if (props.group.key !== "active" || props.compact) return null;
  const raw = settings.values["max_concurrent_downloads"];
  const max = typeof raw === "number" && raw > 0 ? raw : DEFAULT_MAX_CONCURRENT;
  // The section can briefly hold more rows than slots (merging rows, a
  // limit lowered mid-flight); never draw more busy pips than pips.
  return { busy: Math.min(props.group.rows.length, max), max };
});

// Downloading: when the last of them finishes — only when every row has
// an estimate, otherwise the figure would be a guess.
const etaLabel = computed<string | null>(() => {
  if (props.group.key !== "active") return null;
  let longest = 0;
  for (const row of props.group.rows) {
    const eta = downloads.statsFor(row.id)?.eta;
    if (eta == null || !Number.isFinite(eta) || eta <= 0) return null;
    longest = Math.max(longest, eta);
  }
  const { n, unit } = approxDuration(longest);
  const time = t(`downloads.duration_${unit}`, { n });
  const count = props.group.rows.length;
  return count === 1
    ? t("downloads.groupEtaOne", { time })
    : t("downloads.groupEta", { n: count, time });
});

const note = computed<string | null>(() => {
  if (props.group.key === "needs-attention" && !props.compact) {
    const failed = props.group.rows.filter((r) => r.status === "failed").length;
    const paused = props.group.rows.length - failed;
    return t("downloads.groupAttentionNote", { failed, paused });
  }
  if (props.group.key === "queued") return t("downloads.groupQueuedNote");
  return null;
});

/** Resume every row in the section — paused rows continue, failed rows
 *  pick up from their partial file (the same call the row menu makes). */
async function resumeAll() {
  const settled = await Promise.allSettled(
    props.group.rows.map((r) => downloads.resume(r.id)),
  );
  const failed = settled.filter((s) => s.status === "rejected").length;
  if (failed > 0) {
    toast.push(
      t("downloads.batchFailed", { action: t("downloads.resumeAll"), n: failed }),
      "error",
    );
  }
}

// Position in the section as listed. Only the Queued section numbers rows.
function queuePosition(index: number): number | null {
  return props.group.key === "queued" ? index + 1 : null;
}
</script>

<template>
  <div role="rowgroup">
    <div
      role="row"
      class="flex h-10 items-center gap-3 border-b border-dl-line-soft bg-dl-group px-5"
    >
      <button
        type="button"
        class="flex h-[30px] shrink-0 items-center gap-2 rounded-lg px-2 font-semibold text-foreground transition-colors hover:bg-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
        :aria-expanded="!collapsed"
        @click="emit('toggle')"
      >
        <ChevronRight
          v-if="collapsed"
          class="h-3 w-3 text-muted-foreground"
          aria-hidden="true"
        />
        <ChevronDown v-else class="h-3 w-3 text-muted-foreground" aria-hidden="true" />
        <span class="h-2 w-2 rounded-full" :class="dotClass" aria-hidden="true" />
        <span role="rowheader">{{ label }}</span>
        <span
          class="rounded-[10px] bg-muted px-[7px] py-px font-mono text-[11px] font-medium text-dl-soft"
        >
          {{ group.rows.length }}
        </span>
      </button>

      <div
        class="ml-auto flex min-w-0 items-center gap-[18px] overflow-hidden whitespace-nowrap text-xs text-muted-foreground"
      >
        <span v-if="slots" class="flex items-center gap-2">
          <span class="flex gap-[3px]" aria-hidden="true">
            <span
              v-for="pip in slots.max"
              :key="pip"
              class="h-3.5 w-1.5 rounded-sm"
              :class="pip <= slots.busy ? 'bg-primary' : 'bg-dl-track'"
            />
          </span>
          <span>{{ t("downloads.groupSlots", { busy: slots.busy, max: slots.max }) }}</span>
        </span>
        <span v-if="etaLabel" class="flex items-center gap-1.5">
          <Timer class="h-3.5 w-3.5" aria-hidden="true" />
          {{ etaLabel }}
        </span>
        <span v-if="note" class="truncate">{{ note }}</span>
        <button
          v-if="group.key === 'needs-attention'"
          type="button"
          class="flex h-7 shrink-0 items-center gap-1.5 rounded-[7px] border border-border bg-dl-raised px-2.5 text-xs font-medium text-foreground transition-colors hover:bg-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          @click="resumeAll"
        >
          <RotateCw class="h-[13px] w-[13px]" aria-hidden="true" />
          {{ t("downloads.resumeAll") }}
        </button>
      </div>
    </div>

    <template v-if="!collapsed">
      <DownloadRow
        v-for="(row, index) in visibleRows"
        :key="row.id"
        :download="row"
        :queue-position="queuePosition(index)"
      />
      <div
        v-if="hiddenCount > 0"
        role="row"
        class="dl-grid-row h-10 border-b border-dl-line-soft pl-7 pr-5"
      >
        <span role="gridcell" />
        <span role="gridcell" class="col-[2/-1]">
          <button
            type="button"
            class="flex h-[30px] items-center gap-1.5 rounded-lg px-2.5 text-xs font-medium text-primary transition-colors hover:bg-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
            @click="emit('show-all')"
          >
            <ChevronDown class="h-3 w-3" aria-hidden="true" />
            {{ t("downloads.showAllInGroup", { n: group.rows.length }) }}
          </button>
        </span>
      </div>
    </template>
  </div>
</template>
