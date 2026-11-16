<script setup lang="ts">
// History tab: what happened to this download, oldest first, on a rail.
// The entries are collected while the app runs, so a download from an
// earlier session starts with an empty history.

import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Copy } from "lucide-vue-next";

import Button from "@/components/ui/Button.vue";

import { useDownloadsStore, type TimelineEntry } from "@/stores/downloads";
import { useToast } from "@/composables/useToast";
import { timelineTone, useTimelineText, type TimelineTone } from "@/composables/useTimelineText";
import type { DownloadRecord } from "@/types/tauri-bindings";

const { t } = useI18n();
const toast = useToast();
const { describe } = useTimelineText();

const props = defineProps<{ download: DownloadRecord }>();

const store = useDownloadsStore();

interface Row {
  key: string;
  title: string;
  detail: string;
  time: string;
  tone: TimelineTone;
}

function pad(n: number): string {
  return n.toString().padStart(2, "0");
}

/** `17:53:27` — the rail shows the time only; the day is in "Added". */
function clockTime(iso: string): string {
  const ms = Date.parse(iso);
  if (Number.isNaN(ms)) return t("common.dash");
  const d = new Date(ms);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

const entries = computed<TimelineEntry[]>(() => store.timelineFor(props.download.id));

const rows = computed<Row[]>(() =>
  entries.value.map((entry, index) => {
    const text = describe(entry);
    return {
      key: `${entry.at}-${index}`,
      title: text.title,
      detail: text.detail,
      time: clockTime(entry.at),
      tone: timelineTone(entry),
    };
  }),
);

const DOT_TONE: Record<TimelineTone, string> = {
  muted: "border-muted-foreground",
  queue: "border-info",
  accent: "border-primary",
  ok: "border-success",
  fail: "border-danger",
};

async function copyLog() {
  // Plain text, one event per line: the full timestamp, then what happened.
  const lines = entries.value.map((entry) => {
    const text = describe(entry);
    const what = text.detail ? `${text.title}: ${text.detail}` : text.title;
    return `[${entry.at}] ${what}`;
  });
  try {
    await navigator.clipboard.writeText(`${lines.join("\n")}\n`);
    toast.push(t("detail.copiedLogEntries", { n: lines.length }, lines.length), "success");
  } catch {
    toast.push(t("detail.copyFailed"), "error");
  }
}
</script>

<template>
  <div v-if="rows.length > 0" class="flex flex-col gap-5">
    <ol :aria-label="t('detail.timeline')" class="flex flex-col">
      <li
        v-for="(row, index) in rows"
        :key="row.key"
        class="grid grid-cols-[20px_minmax(0,1fr)_auto] items-start gap-x-2.5"
      >
        <span class="flex flex-col items-center self-stretch">
          <span
            class="mt-1 h-2.5 w-2.5 shrink-0 rounded-full border-2 bg-dl-chrome"
            :class="DOT_TONE[row.tone]"
          />
          <span
            class="mt-1 w-0.5 flex-1"
            :class="index === rows.length - 1 ? 'bg-transparent' : 'bg-dl-line'"
          />
        </span>
        <span class="flex min-w-0 flex-col gap-[3px] pb-4">
          <span class="font-medium">{{ row.title }}</span>
          <span
            v-if="row.detail"
            class="break-words text-xs leading-[1.45] text-muted-foreground"
          >
            {{ row.detail }}
          </span>
        </span>
        <span class="pt-px font-mono text-xs text-muted-foreground">{{ row.time }}</span>
      </li>
    </ol>

    <div class="flex items-center gap-3 rounded-xl border border-dl-line px-3.5 py-3">
      <span class="flex min-w-0 flex-1 flex-col gap-0.5">
        <span class="font-medium">{{ t("detail.saveTimeline") }}</span>
        <span class="text-xs leading-[1.45] text-muted-foreground">
          {{ t("detail.saveTimelineHint") }}
        </span>
      </span>
      <Button size="sm" @click="copyLog">
        <Copy class="h-[13px] w-[13px]" aria-hidden="true" />
        {{ t("detail.copyLog") }}
      </Button>
    </div>
  </div>

  <div v-else class="flex flex-col gap-1 py-6">
    <p class="font-medium">{{ t("detail.historyEmptyTitle") }}</p>
    <p class="text-xs leading-[1.45] text-muted-foreground">
      {{ t("detail.historyEmptyBody") }}
    </p>
  </div>
</template>
