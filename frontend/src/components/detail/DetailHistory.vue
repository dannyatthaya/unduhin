<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Check, Copy } from "lucide-vue-next";

import { useDownloadsStore, type TimelineEntry } from "@/stores/downloads";
import { useToast } from "@/composables/useToast";
import { useElapsedSeconds } from "@/composables/useElapsed";
import { relativeTime } from "@/lib/format";
import type { DownloadRecord } from "@/types/tauri-bindings";

const { t } = useI18n();
const toast = useToast();

const props = defineProps<{ download: DownloadRecord }>();

const store = useDownloadsStore();

const entries = computed<TimelineEntry[]>(() => store.timelineFor(props.download.id));

const stats = computed(() => {
  const list = entries.value;
  const failures = list.filter((e) => e.kind === "failed").length;
  const retries = list.filter((e) => e.kind === "retry").length;
  return { total: list.length, failures, retries };
});

function dotClass(e: TimelineEntry): string {
  switch (e.kind) {
    case "completed":
    case "retry":
      return "bg-success";
    case "failed":
      return "bg-danger";
    case "started":
      return "bg-muted-foreground";
    default:
      return "bg-primary";
  }
}

function timeLabel(iso: string): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "—";
  const d = new Date(t);
  const month = d.toLocaleString("en-US", { month: "short" });
  const day = d.getDate();
  const year = d.getFullYear();
  const hh = d.getHours().toString().padStart(2, "0");
  const mm = d.getMinutes().toString().padStart(2, "0");
  const ss = d.getSeconds().toString().padStart(2, "0");
  return `${month} ${day}, ${year} · ${hh}:${mm}:${ss}`;
}

function shortTime(iso: string): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "—";
  const d = new Date(t);
  const hh = d.getHours().toString().padStart(2, "0");
  const mm = d.getMinutes().toString().padStart(2, "0");
  const ss = d.getSeconds().toString().padStart(2, "0");
  return `${hh}:${mm}:${ss}`;
}

const elapsedSeconds = useElapsedSeconds(() => props.download);
const elapsedLabel = computed(() => {
  const s = elapsedSeconds.value;
  if (s == null) return "—";
  const mm = Math.floor(s / 60).toString().padStart(2, "0");
  const ss = Math.floor(s % 60).toString().padStart(2, "0");
  return `${mm}:${ss}`;
});

const startedLabel = computed(() => {
  const t = Date.parse(props.download.created_at);
  if (Number.isNaN(t)) return "—";
  const d = new Date(t);
  const hh = d.getHours().toString().padStart(2, "0");
  const mm = d.getMinutes().toString().padStart(2, "0");
  const ss = d.getSeconds().toString().padStart(2, "0");
  return `${hh}:${mm}:${ss}`;
});

async function copyLog() {
  // Plain-text timeline, one event per line. No URL or other metadata —
  // the user is copying the *log*, not the download identity.
  const lines = entries.value.map((e) => {
    const detail = e.detail ? ` — ${e.detail}` : "";
    return `[${timeLabel(e.at)}] ${e.title}${detail}`;
  });
  try {
    await navigator.clipboard.writeText(`${lines.join("\n")}\n`);
    toast.push(t("detail.copiedLogEntries", { n: lines.length }, lines.length), "success");
  } catch (err) {
    console.error("clipboard write failed", err);
    toast.push(t("detail.copyFailed"), "error");
  }
}
</script>

<template>
  <div class="flex h-full flex-col">
    <template v-if="entries.length > 0">
      <!-- Stat pills -->
      <div class="flex flex-wrap gap-1.5">
        <span class="rounded-full border border-border bg-card px-2.5 py-0.5 text-xs font-medium text-foreground">
          {{ t("detail.historyEvents", { n: stats.total }, stats.total) }}
        </span>
        <span
          v-if="stats.failures > 0"
          class="rounded-full border border-danger/30 bg-danger/10 px-2.5 py-0.5 text-xs font-medium text-danger"
        >
          {{ t("detail.historyFailures", { n: stats.failures }, stats.failures) }}
        </span>
        <span
          v-if="stats.retries > 0"
          class="rounded-full border border-success/30 bg-success/10 px-2.5 py-0.5 text-xs font-medium text-success"
        >
          {{ t("detail.historyRetriesSucceeded", { n: stats.retries }) }}
        </span>
      </div>

      <h3 class="mt-5 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
        {{ t("detail.timeline") }}
      </h3>
      <ol class="mt-2 space-y-3">
        <li
          v-for="(e, i) in entries"
          :key="`${e.at}-${i}`"
          class="flex gap-3"
        >
          <span class="relative mt-1.5 flex h-2.5 w-2.5 shrink-0 items-center justify-center">
            <span class="absolute inset-0 rounded-full" :class="dotClass(e)" />
          </span>
          <div class="min-w-0 flex-1">
            <p class="text-sm font-medium text-foreground">{{ e.title }}</p>
            <p class="text-xs text-muted-foreground">
              {{ timeLabel(e.at) }}
              <span v-if="e.detail" class="block sm:inline">
                <span class="hidden sm:inline"> · </span>{{ e.detail }}
              </span>
            </p>
          </div>
          <span class="shrink-0 font-mono text-[11px] text-muted-foreground">
            {{ shortTime(e.at) }}
          </span>
        </li>
      </ol>

      <div class="mt-6 rounded-lg border border-border bg-card p-3">
        <div class="flex items-center justify-between gap-3">
          <div>
            <p class="text-sm font-medium text-foreground">{{ t("detail.saveTimeline") }}</p>
            <p class="text-xs text-muted-foreground">{{ t("detail.saveTimelineHint") }}</p>
          </div>
          <button
            type="button"
            class="inline-flex h-8 items-center gap-1.5 rounded-md border border-border bg-card px-3 text-xs font-medium text-foreground transition-colors hover:bg-accent"
            @click="copyLog"
          >
            <Copy class="h-3.5 w-3.5" />
            {{ t("detail.copyLog") }}
          </button>
        </div>
      </div>
    </template>

    <!-- Empty state -->
    <template v-else>
      <div class="flex flex-1 flex-col items-center justify-center px-6 py-12 text-center">
        <span class="flex h-14 w-14 items-center justify-center rounded-full bg-success/15 text-success">
          <Check class="h-6 w-6" />
        </span>
        <p class="mt-4 text-base font-semibold text-foreground">
          {{ t("detail.historyEmptyTitle") }}
        </p>
        <p class="mt-2 max-w-xs text-sm text-muted-foreground">
          {{ t("detail.historyEmptyBody") }}
        </p>
        <p class="mt-6 text-xs text-muted-foreground">
          {{ t("detail.historyEmptyFooter", { started: startedLabel, elapsed: elapsedLabel }) }}
        </p>
      </div>
    </template>
  </div>
</template>
