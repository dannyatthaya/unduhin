<script setup lang="ts">
import { computed, inject } from "vue";
import { useI18n } from "vue-i18n";
import {
  Clock,
  Copy,
  ExternalLink,
  Folder,
  FolderTree,
  MoreHorizontal,
  Pause,
  Pencil,
  Play,
  RotateCw,
  Trash2,
  Users,
} from "lucide-vue-next";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";

import Button from "./ui/Button.vue";
import ExtBadge from "./ExtBadge.vue";
import ProgressBar from "./ProgressBar.vue";
import StatusBadge from "./StatusBadge.vue";
import RowMenu from "./RowMenu.vue";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "./ui/context-menu";

import { useCategoriesStore } from "@/stores/categories";
import { useDetailStore } from "@/stores/detail";
import { useDownloadsStore } from "@/stores/downloads";
import { useSelectionStore } from "@/stores/selection";
import { useDeleteConfirm } from "@/composables/useDeleteConfirm";
import { useToast } from "@/composables/useToast";
import {
  formatBytes,
  formatEta,
  formatSpeed,
  percent,
  shortenPath,
  shortenUrl,
} from "@/lib/format";
import type {
  Category,
  DownloadId,
  DownloadRecord,
} from "@/types/tauri-bindings";

const props = defineProps<{
  download: DownloadRecord;
  queuePosition?: number | null;
}>();

const { t } = useI18n();
const store = useDownloadsStore();
const selection = useSelectionStore();
const detail = useDetailStore();
const categories = useCategoriesStore();
const deleteConfirm = useDeleteConfirm();
const toast = useToast();

async function restart(id: DownloadId) {
  try {
    await store.retry(id);
  } catch (e) {
    // Surface the backend's reason — historically these failed silently
    // when the row's status wasn't a valid source for `retry`.
    const message = e instanceof Error ? e.message : String(e);
    toast.push(t("errors.restartDownload", { error: message }), "error");
  }
}

// ── Selection-aware context-menu actions ────────────────────────────────────
// The right-click menu acts on the WHOLE multi-selection when this row is part
// of one (count > 1); otherwise just this row — same rule as drag-and-drop.
// This replaces the floating batch bar: every batch action lives in the menu.
const targetIds = computed<DownloadId[]>(() =>
  selection.has(props.download.id) && selection.count > 1
    ? Array.from(selection.ids)
    : [props.download.id],
);
const targetRecords = computed<DownloadRecord[]>(() =>
  targetIds.value
    .map((id) => store.records.get(id))
    .filter((r): r is DownloadRecord => r != null),
);
const isBatch = computed(() => targetIds.value.length > 1);

// Each action shows when ANY target qualifies, and runs only on the qualifying
// subset — so a mixed multi-selection still does the sensible thing.
const anyPlayable = computed(() =>
  targetRecords.value.some(
    (r) => r.status === "active" || r.status === "queued",
  ),
);
const anyResumable = computed(() =>
  targetRecords.value.some(
    (r) => r.status === "paused" || r.status === "failed",
  ),
);
// Restart applies to terminal rows: `failed` resumes from the partial,
// `completed` re-downloads from scratch. (`cancelled` kept for defensiveness;
// it's no longer user-creatable.)
const anyRestartable = computed(() =>
  targetRecords.value.some(
    (r) =>
      r.status === "failed" ||
      r.status === "cancelled" ||
      r.status === "completed",
  ),
);

/** Run `action` over every target row matching `filter`, in parallel. */
async function runOnTargets(
  filter: (r: DownloadRecord) => boolean,
  action: (id: DownloadId) => Promise<unknown>,
): Promise<void> {
  await Promise.allSettled(
    targetRecords.value.filter(filter).map((r) => action(r.id)),
  );
}

const ctxPause = () =>
  runOnTargets(
    (r) => r.status === "active" || r.status === "queued",
    (id) => store.pause(id),
  );
const ctxResume = () =>
  runOnTargets(
    (r) => r.status === "paused" || r.status === "failed",
    (id) => store.resume(id),
  );
const ctxRetry = () =>
  runOnTargets(
    (r) =>
      r.status === "failed" ||
      r.status === "cancelled" ||
      r.status === "completed",
    (id) => restart(id),
  );
const ctxDelete = () => deleteConfirm.requestDelete(targetIds.value);
async function ctxMoveTo(target: Category) {
  await runOnTargets(
    (r) => r.category_id !== target.id,
    (id) => store.setCategory(id, target.id),
  );
}

// Parent (DownloadsView) provides the currently visible ordered ids
// so shift-click range-select knows what to walk between. Falls back to
// the single row when the component is rendered in isolation.
const orderedIds = inject<() => DownloadId[]>("orderedIds", () => [
  props.download.id,
]);

const stats = computed(() => store.statsFor(props.download.id));
const isSelected = computed(() => selection.has(props.download.id));
const isOpenInDetail = computed(() => detail.openId === props.download.id);

/** Peers/seeds badge — only for torrent rows that already carry a swarm
 *  snapshot (persisted on the `torrent` blob, re-emitted live by
 *  `swarm_progress`). Null otherwise, so the badge stays hidden for
 *  HTTP/media rows and torrents that haven't attached to the swarm yet. */
const swarm = computed(() =>
  props.download.kind === "torrent"
    ? props.download.torrent?.swarm ?? null
    : null
);

const pct = computed(() =>
  percent(props.download.downloaded_bytes, props.download.total_bytes)
);

const tone = computed(() => {
  switch (props.download.status) {
    case "muxing":
      return "muxing" as const;
    case "paused":
      return "paused" as const;
    case "failed":
      return "failed" as const;
    case "completed":
      return "done" as const;
    default:
      return "active" as const;
  }
});

const outerClass = computed(() => {
  const base =
    "group relative overflow-hidden rounded-lg border bg-card transition-shadow before:absolute before:inset-y-0 before:left-0 before:w-1 before:rounded-l-lg";
  const accent = (() => {
    switch (props.download.status) {
      case "active":
        return "before:bg-primary";
      case "muxing":
        return "before:bg-info";
      case "paused":
        return "before:bg-warning";
      case "queued":
        return "before:bg-muted-foreground/60";
      case "failed":
        return "before:bg-danger";
      case "completed":
        return "before:bg-success";
      default:
        return "before:bg-muted-foreground/60";
    }
  })();
  let frame = "border-border hover:shadow-sm";
  if (isSelected.value) {
    frame = "border-primary ring-1 ring-primary bg-primary/5";
  } else if (isOpenInDetail.value) {
    frame = "border-primary/70 ring-1 ring-primary/30";
  }
  return `${base} ${accent} ${frame} cursor-pointer`;
});

const showProgress = computed(() =>
  ["active", "muxing", "paused", "failed"].includes(props.download.status)
);

const errorLine = computed(() => {
  if (props.download.status !== "failed" || !props.download.error) return null;
  return `${props.download.error}`;
});

async function togglePauseResume() {
  if (props.download.status === "active" || props.download.status === "queued") {
    await store.pause(props.download.id);
  } else if (
    props.download.status === "paused" ||
    props.download.status === "failed"
  ) {
    await store.resume(props.download.id);
  }
}

async function openFile() {
  try {
    await openPath(props.download.output_path);
  } catch (e) {
    console.error("openPath failed", e);
  }
}

async function openFolder() {
  try {
    await revealItemInDir(props.download.output_path);
  } catch (e) {
    console.error("revealItemInDir failed", e);
  }
}

function copyUrl() {
  void navigator.clipboard.writeText(props.download.url);
}

function openSourceUrl() {
  void openPath(props.download.url).catch((e) =>
    console.error("openPath(url) failed", e)
  );
}

function onRowClick(e: MouseEvent) {
  if (e.shiftKey) {
    e.preventDefault();
    selection.extendTo(props.download.id, orderedIds());
    return;
  }
  if (e.ctrlKey || e.metaKey) {
    e.preventDefault();
    selection.toggle(props.download.id);
    return;
  }
  // Plain click only opens the detail drawer. Multi-select is driven
  // by the row's checkbox (or the Ctrl/Shift shortcuts above), so we
  // intentionally leave `selection` alone here — otherwise a plain
  // click while inspecting another row would tear down whatever the
  // user was building in the batch bar.
  detail.open(props.download.id);
}

function onCheckboxClick(e: MouseEvent) {
  // The checkbox is its own selection affordance — don't let the click
  // bubble to the row body and reopen the detail drawer.
  e.stopPropagation();
  if (e.shiftKey) {
    selection.extendTo(props.download.id, orderedIds());
    return;
  }
  selection.toggle(props.download.id);
}

// HTML5 drag-source. If the row is part of an active multi-select we
// ship the whole set of ids (so the sidebar drop-target can recategorize
// the batch); otherwise we ship just this row's id. Format is plain
// `application/x-unduhin-ids` carrying a JSON array of numbers — matches
// the AppSidebar drop handler.
function onDragStart(e: DragEvent) {
  if (!e.dataTransfer) return;
  const id = props.download.id;
  const ids =
    selection.has(id) && selection.count > 1
      ? Array.from(selection.ids)
      : [id];
  e.dataTransfer.effectAllowed = "move";
  e.dataTransfer.setData("application/x-unduhin-ids", JSON.stringify(ids));
  // Plain text fallback (browsers refuse a drag with no compatible type
  // in some scenarios). The actual handler ignores it.
  e.dataTransfer.setData("text/plain", String(id));
}

const isRenamable = computed(
  () =>
    props.download.status !== "active" && props.download.status !== "muxing"
);

function renameFile() {
  console.info(`Rename ${props.download.id} (todo)`);
}

const menuItems = computed(() => {
  const items: { label: string; danger?: boolean; onSelect: () => void }[] = [];
  switch (props.download.status) {
    case "completed":
      items.push({ label: t("downloads.rowOpenFile"), onSelect: openFile });
      items.push({ label: t("downloads.rowOpenFolder"), onSelect: openFolder });
      // Re-download a finished file from scratch.
      items.push({ label: t("downloads.menuRestart"), onSelect: () => restart(props.download.id) });
      break;
    case "active":
    case "queued":
      items.push({ label: t("downloads.menuPause"), onSelect: () => store.pause(props.download.id) });
      break;
    // `muxing` has no specific action — you can't pause mid-merge, and there's
    // no cancel; the Remove item appended below is the only way to stop it.
    case "paused":
      items.push({ label: t("downloads.menuResume"), onSelect: () => store.resume(props.download.id) });
      break;
    case "failed":
      items.push({ label: t("downloads.batchRetry"), onSelect: () => restart(props.download.id) });
      break;
    case "cancelled":
      items.push({ label: t("downloads.batchRetry"), onSelect: () => restart(props.download.id) });
      break;
  }
  items.push({
    label: t("downloads.menuRemoveFromList"),
    danger: true,
    onSelect: () => deleteConfirm.requestDelete([props.download.id]),
  });
  return items;
});
</script>

<template>
  <ContextMenu>
    <ContextMenuTrigger as-child>
      <article
        :class="outerClass"
        draggable="true"
        @click="onRowClick"
        @dragstart="onDragStart"
      >
    <div class="flex items-start gap-3 px-4 py-3">
      <label
        class="mt-0.5 inline-flex h-5 w-5 shrink-0 cursor-pointer items-center justify-center"
        :title="isSelected ? t('downloads.deselect') : t('downloads.select')"
        @click.stop
      >
        <input
          type="checkbox"
          class="h-4 w-4 cursor-pointer rounded border-border accent-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          :checked="isSelected"
          :aria-label="t('downloads.selectAria', { filename: download.filename })"
          @click="onCheckboxClick"
        />
      </label>
      <ExtBadge :filename="download.filename" />

      <div class="min-w-0 flex-1">
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
          <h3 class="truncate text-sm font-semibold text-foreground">
            {{ download.filename }}
          </h3>
          <StatusBadge :status="download.status" :queue-position="queuePosition" />
          <span
            v-if="swarm"
            class="inline-flex items-center gap-1 rounded-md bg-info/10 px-1.5 py-0.5 text-[11px] font-medium text-info"
            :title="t('downloads.torrentSwarmBadgeTitle', { peers: swarm.peers, seeds: swarm.seeds })"
          >
            <Users class="h-3 w-3" aria-hidden />
            {{ t("downloads.torrentSwarmBadge", { peers: swarm.peers, seeds: swarm.seeds }) }}
          </span>
        </div>

        <div
          v-if="download.status === 'active' || download.status === 'muxing' || download.status === 'paused'"
          class="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-muted-foreground"
        >
          <span class="font-medium text-foreground/80">
            {{ formatBytes(download.total_bytes) }}
          </span>
          <span v-if="download.status === 'active' && stats" class="font-medium text-foreground/80">
            {{ formatSpeed(stats.speed_bps) }}
          </span>
          <span v-if="download.status === 'active' && stats">
            {{ t("detail.metricEta") }} <span class="font-medium text-foreground/80">{{ formatEta(stats.eta) }}</span>
          </span>
          <span v-else-if="download.status === 'muxing'" class="font-medium text-info">
            {{ t("downloads.mergingMedia") }}
          </span>
          <span v-else-if="download.status === 'paused'">
            {{ t("downloads.bytesDownloaded", { bytes: formatBytes(download.downloaded_bytes) }) }}
          </span>
          <span class="truncate">{{ shortenUrl(download.url) }}</span>
        </div>

        <p
          v-else-if="download.status === 'failed'"
          class="mt-1 text-xs text-danger"
        >
          {{ errorLine }}
        </p>

        <p
          v-else-if="download.status === 'queued'"
          class="mt-1 text-xs text-muted-foreground"
        >
          {{
            download.downloaded_bytes > 0
              ? t("downloads.queuedSoFar", { bytes: formatBytes(download.downloaded_bytes), url: shortenUrl(download.url) })
              : t("downloads.queuedTotal", { bytes: formatBytes(download.total_bytes), url: shortenUrl(download.url) })
          }}
          <span class="block">{{ t("downloads.willStartWhenSlotFrees") }}</span>
        </p>

        <div
          v-else-if="download.status === 'completed'"
          class="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-muted-foreground"
        >
          <span class="font-medium text-foreground/80">
            {{ formatBytes(download.total_bytes ?? download.downloaded_bytes) }}
          </span>
          <span class="truncate font-mono">{{ shortenPath(download.output_path) }}</span>
        </div>

        <ProgressBar v-if="showProgress" :value="pct" :tone="tone" class="mt-2" />
      </div>

      <div class="flex shrink-0 items-center gap-1" @click.stop>
        <template v-if="download.status === 'failed'">
          <Button size="sm" variant="secondary" @click="restart(download.id)">
            {{ t("downloads.retryNow") }}
          </Button>
        </template>
        <template v-else-if="download.status === 'completed'">
          <Button size="icon" variant="ghost" :title="t('downloads.rowOpenFile')" @click="openFile">
            <ExternalLink class="h-4 w-4" />
          </Button>
          <Button size="icon" variant="ghost" :title="t('downloads.rowOpenFolder')" @click="openFolder">
            <Folder class="h-4 w-4" />
          </Button>
        </template>
        <template
          v-else-if="
            download.status !== 'cancelled' && download.status !== 'muxing'
          "
        >
          <Button
            size="icon"
            variant="ghost"
            :title="download.status === 'paused' || download.status === 'queued' ? t('downloads.rowResume') : t('downloads.rowPause')"
            @click="togglePauseResume"
          >
            <Pause v-if="download.status === 'active'" class="h-4 w-4" />
            <Play v-else class="h-4 w-4" />
          </Button>
        </template>
        <RowMenu :items="menuItems">
          <MoreHorizontal class="h-4 w-4" />
        </RowMenu>
      </div>
    </div>
      </article>
    </ContextMenuTrigger>

    <ContextMenuContent class="min-w-[14rem]">
      <ContextMenuItem v-if="anyPlayable" @select="ctxPause">
        <Pause class="h-3.5 w-3.5" />
        <span>{{ t("downloads.menuPause") }}</span>
        <ContextMenuShortcut>Space</ContextMenuShortcut>
      </ContextMenuItem>
      <ContextMenuItem v-else-if="anyResumable" @select="ctxResume">
        <Play class="h-3.5 w-3.5" />
        <span>{{ t("downloads.menuResume") }}</span>
        <ContextMenuShortcut>Space</ContextMenuShortcut>
      </ContextMenuItem>

      <ContextMenuItem :disabled="!anyRestartable" @select="ctxRetry">
        <RotateCw class="h-3.5 w-3.5" />
        <span>{{ t("downloads.menuRestart") }}</span>
      </ContextMenuItem>
      <!-- Single-row-only conveniences (meaningless for a multi-selection). -->
      <template v-if="!isBatch">
        <ContextMenuItem @select="detail.open(download.id, 'history')">
          <Clock class="h-3.5 w-3.5" />
          <span>{{ t("downloads.menuSchedule") }}</span>
        </ContextMenuItem>

        <ContextMenuSeparator />
        <ContextMenuLabel>{{ t("downloads.menuOpen") }}</ContextMenuLabel>
        <ContextMenuItem @select="openFolder">
          <Folder class="h-3.5 w-3.5" />
          <span>{{ t("downloads.menuOpenDestination") }}</span>
          <ContextMenuShortcut>Ctrl+O</ContextMenuShortcut>
        </ContextMenuItem>
        <ContextMenuItem @select="openSourceUrl">
          <ExternalLink class="h-3.5 w-3.5" />
          <span>{{ t("downloads.menuOpenSource") }}</span>
        </ContextMenuItem>
        <ContextMenuItem @select="copyUrl">
          <Copy class="h-3.5 w-3.5" />
          <span>{{ t("downloads.menuCopyUrl") }}</span>
          <ContextMenuShortcut>Ctrl+C</ContextMenuShortcut>
        </ContextMenuItem>
      </template>

      <ContextMenuSeparator />
      <ContextMenuLabel>{{ t("downloads.menuOrganize") }}</ContextMenuLabel>
      <ContextMenuSub>
        <ContextMenuSubTrigger>
          <FolderTree class="h-3.5 w-3.5" />
          <span>{{ t("downloads.menuMoveToCategory") }}</span>
        </ContextMenuSubTrigger>
        <ContextMenuSubContent>
          <ContextMenuItem
            v-for="c in categories.list"
            :key="c.id"
            :disabled="!isBatch && c.id === download.category_id"
            @select="ctxMoveTo(c)"
          >
            {{ c.name }}
          </ContextMenuItem>
        </ContextMenuSubContent>
      </ContextMenuSub>
      <ContextMenuItem
        v-if="!isBatch"
        :disabled="!isRenamable"
        @select="renameFile"
      >
        <Pencil class="h-3.5 w-3.5" />
        <span>{{ t("downloads.menuRename") }}</span>
        <ContextMenuShortcut>F2</ContextMenuShortcut>
      </ContextMenuItem>

      <ContextMenuSeparator />
      <ContextMenuLabel>{{ t("downloads.menuDanger") }}</ContextMenuLabel>
      <ContextMenuItem variant="danger" @select="ctxDelete">
        <Trash2 class="h-3.5 w-3.5" />
        <span>{{ t("downloads.menuDelete") }}</span>
      </ContextMenuItem>
    </ContextMenuContent>
  </ContextMenu>
</template>
