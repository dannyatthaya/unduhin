<script setup lang="ts">
// Top-3 active downloads, with mini progress bars and ETA. The popover
// window itself is borderless / always-on-top and auto-hides when
// focus leaves; toggle and positioning are driven by the Rust tray
// reducer.

import { computed, onMounted, onBeforeUnmount, ref } from "vue";
import { useI18n } from "vue-i18n";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Play, Pause, AlertCircle, ExternalLink } from "lucide-vue-next";

const { t } = useI18n();

import {
  api,
  onCoreEvent,
  type DownloadRecord,
  type CoreEvent,
} from "@/types/tauri-bindings";
import ProgressBar from "@/components/ProgressBar.vue";
import { formatBytes, formatEta, formatSpeed, percent } from "@/lib/format";

interface Stats {
  speed_bps: number;
  eta: number | null;
}

// Local mirrors — we don't share the main window's Pinia store, so this
// view maintains its own minimal slice of the same data. `applyEvent`
// in the main store is overkill for this surface; we only need the
// fields rendered below.
const records = ref<Map<number, DownloadRecord>>(new Map());
const stats = ref<Map<number, Stats>>(new Map());

let unlisten: (() => void) | null = null;
let unlistenFocus: (() => void) | null = null;

const activeRows = computed(() => {
  const rows: DownloadRecord[] = [];
  records.value.forEach((rec) => {
    if (rec.status === "active" || rec.status === "muxing") {
      rows.push(rec);
    }
  });
  rows.sort((a, b) => b.created_at.localeCompare(a.created_at));
  return rows.slice(0, 3);
});

const totalActiveBytes = computed(() => {
  let downloaded = 0;
  let total = 0;
  records.value.forEach((rec) => {
    if (rec.status === "active" || rec.status === "muxing") {
      downloaded += rec.downloaded_bytes ?? 0;
      total += rec.total_bytes ?? 0;
    }
  });
  return { downloaded, total };
});

const pausedCount = computed(() => {
  let n = 0;
  records.value.forEach((rec) => {
    if (rec.status === "paused") n += 1;
  });
  return n;
});

const failedCount = computed(() => {
  let n = 0;
  records.value.forEach((rec) => {
    if (rec.status === "failed") n += 1;
  });
  return n;
});

function statsFor(id: number): Stats | undefined {
  return stats.value.get(id);
}

function rowSubtitle(rec: DownloadRecord): string {
  const s = statsFor(rec.id);
  const speed = formatSpeed(s?.speed_bps ?? null);
  const eta = formatEta(s?.eta ?? null);
  return t("tray.popoverRowSubtitle", { speed, eta });
}

async function pauseAll() {
  try {
    await api.pauseAll();
  } catch {
    // Soft-fail. The user will retry from the main window.
  }
}

async function resumeAll() {
  try {
    await api.resumeAll();
  } catch {
    // ditto
  }
}

function showMainWindow() {
  // The tray reducer also forwards left-click to a toggle, but the
  // popover gets dismissed by focus loss before we can hide it; just
  // surface the main window via the existing route.
  // (The window plugin permission `core:window:allow-show` is in
  // capabilities/default.json — same permission applies here through
  // the popover capability file.)
  import("@tauri-apps/api/webviewWindow").then(async ({ WebviewWindow }) => {
    const main = await WebviewWindow.getByLabel("main");
    if (main) {
      await main.show();
      await main.unminimize();
      await main.setFocus();
    }
    await getCurrentWindow().hide();
  });
}

function applyEvent(event: CoreEvent) {
  switch (event.type) {
    case "download_added":
      records.value.set(event.id, event.snapshot);
      break;
    case "status_changed": {
      const rec = records.value.get(event.id);
      if (rec) {
        rec.status = event.to;
        if (event.to !== "active" && event.to !== "muxing") {
          stats.value.delete(event.id);
        }
      }
      break;
    }
    case "progress_update": {
      const rec = records.value.get(event.id);
      if (rec) {
        rec.downloaded_bytes = event.downloaded;
        if (event.total != null) rec.total_bytes = event.total;
      }
      stats.value.set(event.id, {
        speed_bps: event.speed_bps,
        eta: event.eta,
      });
      break;
    }
    case "completed": {
      const rec = records.value.get(event.id);
      if (rec) rec.status = "completed";
      stats.value.delete(event.id);
      break;
    }
    case "failed": {
      const rec = records.value.get(event.id);
      if (rec) {
        rec.status = "failed";
        rec.error = event.error;
      }
      stats.value.delete(event.id);
      break;
    }
    case "removed":
      records.value.delete(event.id);
      stats.value.delete(event.id);
      break;
    default:
      // Other events (segment_progress, category_changed, etc.) are not
      // visible in the popover — skip to keep the reducer cheap.
      break;
  }
}

onMounted(async () => {
  try {
    const initial = await api.listDownloads();
    initial.forEach((rec) => records.value.set(rec.id, rec));
  } catch {
    // No-op: an empty popover is a fine startup state.
  }

  unlisten = await onCoreEvent(applyEvent);

  // Auto-hide on focus loss — the popover's own dismissal idiom.
  const win = getCurrentWindow();
  unlistenFocus = await win.onFocusChanged(({ payload: focused }) => {
    if (!focused) {
      void win.hide();
    }
  });
});

onBeforeUnmount(() => {
  unlisten?.();
  unlistenFocus?.();
});
</script>

<template>
  <div class="flex h-screen w-screen flex-col gap-2 border border-border bg-card p-3 text-foreground">
    <header class="flex items-center justify-between">
      <div class="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
        {{ t("tray.popoverHeader") }}
      </div>
      <div class="flex items-center gap-1 text-[10px] text-muted-foreground">
        <template v-if="failedCount > 0">
          <AlertCircle class="h-3 w-3 text-danger" />
          <span>{{ t("downloads.statusbarCountFailed", { n: failedCount }) }}</span>
        </template>
        <template v-else-if="pausedCount > 0">
          <Pause class="h-3 w-3 text-warning" />
          <span>{{ t("downloads.statusbarCountPaused", { n: pausedCount }) }}</span>
        </template>
      </div>
    </header>

    <div v-if="activeRows.length === 0" class="flex flex-1 flex-col items-center justify-center gap-1 text-center">
      <p class="text-xs font-medium text-foreground">{{ t("tray.popoverEmptyTitle") }}</p>
      <p class="text-[11px] text-muted-foreground">
        {{ t("tray.popoverEmptyHint") }}
      </p>
    </div>

    <ul v-else class="flex flex-1 flex-col gap-2 overflow-hidden">
      <li v-for="rec in activeRows" :key="rec.id" class="flex flex-col gap-1">
        <div class="flex items-center justify-between gap-2">
          <span class="truncate text-xs font-medium text-foreground" :title="rec.filename">
            {{ rec.filename }}
          </span>
          <span class="shrink-0 text-[10px] tabular-nums text-muted-foreground">
            {{ formatBytes(rec.downloaded_bytes) }}<span v-if="rec.total_bytes"> / {{ formatBytes(rec.total_bytes) }}</span>
          </span>
        </div>
        <ProgressBar
          :value="percent(rec.downloaded_bytes, rec.total_bytes)"
          :tone="rec.status === 'muxing' ? 'muxing' : 'active'"
          :show-percent="false"
        />
        <div class="text-[10px] tabular-nums text-muted-foreground">
          {{ rowSubtitle(rec) }}
        </div>
      </li>
    </ul>

    <footer class="flex items-center justify-between gap-1 border-t border-border pt-2">
      <div class="text-[10px] text-muted-foreground">
        {{ formatBytes(totalActiveBytes.downloaded) }}<span v-if="totalActiveBytes.total > 0">
          {{ t("tray.popoverOfTotal", { total: formatBytes(totalActiveBytes.total) }) }}
        </span>
      </div>
      <div class="flex items-center gap-1">
        <button
          v-if="pausedCount > 0"
          type="button"
          class="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium text-foreground hover:bg-accent"
          @click="resumeAll"
        >
          <Play class="h-3 w-3" />
          {{ t("tray.popoverResume") }}
        </button>
        <button
          v-if="activeRows.length > 0"
          type="button"
          class="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium text-foreground hover:bg-accent"
          @click="pauseAll"
        >
          <Pause class="h-3 w-3" />
          {{ t("tray.popoverPause") }}
        </button>
        <button
          type="button"
          class="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium text-foreground hover:bg-accent"
          @click="showMainWindow"
        >
          <ExternalLink class="h-3 w-3" />
          {{ t("tray.popoverOpen") }}
        </button>
      </div>
    </footer>
  </div>
</template>
