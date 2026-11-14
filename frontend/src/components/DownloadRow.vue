<script setup lang="ts">
import { computed, inject, nextTick, type Component } from "vue";
import { useI18n } from "vue-i18n";
import {
  CircleAlert,
  CircleCheck,
  Clock,
  Copy,
  ExternalLink,
  Folder,
  FolderTree,
  Link,
  MoreHorizontal,
  Pause,
  Pencil,
  Play,
  RotateCw,
  Trash2,
  Users,
} from "lucide-vue-next";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";

import KindBadge from "./downloads/KindBadge.vue";
import { DOWNLOADS_LIST_KEY } from "./downloads/listContext";
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
import { useRefreshLink } from "@/composables/useRefreshLink";
import { useToast } from "@/composables/useToast";
import {
  formatAdded,
  formatBytes,
  formatEta,
  formatSpeed,
  hostOf,
  parentFolderLabel,
  percent,
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

const { t, locale } = useI18n();
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

// DownloadsView provides the rendered ids (for shift-click ranges), the
// compact flag and the roving tab stop. The fallbacks keep the row usable
// when it is rendered on its own.
const list = inject(DOWNLOADS_LIST_KEY, null);
const orderedIds = (): DownloadId[] =>
  list?.orderedIds() ?? [props.download.id];
const compact = computed(() => list?.compact.value ?? false);
const tabIndex = computed(() => {
  if (!list) return 0;
  const active = list.activeRowId.value;
  if (active != null) return active === props.download.id ? 0 : -1;
  return orderedIds()[0] === props.download.id ? 0 : -1;
});

const stats = computed(() => store.statsFor(props.download.id));
const isSelected = computed(() => selection.has(props.download.id));
const isOpenInDetail = computed(() => detail.openId === props.download.id);
/** Selected, or the row the details panel is showing. */
const isLit = computed(() => isSelected.value || isOpenInDetail.value);

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
const hasKnownSize = computed(
  () => props.download.total_bytes != null && props.download.total_bytes > 0,
);

function openRefreshLink() {
  void useRefreshLink().requestRefresh(props.download.id, props.download.filename);
}

/** A link or session that expired. The row offers "Refresh link" for these,
 *  because a plain retry replays the same dead URL and fails identically. */
const isExpiredAuth = computed(
  () => props.download.status === "failed" && props.download.error_kind === "expired_auth"
);

const errorLine = computed(() => {
  if (props.download.status !== "failed" || !props.download.error) return null;
  // A raw "server returned terminal status 403" tells the user nothing they
  // can act on. Say what happened and what fixes it; keep the backend string
  // available in the detail pane for anyone debugging.
  if (isExpiredAuth.value) return t("downloads.errorExpiredLink");
  return `${props.download.error}`;
});

/** Second line under the name: what went wrong, else where it comes from. */
const subLine = computed(() => errorLine.value ?? hostOf(props.download.url));

// ── Status cell ─────────────────────────────────────────────────────────────

interface StatusView {
  label: string;
  /** Text colour of the label (and of the icon, when there is one). */
  tone: string;
  icon: Component | null;
  /** Fill colour of the progress bar; null hides the bar. */
  bar: string | null;
}

const statusView = computed<StatusView>(() => {
  const d = props.download;
  switch (d.status) {
    case "active":
      return {
        label: hasKnownSize.value ? `${pct.value}%` : formatBytes(d.downloaded_bytes),
        tone: "text-primary",
        icon: null,
        bar: "bg-primary",
      };
    case "muxing":
      return {
        label: t("downloads.statusMuxing"),
        tone: "text-info",
        icon: null,
        bar: "bg-info",
      };
    case "queued":
      return {
        label:
          props.queuePosition != null
            ? t("downloads.rowQueuedAt", { position: props.queuePosition })
            : t("downloads.statusQueued"),
        tone: "text-info",
        icon: null,
        bar: null,
      };
    case "paused":
      return {
        label: hasKnownSize.value
          ? t("downloads.rowPausedAt", { pct: pct.value })
          : t("downloads.statusPaused"),
        tone: "text-dl-soft",
        icon: Pause,
        bar: "bg-muted-foreground",
      };
    case "failed":
      return {
        label:
          pct.value > 0
            ? t("downloads.rowFailedAt", { pct: pct.value })
            : t("downloads.statusFailed"),
        tone: "text-danger",
        icon: CircleAlert,
        bar: "bg-danger",
      };
    case "completed":
      return {
        label: t("downloads.statusDone"),
        tone: "text-dl-soft",
        icon: CircleCheck,
        bar: null,
      };
    default:
      return {
        label: t("downloads.statusCancelled"),
        tone: "text-muted-foreground",
        icon: null,
        bar: null,
      };
  }
});

/** Right-aligned figures in the status cell: speed and time left while
 *  running, bytes so far while paused (when there is room). */
const metaLine = computed(() => {
  const d = props.download;
  if (d.status === "active" && stats.value) {
    return `${formatSpeed(stats.value.speed_bps)} · ${formatEta(stats.value.eta)}`;
  }
  if (d.status === "paused" && hasKnownSize.value && !compact.value) {
    return t("downloads.rowBytesOf", {
      done: formatBytes(d.downloaded_bytes),
      total: formatBytes(d.total_bytes),
    });
  }
  return "";
});

const folderLabel = computed(() =>
  props.download.status === "completed"
    ? parentFolderLabel(props.download.output_path)
    : "",
);

const sizeLabel = computed(() => {
  const d = props.download;
  const bytes =
    d.status === "completed" ? d.total_bytes ?? d.downloaded_bytes : d.total_bytes;
  return bytes == null ? null : formatBytes(bytes);
});

const addedLabel = computed(() =>
  formatAdded(props.download.created_at, t("downloads.addedYesterday"), locale.value),
);

// ── Actions ─────────────────────────────────────────────────────────────────

/** Statuses that a pause can stop. `muxing` is included deliberately —
 *  see `Core::pause`: without it a row that reaches the merge phase has
 *  no way out but deletion. */
const PAUSABLE = ["active", "queued", "muxing"] as const;

const isPausable = computed(() =>
  (PAUSABLE as readonly string[]).includes(props.download.status),
);

async function togglePauseResume() {
  const status = props.download.status;
  // The backend rejects an out-of-order transition with
  // `InvalidTransition`. That used to reject silently — an unhandled
  // promise rejection and a button that visibly did nothing.
  try {
    if ((PAUSABLE as readonly string[]).includes(status)) {
      await store.pause(props.download.id);
    } else if (status === "paused" || status === "failed") {
      await store.resume(props.download.id);
    }
  } catch (e: unknown) {
    toast.push(
      (e as { message?: string })?.message ?? t("downloads.actionFailed"),
      "error",
    );
  }
}

interface PrimaryAction {
  label: string;
  icon: Component;
  run: () => void;
}

/** The single icon button on an unfinished row. Finished rows get the
 *  Open + folder pair instead; cancelled rows only have the menu. */
const primaryAction = computed<PrimaryAction | null>(() => {
  switch (props.download.status) {
    case "active":
    case "queued":
    case "muxing":
      return { label: t("downloads.rowPause"), icon: Pause, run: () => void togglePauseResume() };
    case "paused":
      return { label: t("downloads.rowResume"), icon: Play, run: () => void togglePauseResume() };
    case "failed":
      return isExpiredAuth.value
        ? { label: t("downloads.refreshLinkHint"), icon: Link, run: openRefreshLink }
        : { label: t("downloads.rowRetry"), icon: RotateCw, run: () => void restart(props.download.id) };
    default:
      return null;
  }
});

async function openFile() {
  try {
    await openPath(props.download.output_path);
  } catch (e) {
    console.error("openPath failed", e);
    toast.push(t("errors.openFile", { filename: props.download.filename }), "error");
  }
}

async function openFolder() {
  try {
    await revealItemInDir(props.download.output_path);
  } catch (e) {
    console.error("revealItemInDir failed", e);
    toast.push(t("errors.revealFile"), "error");
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
  // Plain click only opens the details panel. Multi-select is driven
  // by the row's checkbox (or the Ctrl/Shift shortcuts above), so we
  // intentionally leave `selection` alone here — otherwise a plain
  // click while inspecting another row would tear down whatever the
  // user was building in the bulk bar.
  detail.open(props.download.id);
}

function onCheckboxClick(e: MouseEvent) {
  // The checkbox is its own selection affordance — don't let the click
  // bubble to the row body and reopen the details panel.
  e.stopPropagation();
  if (e.shiftKey) selection.extendTo(props.download.id, orderedIds());
  else selection.toggle(props.download.id);
  // The browser has already flipped the box. When the selection ends up
  // unchanged for this row (a shift-click inside the range), Vue has nothing
  // to patch, so put the box back in step with the store ourselves.
  const box = e.target as HTMLInputElement;
  void nextTick(() => {
    box.checked = isSelected.value;
  });
}

// Keyboard on the focused row itself (not on a control inside it):
// Space selects, Enter opens details. Arrow keys are handled by the list.
function onRowKeydown(e: KeyboardEvent) {
  if (e.target !== e.currentTarget) return;
  if (e.key === " ") {
    e.preventDefault();
    selection.toggle(props.download.id);
  } else if (e.key === "Enter") {
    e.preventDefault();
    detail.open(props.download.id);
  }
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
    // Pause is offered mid-merge too. It kills the ffmpeg doing the
    // merge and resume re-runs it from the intermediates yt-dlp still
    // has — worth it, because Remove was previously the only way out of
    // a row that got stuck on `muxing`.
    case "muxing":
      items.push({ label: t("downloads.menuPause"), onSelect: () => void togglePauseResume() });
      break;
    case "paused":
      items.push({ label: t("downloads.menuResume"), onSelect: () => void togglePauseResume() });
      break;
    case "failed":
      // Retry is still offered on an expired link — it is harmless, just
      // futile — but Refresh link comes first because it is the one that works.
      if (isExpiredAuth.value) {
        items.push({ label: t("downloads.refreshLink"), onSelect: openRefreshLink });
      }
      items.push({ label: t("downloads.batchRetry"), onSelect: () => restart(props.download.id) });
      break;
    case "cancelled":
      items.push({ label: t("downloads.batchRetry"), onSelect: () => restart(props.download.id) });
      break;
  }
  // Same pair, same meaning as the bulk bar: "Remove from list" never
  // touches the file, "Delete files…" always asks first. Neither follows
  // the default delete action — the right-click "Delete…" item does.
  items.push({
    label: t("downloads.menuRemoveFromList"),
    onSelect: () => void deleteConfirm.removeFromList([props.download.id]),
  });
  items.push({
    label: t("downloads.bulkDeleteFiles"),
    danger: true,
    onSelect: () =>
      void deleteConfirm.requestDelete([props.download.id], { alwaysAsk: true }),
  });
  return items;
});

const FOCUS_RING =
  "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
// Row controls sit quiet until the row is hovered, focused or lit, then
// pick up the raised-button treatment.
const QUIET_BUTTON = `flex h-8 items-center justify-center rounded-lg border transition-colors ${FOCUS_RING}`;
</script>

<template>
  <ContextMenu>
    <ContextMenuTrigger as-child>
      <div
        role="row"
        :tabindex="tabIndex"
        :aria-selected="isSelected"
        :data-row-id="download.id"
        class="group dl-grid-row h-[52px] cursor-default border-b border-dl-line-soft pl-7 pr-5 outline-none focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
        :class="
          isLit
            ? 'bg-dl-selrow shadow-[inset_3px_0_0_hsl(var(--primary))]'
            : 'hover:bg-dl-hover'
        "
        draggable="true"
        @click="onRowClick"
        @keydown="onRowKeydown"
        @focus="list?.setActiveRow(download.id)"
        @dragstart="onDragStart"
      >
        <span role="gridcell" class="flex items-center" @click.stop>
          <input
            type="checkbox"
            class="cursor-pointer accent-primary"
            :checked="isSelected"
            :aria-label="t('downloads.selectAria', { filename: download.filename })"
            @click="onCheckboxClick"
          />
        </span>

        <span role="gridcell" class="flex min-w-0 items-center gap-2.5 pr-4">
          <KindBadge :filename="download.filename" :download-kind="download.kind" />
          <span class="flex min-w-0 flex-col gap-0.5">
            <span class="flex min-w-0 items-center gap-2">
              <span
                class="truncate font-medium"
                :class="download.status === 'completed' ? 'text-dl-soft' : 'text-foreground'"
              >
                {{ download.filename }}
              </span>
              <span
                v-if="swarm"
                class="flex h-5 shrink-0 items-center gap-1 rounded-[10px] bg-muted px-2 text-[11px] font-medium text-dl-soft"
                :title="t('downloads.torrentSwarmBadgeTitle', { peers: swarm.peers, seeds: swarm.seeds })"
              >
                <Users class="h-3 w-3" aria-hidden="true" />
                {{ t("downloads.torrentSwarmBadge", { peers: swarm.peers, seeds: swarm.seeds }) }}
              </span>
            </span>
            <span v-if="subLine" class="truncate text-xs text-muted-foreground">
              {{ subLine }}
            </span>
          </span>
        </span>

        <span role="gridcell" class="flex min-w-0 flex-col justify-center gap-1.5 pr-5">
          <span class="flex min-w-0 items-center gap-1.5 whitespace-nowrap text-xs">
            <component
              :is="statusView.icon"
              v-if="statusView.icon"
              class="h-3.5 w-3.5 shrink-0"
              :class="statusView.tone"
              aria-hidden="true"
            />
            <span class="font-medium" :class="statusView.tone">{{ statusView.label }}</span>
            <template v-if="folderLabel">
              <span class="text-dl-dim" aria-hidden="true">·</span>
              <button
                type="button"
                class="min-w-0 truncate text-muted-foreground underline decoration-border underline-offset-[3px] hover:text-foreground"
                :class="FOCUS_RING"
                :title="t('downloads.rowShowInFolder')"
                @click.stop="openFolder"
              >
                {{ folderLabel }}
              </button>
            </template>
            <span v-if="metaLine" class="ml-auto pl-2 font-mono text-muted-foreground">
              {{ metaLine }}
            </span>
          </span>
          <span
            v-if="statusView.bar"
            class="block h-1 rounded-sm bg-dl-track"
            role="progressbar"
            :aria-valuenow="pct"
            aria-valuemin="0"
            aria-valuemax="100"
          >
            <span
              class="block h-1 rounded-sm transition-[width] duration-300 ease-out"
              :class="statusView.bar"
              :style="{ width: `${pct}%` }"
            />
          </span>
        </span>

        <template v-if="!compact">
          <span
            role="gridcell"
            class="text-right font-mono text-xs"
            :class="sizeLabel ? 'text-foreground' : 'text-muted-foreground'"
          >
            {{ sizeLabel ?? t("downloads.sizeUnknown") }}
          </span>
          <span role="gridcell" class="text-right font-mono text-xs text-muted-foreground">
            {{ addedLabel }}
          </span>
        </template>

        <span role="gridcell" class="flex items-center justify-end gap-1" @click.stop>
          <template v-if="download.status === 'completed'">
            <button
              type="button"
              :class="[
                QUIET_BUTTON,
                'border-border px-3 text-xs font-semibold text-foreground hover:bg-accent group-hover:bg-dl-raised',
                isLit ? 'bg-dl-raised' : 'bg-transparent',
              ]"
              :aria-label="t('downloads.rowOpenAria', { filename: download.filename })"
              @click="openFile"
            >
              {{ t("downloads.rowOpen") }}
            </button>
            <button
              type="button"
              :class="[
                QUIET_BUTTON,
                'w-8 border-transparent text-muted-foreground hover:bg-accent hover:text-foreground',
              ]"
              :aria-label="t('downloads.rowShowInFolder')"
              :title="t('downloads.rowShowInFolder')"
              @click="openFolder"
            >
              <Folder class="h-4 w-4" aria-hidden="true" />
            </button>
          </template>
          <button
            v-else-if="primaryAction"
            type="button"
            :class="[
              QUIET_BUTTON,
              'w-8 hover:bg-accent group-hover:border-border group-hover:bg-dl-raised group-hover:text-foreground',
              isLit
                ? 'border-border bg-dl-raised text-foreground'
                : 'border-transparent text-muted-foreground',
            ]"
            :aria-label="primaryAction.label"
            :title="primaryAction.label"
            @click="primaryAction.run()"
          >
            <component :is="primaryAction.icon" class="h-[15px] w-[15px]" aria-hidden="true" />
          </button>
          <RowMenu :items="menuItems">
            <MoreHorizontal class="h-[18px] w-[18px]" aria-hidden="true" />
          </RowMenu>
        </span>
      </div>
    </ContextMenuTrigger>

    <ContextMenuContent class="dl-surface min-w-[14rem]">
      <ContextMenuItem v-if="anyPlayable" @select="ctxPause">
        <Pause class="h-3.5 w-3.5" />
        <span>{{ t("downloads.menuPause") }}</span>
      </ContextMenuItem>
      <ContextMenuItem v-else-if="anyResumable" @select="ctxResume">
        <Play class="h-3.5 w-3.5" />
        <span>{{ t("downloads.menuResume") }}</span>
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
        <ContextMenuSubContent class="dl-surface">
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
