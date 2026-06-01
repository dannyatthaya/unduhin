<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, provide, ref } from "vue";
import { onKeyStroke } from "@vueuse/core";

import { onOpenAddUrl } from "@/types/tauri-bindings";

import AppSidebar from "@/components/AppSidebar.vue";
import AppTopBar from "@/components/AppTopBar.vue";
import AppStatusBar from "@/components/AppStatusBar.vue";
import AddUrlDialog from "@/components/AddUrlDialog.vue";
import DownloadGroup from "@/components/DownloadGroup.vue";
import DownloadRow from "@/components/DownloadRow.vue";
import EmptyState from "@/components/EmptyState.vue";
import DetailPane from "@/components/detail/DetailPane.vue";
import BatchActionBar from "@/components/BatchActionBar.vue";
import SkeletonRow from "@/components/SkeletonRow.vue";
import SortToolbar from "@/components/SortToolbar.vue";

import { useDetailStore } from "@/stores/detail";
import { useDownloadsStore } from "@/stores/downloads";
import { useSelectionStore } from "@/stores/selection";
import { useGroupedDownloads } from "@/composables/useGroupedDownloads";
import { useDownloadsSort } from "@/composables/useDownloadsSort";
import type { DownloadId, Status } from "@/types/tauri-bindings";

const downloads = useDownloadsStore();
const detail = useDetailStore();
const selection = useSelectionStore();
const sort = useDownloadsSort();

const search = ref("");
const selectedCategoryId = ref<number | null>(null);
const selectedStatus = ref<Status | null>(null);
const showAddUrl = ref(false);

const { groups, sortedMatching } = useGroupedDownloads(
  () => search.value,
  () => selectedCategoryId.value,
  () => sort.column.value,
  () => sort.dir.value,
);

const visibleGroups = computed(() => {
  if (selectedStatus.value == null) return groups.value;
  const s = selectedStatus.value;
  return groups.value
    .map((g) => ({
      ...g,
      rows: g.rows.filter((r) => r.status === s),
    }))
    .filter((g) => g.rows.length > 0);
});

const flatRows = computed(() => {
  if (selectedStatus.value == null) return sortedMatching.value;
  const s = selectedStatus.value;
  return sortedMatching.value.filter((r) => r.status === s);
});

// Flattened, display-ordered ids — used by Ctrl-A select-all and by
// shift-click range selection inside rows. In grouped mode this walks
// the groups in their display order; in flat mode it's the same list
// the renderer iterates.
const orderedIds = computed<DownloadId[]>(() => {
  if (sort.view.value === "flat") return flatRows.value.map((r) => r.id);
  const out: DownloadId[] = [];
  for (const g of visibleGroups.value) for (const r of g.rows) out.push(r.id);
  return out;
});
provide("orderedIds", () => orderedIds.value);

const noResults = computed(() =>
  sort.view.value === "flat"
    ? flatRows.value.length === 0
    : visibleGroups.value.length === 0,
);

const isEmpty = computed(
  () => !downloads.loading && downloads.all.length === 0
);

async function refresh() {
  await downloads.refresh();
}

function clearSelectionFromBackground(e: MouseEvent) {
  if (e.target === e.currentTarget) selection.clear();
}

onKeyStroke(
  (e) => (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "n",
  (e) => {
    e.preventDefault();
    showAddUrl.value = true;
  }
);
onKeyStroke(
  (e) => (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a",
  (e) => {
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable))
      return;
    e.preventDefault();
    const ids = orderedIds.value;
    if (ids.length === 0) return;
    selection.set(ids, ids[0]);
  }
);
onKeyStroke("Escape", () => {
  if (!selection.empty) selection.clear();
  else if (detail.openId != null) detail.close();
});

// Tray "Add URL…" menu item: surface the same dialog the keyboard
// shortcut + top-bar button open. The Rust side already calls
// `window.show()` + `set_focus()` before emitting, so the user lands
// on this view with the dialog open.
let unlistenOpenAddUrl: (() => void) | null = null;
onMounted(async () => {
  unlistenOpenAddUrl = await onOpenAddUrl(() => {
    showAddUrl.value = true;
  });
});
onBeforeUnmount(() => {
  unlistenOpenAddUrl?.();
});
</script>

<template>
  <div class="flex min-h-0 flex-1">
    <AppSidebar
      :selected-category-id="selectedCategoryId"
      :selected-status="selectedStatus"
      :loading="downloads.loading"
      @select-category="selectedCategoryId = $event"
      @select-status="selectedStatus = $event"
    />

    <div class="flex h-full min-w-0 flex-1 flex-col bg-muted/40">
      <AppTopBar
        v-model:search="search"
        :is-empty="isEmpty"
        :loading="downloads.loading"
        @add-url="showAddUrl = true"
        @pause-all="downloads.pauseAll()"
        @refresh="refresh"
      />

      <main
        class="relative flex-1 overflow-y-auto overflow-x-hidden px-5 pb-5 pt-2"
        @mousedown="clearSelectionFromBackground"
      >
        <div
          v-if="downloads.loading"
          class="mx-auto max-w-5xl space-y-3"
        >
          <SkeletonRow
            v-for="i in 5"
            :key="i"
          />
        </div>

        <EmptyState
          v-else-if="isEmpty"
          variant="welcome"
          @add-url="showAddUrl = true"
        />

        <div
          v-else
          class="mx-auto max-w-5xl space-y-4"
        >
          <SortToolbar />

          <template v-if="!noResults">
            <template v-if="sort.view.value === 'grouped'">
              <div class="space-y-6">
                <DownloadGroup
                  v-for="group in visibleGroups"
                  :key="group.key"
                  :group="group"
                />
              </div>
            </template>

            <template v-else>
              <TransitionGroup
                tag="div"
                class="space-y-2"
                enter-active-class="transition-all duration-150 ease-out"
                leave-active-class="transition-all duration-150 ease-in"
                enter-from-class="opacity-0 -translate-y-1"
                leave-to-class="opacity-0"
              >
                <DownloadRow
                  v-for="row in flatRows"
                  :key="row.id"
                  :download="row"
                />
              </TransitionGroup>
            </template>
          </template>

          <EmptyState
            v-else
            :variant="selectedStatus ?? 'filtered'"
            @add-url="showAddUrl = true"
          />
        </div>

        <BatchActionBar />
      </main>

      <AppStatusBar :loading="downloads.loading" />
    </div>

    <DetailPane />

    <AddUrlDialog
      :open="showAddUrl"
      @close="showAddUrl = false"
    />
  </div>
</template>
