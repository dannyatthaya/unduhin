<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { ArrowRight, Trash2, Lock } from "lucide-vue-next";

import ChipInput from "@/components/settings/controls/ChipInput.vue";
import Select from "@/components/ui/Select.vue";

import { useCategoriesStore } from "@/stores/categories";
import { iconFor } from "@/lib/categoryIcons";
import type { Category, CategoryId } from "@/types/tauri-bindings";

const props = defineProps<{
  category: Category;
  /** Locked rows render the static "everything else" form. */
  locked?: boolean;
}>();
const emit = defineEmits<{
  "update:extensions": [extensions: string[]];
  "update:category": [target: Category];
  remove: [];
}>();

const { t } = useI18n();
const store = useCategoriesStore();

const targetOptions = computed(() =>
  store.list.map((c) => ({ value: c.id, label: c.name })),
);

const targetId = computed({
  get: () => props.category.id,
  set: (id: CategoryId) => {
    const next = store.byId.get(id);
    if (next) emit("update:category", next);
  },
});

const iconOpt = computed(() => iconFor(props.category.icon));
</script>

<template>
  <div class="flex items-center gap-3 border-t border-border/60 px-5 py-3 first:border-t-0">
    <div class="flex flex-1 items-center gap-3">
      <ChipInput
        v-if="!locked"
        :model-value="category.extension_rules"
        :prefix="'.'"
        placeholder=".ext"
        @update:model-value="(v) => emit('update:extensions', v)"
      />
      <div
        v-else
        class="flex h-9 flex-1 items-center gap-2 rounded-md border border-dashed border-border bg-muted/40 px-3 text-xs text-muted-foreground"
      >
        <Lock class="h-3.5 w-3.5" />
        <span>{{ t("settings.categoriesRuleEverythingElse") }}</span>
      </div>
    </div>
    <ArrowRight class="h-4 w-4 shrink-0 text-muted-foreground" />
    <div class="w-48 shrink-0">
      <div class="flex items-center gap-2">
        <span
          class="flex h-7 w-7 items-center justify-center rounded-md"
          :class="iconOpt.background"
        >
          <component :is="iconOpt.icon" class="h-3.5 w-3.5" :class="iconOpt.tone" />
        </span>
        <Select
          v-model="targetId"
          :options="targetOptions"
          :disabled="locked"
        />
      </div>
    </div>
    <button
      type="button"
      :disabled="locked"
      class="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-danger disabled:cursor-not-allowed disabled:opacity-30"
      :title="locked ? t('settings.categoriesRuleFallbackLocked') : t('common.delete')"
      @click="emit('remove')"
    >
      <Trash2 class="h-3.5 w-3.5" />
    </button>
  </div>
</template>
