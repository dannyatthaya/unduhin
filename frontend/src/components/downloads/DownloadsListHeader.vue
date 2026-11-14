<script setup lang="ts">
// Sticky column header of the downloads list: select-all, the sortable
// column buttons, and the grouped / flat layout toggle. Sort and layout
// persist through `useDownloadsSort` (the `downloads_sort` setting).

import { computed, ref, watchEffect } from "vue";
import { useI18n } from "vue-i18n";
import { ArrowDown, ArrowUp, List, Rows3 } from "lucide-vue-next";

import {
  useDownloadsSort,
  type SortColumn,
  type ViewMode,
} from "@/composables/useDownloadsSort";

const props = defineProps<{
  /** Size and Added columns are hidden. */
  compact: boolean;
  allSelected: boolean;
  someSelected: boolean;
}>();

const emit = defineEmits<{ "toggle-all": [] }>();

const { t } = useI18n();
const sort = useDownloadsSort();

// `indeterminate` is a DOM property with no attribute form.
const selectAllRef = ref<HTMLInputElement | null>(null);
watchEffect(() => {
  if (selectAllRef.value) {
    selectAllRef.value.indeterminate = props.someSelected && !props.allSelected;
  }
});

interface HeaderColumn {
  id: SortColumn;
  label: string;
  /** Right-aligned numeric column. */
  end: boolean;
}

// Size and Added drop out in the compact layout; the grid template the
// list container sets has two fewer tracks to match.
const columns = computed<HeaderColumn[]>(() => {
  const base: HeaderColumn[] = [
    { id: "filename", label: t("downloads.colName"), end: false },
    { id: "status", label: t("downloads.sortStatus"), end: false },
  ];
  if (props.compact) return base;
  return [
    ...base,
    { id: "size", label: t("downloads.sortSize"), end: true },
    { id: "added_at", label: t("downloads.sortAdded"), end: true },
  ];
});

function ariaSort(column: SortColumn): "ascending" | "descending" | "none" {
  if (sort.column.value !== column) return "none";
  return sort.dir.value === "asc" ? "ascending" : "descending";
}

const SORT_BUTTON =
  "inline-flex h-[26px] items-center gap-1 rounded-md px-1.5 uppercase tracking-[inherit] transition-colors hover:bg-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";

function layoutButtonClass(mode: ViewMode): string {
  return sort.view.value === mode
    ? "bg-muted text-foreground"
    : "text-muted-foreground hover:text-foreground";
}
</script>

<template>
  <div
    role="row"
    class="dl-grid-row sticky top-0 z-10 h-8 border-b border-dl-line bg-background pl-7 pr-5 text-[11px] font-semibold uppercase tracking-[0.06em] text-muted-foreground"
  >
    <span role="columnheader" class="flex items-center">
      <input
        ref="selectAllRef"
        type="checkbox"
        class="accent-primary"
        :checked="allSelected"
        :aria-label="t('downloads.selectAll')"
        @change="emit('toggle-all')"
      />
    </span>

    <span
      v-for="column in columns"
      :key="column.id"
      role="columnheader"
      :aria-sort="ariaSort(column.id)"
      :class="column.end ? 'flex justify-end' : ''"
    >
      <button
        type="button"
        :class="[
          SORT_BUTTON,
          column.end ? '-mr-1.5' : '-ml-1.5',
          sort.column.value === column.id ? 'text-foreground' : '',
        ]"
        @click="sort.toggleColumn(column.id)"
      >
        {{ column.label }}
        <template v-if="sort.column.value === column.id">
          <ArrowUp v-if="sort.dir.value === 'asc'" class="h-[11px] w-[11px]" aria-hidden="true" />
          <ArrowDown v-else class="h-[11px] w-[11px]" aria-hidden="true" />
        </template>
      </button>
    </span>

    <span role="columnheader" class="flex justify-end">
      <span class="sr-only">{{ t("downloads.colActions") }}</span>
      <span
        role="group"
        :aria-label="t('downloads.layoutAria')"
        class="flex rounded-lg border border-border bg-card p-0.5"
      >
        <button
          type="button"
          class="flex h-[22px] w-[26px] items-center justify-center rounded-[5px] transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          :class="layoutButtonClass('grouped')"
          :aria-pressed="sort.view.value === 'grouped'"
          :aria-label="t('downloads.layoutGrouped')"
          :title="t('downloads.layoutGrouped')"
          @click="sort.view.value = 'grouped'"
        >
          <Rows3 class="h-[13px] w-[13px]" aria-hidden="true" />
        </button>
        <button
          type="button"
          class="flex h-[22px] w-[26px] items-center justify-center rounded-[5px] transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          :class="layoutButtonClass('flat')"
          :aria-pressed="sort.view.value === 'flat'"
          :aria-label="t('downloads.layoutFlat')"
          :title="t('downloads.layoutFlat')"
          @click="sort.view.value = 'flat'"
        >
          <List class="h-[13px] w-[13px]" aria-hidden="true" />
        </button>
      </span>
    </span>
  </div>
</template>
