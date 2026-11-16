<script setup lang="ts">
// Settings → Categories: the list, and a short note on how sorting works.

import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { Info, Plus } from "lucide-vue-next";

import SettingCard from "@/components/settings/SettingCard.vue";
import SettingsSection from "@/components/settings/SettingsSection.vue";
import CategoriesTable from "@/components/settings/categories/CategoriesTable.vue";
import CategoryDialog from "@/components/settings/categories/CategoryDialog.vue";
import DeleteCategoryDialog from "@/components/settings/categories/DeleteCategoryDialog.vue";
import Button from "@/components/ui/Button.vue";

import { useCategoriesStore } from "@/stores/categories";
import { useDownloadsStore } from "@/stores/downloads";
import { useSettingsFilter } from "@/composables/useSettingsFilter";
import type { Category } from "@/types/tauri-bindings";

const { t } = useI18n();
const store = useCategoriesStore();
const downloads = useDownloadsStore();
const filter = useSettingsFilter();

const dialogOpen = ref(false);
const editing = ref<Category | null>(null);
const deleting = ref<Category | null>(null);

function openAdd() {
  editing.value = null;
  dialogOpen.value = true;
}
function openEdit(category: Category) {
  editing.value = category;
  dialogOpen.value = true;
}
function closeDialog() {
  dialogOpen.value = false;
  editing.value = null;
}
/** Delete asked from inside the edit dialog: one dialog at a time. */
function deleteFromDialog(category: Category) {
  closeDialog();
  deleting.value = category;
}

const summary = computed(() =>
  t("settings.categoriesCounts", {
    downloads: t("settings.categoriesCountDownloads", { n: downloads.all.length }, downloads.all.length),
    categories: t("settings.categoriesCountCategories", { n: store.list.length }, store.list.length),
  }),
);
</script>

<template>
  <SettingsSection
    :title="t('settings.sectionCategories')"
    :description="t('settings.sectionCategoriesDesc')"
  >
    <template #actions>
      <Button @click="openAdd">
        <Plus class="h-[15px] w-[15px]" aria-hidden="true" />
        {{ t("settings.categoriesAdd") }}
      </Button>
    </template>

    <SettingCard
      :title="t('settings.categoriesListTitle')"
      :note="t('settings.categoriesReorderNote')"
    >
      <template #actions>
        <span class="whitespace-nowrap font-mono text-xs text-muted-foreground">{{ summary }}</span>
      </template>
      <div v-show="!filter.isHidden('categories/list')" data-setting-id="categories/list">
        <CategoriesTable @edit="openEdit" @remove="deleting = $event" />
      </div>
    </SettingCard>

    <section class="flex flex-col gap-2.5" aria-labelledby="categories-how">
      <h2
        id="categories-how"
        class="px-0.5 text-xs font-semibold uppercase tracking-[0.06em] text-muted-foreground"
      >
        {{ t("settings.categoriesHowTitle") }}
      </h2>
      <div class="flex items-start gap-3 rounded-xl border border-dl-line bg-card px-[18px] py-3.5">
        <Info class="mt-px h-[18px] w-[18px] shrink-0 text-muted-foreground" aria-hidden="true" />
        <span class="flex flex-col gap-1 leading-normal">
          <span>{{ t("settings.categoriesHowBody") }}</span>
          <span class="text-muted-foreground">{{ t("settings.categoriesHowClash") }}</span>
        </span>
      </div>
    </section>

    <CategoryDialog
      :open="dialogOpen"
      :category="editing"
      @close="closeDialog"
      @delete="deleteFromDialog"
    />
    <DeleteCategoryDialog :category="deleting" @close="deleting = null" @deleted="deleting = null" />
  </SettingsSection>
</template>
