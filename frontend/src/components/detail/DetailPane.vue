<script setup lang="ts">
// Details panel for one download. Docks to the right of the list (420px)
// rather than floating over it: the list stays usable and simply drops
// its Size and Added columns while the panel is open. Esc or ✕ closes it
// (Esc is routed by DownloadsView, after "clear selection").

import { computed, ref, type Component } from "vue";
import { useI18n } from "vue-i18n";
import { ExternalLink, Folder, MoreHorizontal, Pause, Play, RotateCw, X } from "lucide-vue-next";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";

import DetailOverview from "./DetailOverview.vue";
import DetailSegments from "./DetailSegments.vue";
import DetailTorrent from "./DetailTorrent.vue";
import DetailHistory from "./DetailHistory.vue";
import KindBadge from "@/components/downloads/KindBadge.vue";
import RowMenu from "@/components/RowMenu.vue";
import ScheduleDialog from "@/components/ScheduleDialog.vue";

import { useDetailStore, type DetailTab } from "@/stores/detail";
import { useDownloadsStore } from "@/stores/downloads";
import { useElapsedSeconds } from "@/composables/useElapsed";
import { useToast } from "@/composables/useToast";
import { useDeleteConfirm } from "@/composables/useDeleteConfirm";
import { formatBytes, formatEta, formatSpeed, hostOf, percent } from "@/lib/format";
import type { DownloadRecord } from "@/types/tauri-bindings";

const { t } = useI18n();
const toast = useToast();

const detail = useDetailStore();
const downloads = useDownloadsStore();
const deleteConfirm = useDeleteConfirm();

const record = computed<DownloadRecord | null>(() => {
  const id = detail.openId;
  return id == null ? null : downloads.records.get(id) ?? null;
});

const scheduleOpen = ref(false);

const segmentsCount = computed(() => record.value?.segments ?? 0);
const historyCount = computed(() =>
  record.value ? downloads.timelineFor(record.value.id).length : 0
);

/** Torrent rows run the librqbit backend — they have no HTTP segments, so
 *  the Segments tab is replaced by a Torrent tab (peers/seeds/files). */
const isTorrent = computed(() => record.value?.kind === "torrent");
const torrentFileCount = computed(
  () => record.value?.torrent?.files?.length ?? 0
);

const tabs = computed<{ id: DetailTab; label: string; count?: number }[]>(() => [
  { id: "overview", label: t("detail.tabOverview") },
  isTorrent.value
    ? { id: "torrent", label: t("detail.tabTorrent"), count: torrentFileCount.value }
    : { id: "segments", label: t("detail.tabSegments"), count: segmentsCount.value },
  { id: "history", label: t("detail.tabHistory"), count: historyCount.value },
]);

// ── Progress block ──────────────────────────────────────────────────────────

const stats = computed(() =>
  record.value ? downloads.statsFor(record.value.id) : undefined,
);
const pct = computed(() =>
  record.value ? percent(record.value.downloaded_bytes, record.value.total_bytes) : 0,
);
const hasKnownSize = computed(
  () => record.value?.total_bytes != null && record.value.total_bytes > 0,
);

const statusLabel = computed(() => {
  switch (record.value?.status) {
    case "active":
      return t("downloads.groupDownloading");
    case "muxing":
      return t("downloads.statusMuxing");
    case "paused":
      return t("downloads.statusPaused");
    case "queued":
      return t("downloads.statusQueued");
    case "completed":
      return t("downloads.statusDone");
    case "failed":
      return t("downloads.statusFailed");
    case "cancelled":
      return t("downloads.statusCancelled");
    default:
      return "";
  }
});

/** "Downloading · 34%" — the percentage only while it says something. */
const statusLine = computed(() => {
  const status = record.value?.status;
  const partial = status === "active" || status === "paused" || status === "failed";
  return partial && hasKnownSize.value
    ? `${statusLabel.value} · ${pct.value}%`
    : statusLabel.value;
});

const statusTone = computed(() => {
  switch (record.value?.status) {
    case "active":
      return { text: "text-primary", bar: "bg-primary" };
    case "muxing":
    case "queued":
      return { text: "text-info", bar: "bg-info" };
    case "failed":
      return { text: "text-danger", bar: "bg-danger" };
    case "completed":
      return { text: "text-success", bar: "bg-success" };
    default:
      return { text: "text-dl-soft", bar: "bg-muted-foreground" };
  }
});

const barWidth = computed(() =>
  record.value?.status === "completed" ? 100 : pct.value,
);

const timeLeft = computed(() => {
  const eta = stats.value?.eta;
  if (record.value?.status !== "active" || eta == null || eta <= 0) return "";
  return t("detail.timeLeft", { time: formatEta(eta) });
});

const bytesLine = computed(() => {
  const r = record.value;
  if (!r) return "";
  if (r.status === "completed") return formatBytes(r.total_bytes ?? r.downloaded_bytes);
  if (!hasKnownSize.value) return formatBytes(r.downloaded_bytes);
  return t("downloads.rowBytesOf", {
    done: formatBytes(r.downloaded_bytes),
    total: formatBytes(r.total_bytes),
  });
});

// ── Stat tiles ──────────────────────────────────────────────────────────────

const averageSpeed = computed(() => {
  if (!record.value) return 0;
  const samples = downloads.speedHistoryFor(record.value.id).filter((v) => v > 0);
  if (samples.length === 0) return 0;
  return samples.reduce((sum, v) => sum + v, 0) / samples.length;
});

const elapsedSeconds = useElapsedSeconds(() => record.value);

const statTiles = computed<{ label: string; value: string }[]>(() => {
  const r = record.value;
  if (!r) return [];
  const connections = isTorrent.value
    ? { label: t("detail.torrentPeers"), value: String(r.torrent?.swarm?.peers ?? "—") }
    : { label: t("detail.tabSegments"), value: String(r.segments) };
  return [
    { label: t("detail.metricSpeed"), value: formatSpeed(stats.value?.speed_bps) },
    { label: t("detail.metricAverage"), value: formatSpeed(averageSpeed.value) },
    connections,
    {
      label: t("detail.metricElapsedTitle"),
      value: elapsedSeconds.value == null ? "—" : formatEta(elapsedSeconds.value),
    },
  ];
});

// ── Footer actions ──────────────────────────────────────────────────────────

const primaryAction = computed<{ label: string; icon: Component } | null>(() => {
  switch (record.value?.status) {
    case "active":
    case "queued":
    case "muxing":
      return { label: t("detail.actionPause"), icon: Pause };
    case "paused":
      return { label: t("detail.actionResume"), icon: Play };
    case "failed":
    case "cancelled":
      return { label: t("detail.actionRestart"), icon: RotateCw };
    case "completed":
      return { label: t("downloads.rowOpenFile"), icon: ExternalLink };
    default:
      return null;
  }
});

async function runPrimary() {
  const r = record.value;
  if (!r) return;
  try {
    switch (r.status) {
      case "active":
      case "queued":
      case "muxing":
        await downloads.pause(r.id);
        break;
      case "paused":
        await downloads.resume(r.id);
        break;
      case "failed":
      case "cancelled":
        await downloads.retry(r.id);
        break;
      case "completed":
        await openPath(r.output_path);
        break;
    }
  } catch (e) {
    if (r.status === "completed") {
      console.error("openPath failed", e);
      toast.push(t("errors.openFile", { filename: r.filename }), "error");
      return;
    }
    const message = e instanceof Error ? e.message : String(e);
    toast.push(
      r.status === "failed" || r.status === "cancelled"
        ? t("errors.restartDownload", { error: message })
        : message || t("downloads.actionFailed"),
      "error",
    );
  }
}

async function restart(id: DownloadRecord["id"]) {
  try {
    await downloads.retry(id);
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    toast.push(t("errors.restartDownload", { error: message }), "error");
  }
}

async function openFolder() {
  if (!record.value) return;
  try {
    await revealItemInDir(record.value.output_path);
  } catch (e) {
    console.error("revealItemInDir failed", e);
    toast.push(t("errors.revealFile"), "error");
  }
}

const moreMenu = computed(() => {
  const r = record.value;
  if (!r) return [];
  const items: { label: string; danger?: boolean; onSelect: () => void }[] = [
    {
      label: t("downloads.menuSchedule"),
      onSelect: () => {
        scheduleOpen.value = true;
      },
    },
    {
      label: t("downloads.menuCopyUrl"),
      onSelect: () => void navigator.clipboard.writeText(r.url),
    },
  ];
  if (r.status === "completed") {
    // Re-download a finished file from scratch.
    items.push({
      label: t("downloads.menuRestart"),
      onSelect: () => void restart(r.id),
    });
  }
  items.push({
    label: t("common.remove"),
    danger: true,
    onSelect: () => void deleteConfirm.requestDelete([r.id]),
  });
  return items;
});

const FOCUS_RING =
  "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
</script>

<template>
  <aside
    v-if="record"
    :aria-label="t('detail.panelAria', { filename: record.filename })"
    class="flex min-h-0 w-[420px] shrink-0 flex-col border-l border-dl-line bg-dl-chrome"
  >
    <header class="flex items-start gap-3 pb-3.5 pl-5 pr-4 pt-[18px]">
      <KindBadge :filename="record.filename" :download-kind="record.kind" size="lg" />
      <div class="flex min-w-0 flex-1 flex-col gap-[3px]">
        <h2 class="truncate text-[15px] font-semibold" :title="record.filename">
          {{ record.filename }}
        </h2>
        <span class="truncate text-xs text-muted-foreground">
          {{ hostOf(record.url) || t("downloads.kindTorrent") }}
        </span>
      </div>
      <button
        type="button"
        class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :class="FOCUS_RING"
        :aria-label="t('common.close')"
        :title="t('detail.closeHint')"
        @click="detail.close()"
      >
        <X class="h-[15px] w-[15px]" aria-hidden="true" />
      </button>
    </header>

    <div class="flex flex-col gap-2 px-5 pb-4">
      <div class="flex items-baseline justify-between text-xs">
        <span class="font-semibold" :class="statusTone.text">{{ statusLine }}</span>
        <span class="font-mono text-muted-foreground">{{ timeLeft }}</span>
      </div>
      <span class="block h-1.5 rounded-[3px] bg-dl-track">
        <span
          class="block h-1.5 rounded-[3px] transition-[width] duration-300 ease-out"
          :class="statusTone.bar"
          :style="{ width: `${barWidth}%` }"
        />
      </span>
      <span class="font-mono text-xs text-muted-foreground">{{ bytesLine }}</span>
    </div>

    <dl
      class="mx-5 mb-4 grid grid-cols-2 gap-px overflow-hidden rounded-xl border border-dl-line bg-dl-line"
    >
      <div
        v-for="tile in statTiles"
        :key="tile.label"
        class="flex flex-col gap-[3px] bg-card px-3 py-2.5"
      >
        <dt class="text-[11px] uppercase tracking-[0.06em] text-muted-foreground">
          {{ tile.label }}
        </dt>
        <dd class="font-mono text-sm">{{ tile.value }}</dd>
      </div>
    </dl>

    <div
      role="tablist"
      :aria-label="t('detail.tabsAria')"
      class="flex gap-0.5 border-b border-dl-line px-5"
    >
      <button
        v-for="tab in tabs"
        :key="tab.id"
        type="button"
        role="tab"
        :aria-selected="detail.tab === tab.id"
        class="-mb-px flex h-[38px] items-center gap-1.5 px-2.5 font-medium transition-colors focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
        :class="
          detail.tab === tab.id
            ? 'text-foreground shadow-[inset_0_-2px_0_hsl(var(--primary))]'
            : 'text-muted-foreground hover:text-foreground'
        "
        @click="detail.setTab(tab.id)"
      >
        {{ tab.label }}
        <span
          v-if="tab.count"
          class="rounded-md bg-muted px-1.5 py-px font-mono text-[11px] text-dl-soft"
        >
          {{ tab.count }}
        </span>
      </button>
    </div>

    <!-- Body — vertical scroll only, never horizontal. -->
    <div class="min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-5 py-4">
      <DetailOverview
        v-if="detail.tab === 'overview'"
        :download="record"
        @view-segments="detail.setTab(isTorrent ? 'torrent' : 'segments')"
      />
      <DetailTorrent
        v-else-if="isTorrent && detail.tab === 'torrent'"
        :download="record"
      />
      <DetailSegments
        v-else-if="!isTorrent && detail.tab === 'segments'"
        :download="record"
      />
      <DetailHistory
        v-else-if="detail.tab === 'history'"
        :download="record"
      />
      <!-- Fallback when the active tab doesn't apply to this kind (e.g. a
           stale 'segments' tab on a torrent row): show the kind's primary
           secondary view rather than a blank pane. -->
      <DetailTorrent
        v-else-if="isTorrent"
        :download="record"
      />
      <DetailSegments
        v-else
        :download="record"
      />
    </div>

    <footer class="flex shrink-0 items-center gap-2 border-t border-dl-line py-3 pl-5 pr-4">
      <button
        v-if="primaryAction"
        type="button"
        class="flex h-9 items-center gap-1.5 rounded-[9px] border border-border bg-dl-raised px-3.5 font-medium text-foreground transition-colors hover:bg-accent"
        :class="FOCUS_RING"
        @click="runPrimary"
      >
        <component :is="primaryAction.icon" class="h-3.5 w-3.5" aria-hidden="true" />
        {{ primaryAction.label }}
      </button>
      <button
        type="button"
        class="flex h-9 items-center gap-1.5 rounded-[9px] px-3 font-medium text-foreground transition-colors hover:bg-accent"
        :class="FOCUS_RING"
        @click="openFolder"
      >
        <Folder class="h-[15px] w-[15px]" aria-hidden="true" />
        {{ t("downloads.rowShowInFolder") }}
      </button>
      <span class="ml-auto flex">
        <RowMenu :items="moreMenu">
          <MoreHorizontal class="h-[18px] w-[18px]" aria-hidden="true" />
        </RowMenu>
      </span>
    </footer>
  </aside>
  <ScheduleDialog
    v-if="record"
    :open="scheduleOpen"
    :scope="{ kind: 'download', downloadId: record.id }"
    @close="scheduleOpen = false"
  />
</template>
