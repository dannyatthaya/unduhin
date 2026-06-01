<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import Dialog from "@/components/ui/Dialog.vue";
import Button from "@/components/ui/Button.vue";
import Input from "@/components/ui/Input.vue";
import Switch from "@/components/ui/Switch.vue";

import FolderPicker from "@/components/settings/controls/FolderPicker.vue";
import ChipInput from "@/components/settings/controls/ChipInput.vue";
import CategoryIconPicker from "@/components/settings/categories/CategoryIconPicker.vue";

import { useCategoriesStore } from "@/stores/categories";
import type { Category, NewCategoryInput } from "@/types/tauri-bindings";

const { t } = useI18n();

const props = defineProps<{
  open: boolean;
  category?: Category | null;
}>();
const emit = defineEmits<{ close: [] }>();

const store = useCategoriesStore();

const name = ref("");
const icon = ref("document");
const folder = ref("");
const wantsRule = ref(true);
const extensions = ref<string[]>([]);
const saving = ref(false);
const error = ref<string | null>(null);

const isEdit = computed(() => props.category != null);
const title = computed(() =>
  isEdit.value ? t("settings.categoriesEdit") : t("settings.categoriesCreate"),
);
const eyebrow = computed(() =>
  isEdit.value
    ? t("settings.categoriesEyebrowEdit")
    : t("settings.categoriesEyebrowNew"),
);

watch(
  () => [props.open, props.category],
  () => {
    if (!props.open) return;
    error.value = null;
    saving.value = false;
    if (props.category) {
      name.value = props.category.name;
      icon.value = props.category.icon ?? "document";
      folder.value = props.category.default_output_path ?? "";
      extensions.value = [...props.category.extension_rules];
      wantsRule.value = props.category.extension_rules.length > 0;
    } else {
      name.value = "";
      icon.value = "document";
      folder.value = "";
      extensions.value = [];
      wantsRule.value = true;
    }
  },
  { immediate: true },
);

async function submit() {
  if (saving.value) return;
  const trimmed = name.value.trim();
  if (!trimmed) {
    error.value = t("settings.categoriesNameRequired");
    return;
  }
  const input: NewCategoryInput = {
    name: trimmed,
    icon: icon.value,
    default_output_path: folder.value.trim() || null,
    extension_rules: wantsRule.value ? extensions.value : [],
  };
  saving.value = true;
  try {
    if (isEdit.value && props.category) {
      await store.update(props.category.id, input);
    } else {
      await store.add(input);
    }
    emit("close");
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : String(e);
    saving.value = false;
  }
}
</script>

<template>
  <Dialog :open="open" @close="emit('close')">
    <div class="flex flex-col gap-5">
      <div class="flex flex-col gap-1">
        <span class="text-[10px] font-semibold uppercase tracking-[0.18em] text-primary">
          {{ eyebrow }}
        </span>
        <h2 class="text-lg font-semibold">{{ title }}</h2>
        <p class="text-xs text-muted-foreground">
          {{ t("settings.categoriesDialogHint") }}
        </p>
      </div>

      <div class="flex flex-col gap-1.5">
        <label class="text-sm font-medium">{{ t("settings.categoriesNameLabel") }}</label>
        <Input v-model="name" placeholder="eBooks" />
      </div>

      <div class="flex flex-col gap-1.5">
        <label class="text-sm font-medium">{{ t("settings.categoriesIconLabel") }}</label>
        <CategoryIconPicker v-model="icon" />
        <p class="text-xs text-muted-foreground">
          {{ t("settings.categoriesIconHint") }}
        </p>
      </div>

      <div class="flex flex-col gap-1.5">
        <label class="text-sm font-medium">{{ t("settings.categoriesFolderLabel") }}</label>
        <FolderPicker v-model="folder" :placeholder="t('settings.categoriesFolderPlaceholder')" />
      </div>

      <div class="flex flex-col gap-2 rounded-md border border-border p-3">
        <div class="flex items-start justify-between gap-3">
          <div class="flex flex-col">
            <span class="text-sm font-medium">{{ t("settings.categoriesMakeRule") }}</span>
            <span class="text-xs text-muted-foreground">
              {{ t("settings.categoriesMakeRuleHint") }}
            </span>
          </div>
          <Switch v-model="wantsRule" :aria-label="t('settings.categoriesMakeRule')" />
        </div>
        <div v-if="wantsRule">
          <ChipInput
            v-model="extensions"
            prefix="."
            placeholder=".epub .mobi .azw3"
          />
        </div>
      </div>

      <p v-if="error" class="text-sm text-danger">{{ error }}</p>
    </div>

    <template #footer>
      <Button variant="secondary" :disabled="saving" @click="emit('close')">
        {{ t("common.cancel") }}
      </Button>
      <Button variant="primary" :disabled="saving" @click="submit">
        {{ isEdit ? t("settings.categoriesSaveChanges") : t("settings.categoriesCreate") }}
      </Button>
    </template>
  </Dialog>
</template>
