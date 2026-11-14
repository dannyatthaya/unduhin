<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, provide, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { onKeyStroke, useEventListener, useMediaQuery } from "@vueuse/core";

import { onOpenAddUrl } from "@/types/tauri-bindings";

import AppSidebar from "@/components/AppSidebar.vue";
import AddUrlDialog from "@/components/AddUrlDialog.vue";
import DownloadGroup from "@/components/DownloadGroup.vue";
import DownloadRow from "@/components/DownloadRow.vue";
import EmptyState, { type EmptyVariant } from "@/components/EmptyState.vue";
import DetailPane from "@/components/detail/DetailPane.vue";
import SkeletonRow from "@/components/SkeletonRow.vue";
import BulkBar, { type SelectScope } from "@/components/downloads/BulkBar.vue";
import DownloadsListHeader from "@/components/downloads/DownloadsListHeader.vue";
import DownloadsTitleBar from "@/components/downloads/DownloadsTitleBar.vue";
import DownloadsToolbar, {
  type ToolbarTab,
} from "@/components/downloads/DownloadsToolbar.vue";
import { DOWNLOADS_LIST_KEY } from "@/components/downloads/listContext";

import { useCategoriesStore } from "@/stores/categories";
import { useDetailStore } from "@/stores/detail";
import { useDownloadsStore } from "@/stores/downloads";
import { useSelectionStore } from "@/stores/selection";
import { useAggregateSpeedHistory } from "@/composables/useAggregateSpeedHistory";
import {
  useGroupedDownloads,
  type DownloadGroup as DownloadGroupData,
  type DownloadGroupKey,
} from "@/composables/useGroupedDownloads";
import { useDownloadsSort } from "@/composables/useDownloadsSort";
import { useToast } from "@/composables/useToast";
import { downloadLinkFromPaste } from "@/lib/pastedLink";
import {
  countByTab,
  matchesTab,
  STATUS_TABS,
  type StatusTab,
} from "@/lib/statusTabs";
import type { DownloadId, DownloadRecord } from "@/types/tauri-bindings";

/** Below this window width the sidebar becomes an icon rail. */
const NARROW_QUERY = "(max-width: 1099px)";
/** Rows a "Done" section shows before its "Show all" row. */
const DONE_PREVIEW_ROWS = 8;
const SKELETON_ROWS = 6;

const WIDE_COLUMNS = "44px minmax(0, 1fr) 280px 88px 96px 136px";
const COMPACT_COLUMNS = "44px minmax(0, 1fr) 200px 136px";
const WIDE_MIN_WIDTH = "900px";

const { t } = useI18n();
const downloads = useDownloadsStore();
const categories = useCategoriesStore();
const detail = useDetailStore();
const selection = useSelectionStore();
const sort = useDownloadsSort();
const toast = useToast();

const search = ref("");
const selectedCategoryId = ref<number | null>(null);
const activeTab = ref<StatusTab>("all");
const showAddUrl = ref(false);
const pastedUrl = ref("");

const isNarrow = useMediaQuery(NARROW_QUERY);
const detailOpen = computed(
  () => detail.openId != null && downloads.records.has(detail.openId),
);
// The list drops Size and Added whenever it is short on width.
const compact = computed(() => isNarrow.value || detailOpen.value);

const { groups, matching, sortedMatching } = useGroupedDownloads(
  () => search.value,
  () => selectedCategoryId.value,
  () => sort.column.value,
  () => sort.dir.value,
);

// ── Toolbar ─────────────────────────────────────────────────────────────────

const TAB_LABEL_KEYS: Record<StatusTab, string> = {
  all: "downloads.all",
  attention: "downloads.groupNeedsAttention",
  downloading: "downloads.groupDownloading",
  queued: "downloads.groupQueued",
  done: "downloads.tabDone",
};

// Tabs with nothing in them are hidden, except the one that is selected.
const tabs = computed<ToolbarTab[]>(() => {
  const counts = countByTab(matching.value);
  return STATUS_TABS.filter(
    (id) => id === "all" || id === activeTab.value || counts[id] > 0,
  ).map((id) => ({ id, label: t(TAB_LABEL_KEYS[id]), count: counts[id] }));
});

const pageTitle = computed(() =>
  selectedCategoryId.value == null
    ? t("downloads.sidebarAllDownloads")
    : categories.nameOf(selectedCategoryId.value),
);

const isIdle = computed(
  () => downloads.totals.active + downloads.totals.queued === 0,
);
const speedSamples = useAggregateSpeedHistory();

async function pauseAll() {
  try {
    await downloads.pauseAll();
  } catch (e: unknown) {
    const message = e instanceof Error ? e.message : "";
    toast.push(message || t("downloads.actionFailed"), "error");
  }
}

// ── Rows ────────────────────────────────────────────────────────────────────

const GROUP_LABEL_KEYS: Record<DownloadGroupKey, string> = {
  "needs-attention": "downloads.groupNeedsAttention",
  active: "downloads.groupDownloading",
  queued: "downloads.groupQueued",
  cancelled: "downloads.groupCancelled",
  completed: "downloads.groupDoneToday",
  "completed-earlier": "downloads.groupDoneEarlier",
};
const DONE_GROUPS: readonly DownloadGroupKey[] = ["completed", "completed-earlier"];

const collapsedGroups = ref(new Set<DownloadGroupKey>());
const expandedGroups = ref(new Set<DownloadGroupKey>());

function toggled<T>(set: Set<T>, value: T): Set<T> {
  const next = new Set(set);
  if (next.has(value)) next.delete(value);
  else next.add(value);
  return next;
}

interface DisplayGroup {
  group: DownloadGroupData;
  label: string;
  collapsed: boolean;
  visibleRows: DownloadRecord[];
}

const displayGroups = computed<DisplayGroup[]>(() =>
  groups.value
    .map((g) => ({
      ...g,
      rows: g.rows.filter((r) => matchesTab(r.status, activeTab.value)),
    }))
    .filter((g) => g.rows.length > 0)
    .map((group) => {
      const isCut =
        DONE_GROUPS.includes(group.key) && !expandedGroups.value.has(group.key);
      return {
        group,
        label: t(GROUP_LABEL_KEYS[group.key]),
        collapsed: collapsedGroups.value.has(group.key),
        visibleRows: isCut ? group.rows.slice(0, DONE_PREVIEW_ROWS) : group.rows,
      };
    }),
);

const flatRows = computed(() =>
  sortedMatching.value.filter((r) => matchesTab(r.status, activeTab.value)),
);

const isGrouped = computed(() => sort.view.value === "grouped");

// Rendered row ids in display order — what Ctrl-A, shift-click ranges and
// arrow-key navigation walk. Collapsed sections and rows behind a
// "Show all" row are not in it: you can only act on what you can see.
const orderedIds = computed<DownloadId[]>(() => {
  if (!isGrouped.value) return flatRows.value.map((r) => r.id);
  return displayGroups.value.flatMap((g) =>
    g.collapsed ? [] : g.visibleRows.map((r) => r.id),
  );
});

const focusedRowId = ref<DownloadId | null>(null);
const activeRowId = computed(() =>
  focusedRowId.value != null && orderedIds.value.includes(focusedRowId.value)
    ? focusedRowId.value
    : null,
);

provide(DOWNLOADS_LIST_KEY, {
  orderedIds: () => orderedIds.value,
  compact,
  activeRowId,
  setActiveRow: (id) => {
    focusedRowId.value = id;
  },
});

const isEmpty = computed(
  () => !downloads.loading && downloads.all.length === 0,
);
const noResults = computed(
  () =>
    !downloads.loading &&
    (isGrouped.value
      ? displayGroups.value.length === 0
      : flatRows.value.length === 0),
);

const TAB_EMPTY_VARIANTS: Record<StatusTab, EmptyVariant> = {
  all: "filtered",
  attention: "failed",
  downloading: "active",
  queued: "queued",
  done: "completed",
};
const emptyVariant = computed<EmptyVariant>(() =>
  search.value.trim() ? "search" : TAB_EMPTY_VARIANTS[activeTab.value],
);

// ── Selection ───────────────────────────────────────────────────────────────

const allSelected = computed(
  () =>
    orderedIds.value.length > 0 &&
    orderedIds.value.every((id) => selection.has(id)),
);

function toggleSelectAll() {
  if (allSelected.value) selection.clear();
  else selection.set(orderedIds.value, orderedIds.value[0] ?? null);
}

// "Select all N in <section>": the section the last-touched row is in,
// including its rows still behind "Show all". In the flat layout the
// scope is simply every listed row.
const selectScope = computed<SelectScope | null>(() => {
  const anchor = selection.anchorId;
  if (anchor == null) return null;
  const scope: SelectScope | null = isGrouped.value
    ? (() => {
        const hit = displayGroups.value.find((g) =>
          g.group.rows.some((r) => r.id === anchor),
        );
        return hit
          ? { label: hit.label, ids: hit.group.rows.map((r) => r.id) }
          : null;
      })()
    : { label: null, ids: flatRows.value.map((r) => r.id) };
  if (!scope || scope.ids.every((id) => selection.has(id))) return null;
  return scope;
});

function selectScopeRows() {
  const scope = selectScope.value;
  if (!scope) return;
  selection.set(new Set([...selection.ids, ...scope.ids]), selection.anchorId);
}

// A press on the empty area below the rows clears the selection. A press
// on the list's own scrollbar also targets the container, so only count
// presses inside its client box.
function clearSelectionFromBackground(e: MouseEvent) {
  const el = e.currentTarget as HTMLElement;
  if (e.target !== el) return;
  if (e.offsetX >= el.clientWidth || e.offsetY >= el.clientHeight) return;
  selection.clear();
}

// Rows can disappear underneath a selection (removed, finished elsewhere).
// Drop their ids so the bulk bar's count never includes ghosts.
watch(
  () => downloads.records,
  (records) => {
    // A details panel pinned to a row that no longer exists is already
    // hidden; drop the id too so Esc doesn't spend a press closing nothing.
    if (detail.openId != null && !records.has(detail.openId)) detail.close();
    if (selection.empty) return;
    const alive = Array.from(selection.ids).filter((id) => records.has(id));
    if (alive.length === selection.count) return;
    const anchor = selection.anchorId;
    selection.set(alive, anchor != null && records.has(anchor) ? anchor : null);
  },
);

// ── Keyboard and paste ──────────────────────────────────────────────────────

const titleBarRef = ref<InstanceType<typeof DownloadsTitleBar> | null>(null);
const listRef = ref<HTMLElement | null>(null);

/** Inputs that take typed text — checkboxes and buttons do not count. */
const NON_TEXT_INPUT_TYPES = ["checkbox", "radio", "button", "submit", "reset", "range"];

function isEditable(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (el == null) return false;
  if (el.tagName === "INPUT") {
    return !NON_TEXT_INPUT_TYPES.includes((el as HTMLInputElement).type);
  }
  return el.tagName === "TEXTAREA" || el.isContentEditable;
}

// Whether a dialog, popover or menu owned the keyboard when the current
// key press began. Sampled in the capture phase, before the overlay's own
// handler closes it — by the time our bubbling shortcuts run, an overlay
// that Esc just dismissed already reads as closed.
let overlayOpenAtKeydown = false;
useEventListener(
  window,
  "keydown",
  () => {
    overlayOpenAtKeydown =
      document.querySelector(
        '[role="dialog"][data-state="open"], [role="menu"][data-state="open"]',
      ) != null;
  },
  { capture: true },
);

function hasOpenOverlay(): boolean {
  return overlayOpenAtKeydown;
}

/** Same check for events that are not key presses (paste). */
function isOverlayOpenNow(): boolean {
  return (
    document.querySelector(
      '[role="dialog"][data-state="open"], [role="menu"][data-state="open"]',
    ) != null
  );
}

function openAddUrl(url = "") {
  pastedUrl.value = url;
  showAddUrl.value = true;
}

onKeyStroke(
  (e) => (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "n",
  (e) => {
    e.preventDefault();
    openAddUrl();
  },
);
onKeyStroke(
  (e) => (e.ctrlKey || e.metaKey) && ["f", "k"].includes(e.key.toLowerCase()),
  (e) => {
    e.preventDefault();
    if (!hasOpenOverlay()) titleBarRef.value?.focusSearch();
  },
);
onKeyStroke(
  (e) => (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a",
  (e) => {
    if (isEditable(e.target) || hasOpenOverlay()) return;
    e.preventDefault();
    const ids = orderedIds.value;
    if (ids.length === 0) return;
    selection.set(ids, ids[0]);
  },
);
// Esc peels back one layer at a time: selection, then details, then search.
// (Esc while typing in the search field is handled by the field itself.)
onKeyStroke("Escape", () => {
  if (hasOpenOverlay()) return;
  if (!selection.empty) selection.clear();
  else if (detailOpen.value) detail.close();
  else if (search.value !== "") search.value = "";
});

// ↑ / ↓ move focus between rows. Only from a focused row — arrow keys
// inside a row's own controls keep their native meaning.
function onListKeydown(e: KeyboardEvent) {
  if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
  const current = e.target as HTMLElement;
  if (!current.matches("[data-row-id]") || !listRef.value) return;
  const rows = Array.from(
    listRef.value.querySelectorAll<HTMLElement>("[data-row-id]"),
  );
  const next = rows[rows.indexOf(current) + (e.key === "ArrowDown" ? 1 : -1)];
  if (!next) return;
  e.preventDefault();
  next.focus();
}

// A link pasted anywhere outside a text field opens New download with it.
useEventListener(window, "paste", (e: ClipboardEvent) => {
  if (isEditable(e.target) || isOverlayOpenNow()) return;
  const link = downloadLinkFromPaste(e.clipboardData?.getData("text") ?? "");
  if (!link) return;
  e.preventDefault();
  openAddUrl(link);
});

// Tray "Add URL…" menu item: surface the same dialog the keyboard
// shortcut + sidebar button open. The Rust side already calls
// `window.show()` + `set_focus()` before emitting, so the user lands
// on this view with the dialog open.
let unlistenOpenAddUrl: (() => void) | null = null;
onMounted(async () => {
  unlistenOpenAddUrl = await onOpenAddUrl(() => openAddUrl());
});
onBeforeUnmount(() => {
  unlistenOpenAddUrl?.();
});
</script>

<template>
  <div class="dl-surface flex min-h-0 flex-1 flex-col bg-background text-foreground">
    <DownloadsTitleBar
      ref="titleBarRef"
      v-model:search="search"
      :compact="isNarrow"
    />

    <div class="flex min-h-0 flex-1">
      <AppSidebar
        :selected-category-id="selectedCategoryId"
        :loading="downloads.loading"
        :rail="isNarrow"
        @select-category="selectedCategoryId = $event"
        @add-url="openAddUrl()"
      />

      <main class="flex min-h-0 min-w-0 flex-1 flex-col">
        <BulkBar
          v-if="!selection.empty"
          :scope="selectScope"
          @select-scope="selectScopeRows"
        />
        <DownloadsToolbar
          v-else
          v-model:active-tab="activeTab"
          :title="pageTitle"
          :tabs="tabs"
          :speed-bps="downloads.aggregateSpeedBps"
          :speed-samples="speedSamples"
          :idle="isIdle"
          :hide-title="isNarrow"
          :dense="compact"
          @pause-all="pauseAll"
        />

        <EmptyState
          v-if="isEmpty"
          variant="welcome"
          @add-url="openAddUrl()"
        />
        <EmptyState
          v-else-if="noResults"
          :variant="emptyVariant"
          :query="search.trim()"
          @clear-search="search = ''"
        />

        <div
          v-else
          ref="listRef"
          role="grid"
          :aria-label="t('downloads.listAria')"
          aria-multiselectable="true"
          :aria-busy="downloads.loading"
          class="min-h-0 flex-1 overflow-auto"
          :style="{ '--dl-cols': compact ? COMPACT_COLUMNS : WIDE_COLUMNS }"
          @keydown="onListKeydown"
          @mousedown="clearSelectionFromBackground"
        >
          <div :style="{ minWidth: compact ? '0px' : WIDE_MIN_WIDTH }">
            <DownloadsListHeader
              :compact="compact"
              :all-selected="allSelected"
              :some-selected="!selection.empty"
              @toggle-all="toggleSelectAll"
            />

            <template v-if="downloads.loading">
              <SkeletonRow
                v-for="i in SKELETON_ROWS"
                :key="i"
                :compact="compact"
              />
            </template>

            <template v-else-if="isGrouped">
              <DownloadGroup
                v-for="entry in displayGroups"
                :key="entry.group.key"
                :group="entry.group"
                :visible-rows="entry.visibleRows"
                :collapsed="entry.collapsed"
                :label="entry.label"
                :compact="compact"
                @toggle="collapsedGroups = toggled(collapsedGroups, entry.group.key)"
                @show-all="expandedGroups = toggled(expandedGroups, entry.group.key)"
              />
            </template>

            <div v-else role="rowgroup">
              <DownloadRow
                v-for="row in flatRows"
                :key="row.id"
                :download="row"
              />
            </div>
          </div>
        </div>
      </main>

      <DetailPane />
    </div>

    <AddUrlDialog
      :open="showAddUrl"
      :initial-url="pastedUrl"
      @close="showAddUrl = false"
    />
  </div>
</template>
