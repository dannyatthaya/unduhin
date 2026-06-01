<script setup lang="ts">
// Compact view + sort controls for the downloads list. Two segments:
//   1. View toggle: grouped (the screenshot-style bands) vs flat (one
//      uninterrupted list).
//   2. Sort chips: one per supported column. Clicking the active chip
//      flips direction; clicking another switches column with `desc`
//      as the default.
//
// State lives in the `downloads_sort` setting via `useDownloadsSort`,
// so the picks survive restart.

import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { ArrowDown, ArrowUp, LayoutList, Rows3 } from "lucide-vue-next";

import {
  useDownloadsSort,
  type SortColumn,
} from "@/composables/useDownloadsSort";

const { t } = useI18n();
const sort = useDownloadsSort();

const columns = computed<{ id: SortColumn; label: string }[]>(() => [
  { id: "filename", label: t("downloads.sortFilename") },
  { id: "size", label: t("downloads.sortSize") },
  { id: "speed", label: t("downloads.sortSpeed") },
  { id: "eta", label: t("downloads.sortEta") },
  { id: "status", label: t("downloads.sortStatus") },
  { id: "added_at", label: t("downloads.sortAdded") },
]);
</script>

<template>
  <div class="flex flex-wrap items-center gap-x-3 gap-y-2 rounded-md border border-border bg-card/60 px-3 py-2">
    <div class="flex items-center gap-2">
      <span class="text-xs font-medium text-muted-foreground">{{ t("downloads.viewLabel") }}</span>

      <div class="inline-flex overflow-hidden rounded-md border border-border bg-background">
        <button
          type="button"
          class="inline-flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium transition-colors"
          :class="sort.view.value === 'grouped'
              ? 'bg-primary text-primary-foreground'
              : 'text-foreground hover:bg-accent'
            "
          :aria-pressed="sort.view.value === 'grouped'"
          @click="sort.view.value = 'grouped'"
        >
          <Rows3 class="h-3.5 w-3.5" />
          {{ t("downloads.viewGrouped") }}
        </button>

        <button
          type="button"
          class="inline-flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium transition-colors"
          :class="sort.view.value === 'flat'
              ? 'bg-primary text-primary-foreground'
              : 'text-foreground hover:bg-accent'
            "
          :aria-pressed="sort.view.value === 'flat'"
          @click="sort.view.value = 'flat'"
        >
          <LayoutList class="h-3.5 w-3.5" />
          {{ t("downloads.viewFlat") }}
        </button>
      </div>
    </div>

    <span
      class="h-5 w-px bg-border"
      aria-hidden="true"
    />

    <div class="flex flex-wrap items-center gap-1">
      <span class="text-xs font-medium text-muted-foreground">{{ t("downloads.sortBy") }}</span>
      <button
        v-for="c in columns"
        :key="c.id"
        type="button"
        class="inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium transition-colors"
        :class="sort.column.value === c.id
            ? 'bg-primary/10 text-primary'
            : 'text-foreground hover:bg-accent'
          "
        :aria-pressed="sort.column.value === c.id"
        :aria-label="t('downloads.sortByAria', { column: c.label })"
        @click="sort.toggleColumn(c.id)"
      >
        {{ c.label }}
        <ArrowDown
          v-if="sort.column.value === c.id && sort.dir.value === 'desc'"
          class="h-3 w-3"
        />
        <ArrowUp
          v-else-if="sort.column.value === c.id && sort.dir.value === 'asc'"
          class="h-3 w-3"
        />
      </button>
    </div>
  </div>
</template>
