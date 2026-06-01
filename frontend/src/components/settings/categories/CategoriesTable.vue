<script setup lang="ts">
import { computed } from "vue";
import { VueDraggable } from "vue-draggable-plus";

import CategoryRow from "@/components/settings/categories/CategoryRow.vue";
import { useCategoriesStore } from "@/stores/categories";
import type { Category, CategoryId } from "@/types/tauri-bindings";

const emit = defineEmits<{
  edit: [category: Category];
  remove: [category: Category];
}>();

const store = useCategoriesStore();

// The fallback row is pinned at the bottom and excluded from drag-reorder.
// In the seed data it's the one named "Other"; treat that as the marker.
function isFallback(c: Category): boolean {
  return c.name === "Other";
}

const draggable = computed({
  get: () => store.list.filter((c) => !isFallback(c)),
  set: async (next: Category[]) => {
    const fallback = store.list.find(isFallback);
    const orderedIds: CategoryId[] = next.map((c) => c.id);
    if (fallback) orderedIds.push(fallback.id);
    try {
      await store.reorder(orderedIds);
    } catch (e) {
      console.warn("category reorder failed", e);
    }
  },
});

const fallback = computed(() => store.list.find(isFallback) ?? null);
</script>

<template>
  <div class="overflow-hidden">
    <VueDraggable
      v-model="draggable"
      handle=".drag-handle"
      :animation="150"
      tag="div"
    >
      <CategoryRow
        v-for="cat in draggable"
        :key="cat.id"
        :category="cat"
        @edit="(c) => emit('edit', c)"
        @remove="(c) => emit('remove', c)"
      />
    </VueDraggable>
    <CategoryRow
      v-if="fallback"
      :category="fallback"
      :is-fallback="true"
      @edit="(c) => emit('edit', c)"
      @remove="(c) => emit('remove', c)"
    />
  </div>
</template>
