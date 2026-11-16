<script setup lang="ts">
// The category list. Rows reorder by dragging the handle or with
// Alt + ↑ / ↓ on it; the fallback category stays pinned last.

import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { VueDraggable } from "vue-draggable-plus";

import CategoryRow, { CATEGORY_GRID } from "./CategoryRow.vue";
import { useCategoriesStore } from "@/stores/categories";
import { useDownloadsStore } from "@/stores/downloads";
import { useToast } from "@/composables/useToast";
import type { Category, CategoryId } from "@/types/tauri-bindings";

const emit = defineEmits<{
  edit: [category: Category];
  remove: [category: Category];
}>();

const { t } = useI18n();
const store = useCategoriesStore();
const downloads = useDownloadsStore();
const toast = useToast();

// The fallback is the seeded "Other" row: anything no category owns.
function isFallback(c: Category): boolean {
  return c.name === "Other";
}

const fallback = computed(() => store.list.find(isFallback) ?? null);
const movable = computed(() => store.list.filter((c) => !isFallback(c)));

const counts = computed(() => {
  const byCategory = new Map<CategoryId, number>();
  for (const d of downloads.all) {
    if (d.category_id == null) continue;
    byCategory.set(d.category_id, (byCategory.get(d.category_id) ?? 0) + 1);
  }
  return byCategory;
});

async function saveOrder(next: Category[]): Promise<void> {
  const ids: CategoryId[] = next.map((c) => c.id);
  if (fallback.value) ids.push(fallback.value.id);
  try {
    await store.reorder(ids);
  } catch (e: unknown) {
    const message = (e as { message?: string })?.message ?? String(e);
    toast.push(t("settings.categoriesReorderFailed", { error: message }), "error");
  }
}

const draggable = computed({
  get: () => movable.value,
  set: (next: Category[]) => void saveOrder(next),
});

function move(category: Category, step: -1 | 1): void {
  const from = movable.value.findIndex((c) => c.id === category.id);
  const to = from + step;
  if (from === -1 || to < 0 || to >= movable.value.length) return;
  const next = [...movable.value];
  next.splice(to, 0, ...next.splice(from, 1));
  void saveOrder(next);
}
</script>

<template>
  <div role="table" :aria-label="t('settings.sectionCategories')" class="flex flex-col">
    <div
      role="row"
      :class="CATEGORY_GRID"
      class="h-[34px] border-b border-dl-line bg-dl-group text-xs font-semibold text-muted-foreground"
    >
      <span role="columnheader"><span class="sr-only">{{ t("settings.categoriesColOrder") }}</span></span>
      <span role="columnheader">{{ t("settings.categoriesColName") }}</span>
      <span role="columnheader">{{ t("settings.categoriesColFolder") }}</span>
      <span role="columnheader" class="text-right">{{ t("settings.categoriesColFiles") }}</span>
      <span role="columnheader"><span class="sr-only">{{ t("settings.categoriesColActions") }}</span></span>
    </div>
    <VueDraggable v-model="draggable" handle=".drag-handle" :animation="150" tag="div" role="rowgroup">
      <CategoryRow
        v-for="cat in draggable"
        :key="cat.id"
        :category="cat"
        :count="counts.get(cat.id) ?? 0"
        class="!border-b"
        @edit="emit('edit', $event)"
        @remove="emit('remove', $event)"
        @move="move"
      />
    </VueDraggable>
    <CategoryRow
      v-if="fallback"
      :category="fallback"
      :count="counts.get(fallback.id) ?? 0"
      is-fallback
      @edit="emit('edit', $event)"
      @remove="emit('remove', $event)"
    />
  </div>
</template>
