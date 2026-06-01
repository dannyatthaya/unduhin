import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { api } from "@/types/tauri-bindings";
import type { Category, CategoryId, NewCategoryInput } from "@/types/tauri-bindings";

export const useCategoriesStore = defineStore("categories", () => {
  const list = ref<Category[]>([]);

  const byId = computed(() => {
    const m = new Map<CategoryId, Category>();
    for (const c of list.value) m.set(c.id, c);
    return m;
  });

  function nameOf(id: CategoryId | null | undefined): string {
    if (id == null) return "Uncategorized";
    return byId.value.get(id)?.name ?? "Uncategorized";
  }

  async function refresh() {
    list.value = await api.listCategories();
  }

  async function add(input: NewCategoryInput): Promise<CategoryId> {
    const id = await api.addCategory(input);
    await refresh();
    return id;
  }

  async function update(id: CategoryId, input: NewCategoryInput): Promise<void> {
    await api.updateCategory(id, input);
    await refresh();
  }

  async function remove(id: CategoryId): Promise<void> {
    await api.removeCategory(id);
    await refresh();
  }

  /** Reorder optimistically. Rolls back the local list on backend failure. */
  async function reorder(ids: CategoryId[]): Promise<void> {
    const before = list.value.slice();
    const byIdMap = new Map(before.map((c) => [c.id, c]));
    list.value = ids
      .map((id) => byIdMap.get(id))
      .filter((c): c is Category => c != null);
    try {
      await api.setCategoryOrder(ids);
    } catch (e) {
      list.value = before;
      throw e;
    }
  }

  return { list, byId, nameOf, refresh, add, update, remove, reorder };
});
