<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { Plus, Info } from "lucide-vue-next";

import SettingsSection from "@/components/settings/SettingsSection.vue";
import SettingCard from "@/components/settings/SettingCard.vue";
import Button from "@/components/ui/Button.vue";

import CategoriesTable from "@/components/settings/categories/CategoriesTable.vue";
import RulesEditor from "@/components/settings/categories/RulesEditor.vue";
import CategoryDialog from "@/components/settings/categories/CategoryDialog.vue";

import { useCategoriesStore } from "@/stores/categories";
import { useDownloadsStore } from "@/stores/downloads";
import { useSettingsFilter } from "@/composables/useSettingsFilter";
import type { Category } from "@/types/tauri-bindings";

const { t } = useI18n();
const store = useCategoriesStore();
const downloads = useDownloadsStore();
const filter = useSettingsFilter();
const isHidden = (id: string) => filter.isHidden(id);

const dialogOpen = ref(false);
const editing = ref<Category | null>(null);

function openAdd() {
  editing.value = null;
  dialogOpen.value = true;
}
function openEdit(cat: Category) {
  editing.value = cat;
  dialogOpen.value = true;
}
function closeDialog() {
  dialogOpen.value = false;
  editing.value = null;
}

async function confirmRemove(cat: Category) {
  if (cat.name === "Other") return;
  const ok = window.confirm(
    `${t("settings.categoriesDeleteConfirm", { name: cat.name })} ${t("settings.categoriesDeleteHint")}`,
  );
  if (!ok) return;
  await store.remove(cat.id);
}

const counts = computed(() => ({
  categories: store.list.length,
  downloads: downloads.all.length,
}));

const ruleCount = computed(
  () =>
    store.list.filter((c) => c.name !== "Other" && c.extension_rules.length > 0)
      .length + 1, // +1 for the locked "everything else" rule
);
</script>

<template>
  <SettingsSection
    :eyebrow="t('downloads.settings')"
    :title="t('settings.sectionCategories')"
    :description="t('settings.sectionCategoriesDesc')"
  >
    <template #actions>
      <Button variant="primary" @click="openAdd">
        <Plus class="h-4 w-4" />
        {{ t("settings.categoriesAdd") }}
      </Button>
    </template>

    <SettingCard
      :title="t('settings.categoriesListTitle')"
      :description="t('settings.categoriesListDesc')"
    >
      <template #actions>
        <div class="text-right text-xs text-muted-foreground">
          {{ t("settings.categoriesCounts", { categories: counts.categories, downloads: counts.downloads }) }}
        </div>
      </template>
      <div
        :data-setting-id="'categories/list'"
        v-show="!isHidden('categories/list')"
      >
        <CategoriesTable @edit="openEdit" @remove="confirmRemove" />
      </div>
    </SettingCard>

    <SettingCard
      :title="t('settings.categoriesRulesLabel')"
      :description="t('settings.categoriesRulesDesc')"
    >
      <template #actions>
        <div class="text-right text-xs text-muted-foreground">
          {{ t("settings.categoriesRulesCount", { n: ruleCount }) }}
        </div>
      </template>
      <div
        :data-setting-id="'categories/rules'"
        v-show="!isHidden('categories/rules')"
      >
        <RulesEditor />
      </div>
    </SettingCard>

    <div
      class="flex items-start gap-2 rounded-md border border-primary/30 bg-primary/5 px-4 py-3 text-xs text-primary"
    >
      <Info class="mt-0.5 h-4 w-4 shrink-0" />
      <p>
        <strong class="font-semibold">{{ t("settings.categoriesRulesOrderTitle") }}</strong>
        {{ t("settings.categoriesRulesOrderBody") }}
      </p>
    </div>

    <CategoryDialog
      :open="dialogOpen"
      :category="editing"
      @close="closeDialog"
    />
  </SettingsSection>
</template>
