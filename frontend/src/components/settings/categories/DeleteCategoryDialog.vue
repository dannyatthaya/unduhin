<script setup lang="ts">
// Confirmation before a category is deleted. Says what happens to the
// downloads filed in it (they move to Other), to their files (nothing) and
// to its extensions (they fall back to Other too).

import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { Trash2 } from "lucide-vue-next";

import Dialog from "@/components/ui/Dialog.vue";
import Button from "@/components/ui/Button.vue";

import { useCategoriesStore } from "@/stores/categories";
import { useDownloadsStore } from "@/stores/downloads";
import { colorClass, colorFor, iconFor } from "@/lib/categoryIcons";
import { FALLBACK_CATEGORY_NAME } from "@/lib/categoryMatch";
import { parentFolderLabel } from "@/lib/format";
import type { Category } from "@/types/tauri-bindings";

/** Extensions named in the sentence before "and N more". */
const NAMED_EXTENSIONS = 2;

const props = defineProps<{ category: Category | null }>();
const emit = defineEmits<{ close: []; deleted: [] }>();

const { t } = useI18n();
const store = useCategoriesStore();
const downloads = useDownloadsStore();

const deleting = ref(false);
const error = ref<string | null>(null);

watch(
  () => props.category,
  () => {
    error.value = null;
    deleting.value = false;
  },
);

const downloadCount = computed(() =>
  props.category
    ? downloads.all.filter((d) => d.category_id === props.category?.id).length
    : 0,
);

const folderLabel = computed(() => {
  const path = props.category?.default_output_path;
  // `parentFolderLabel` shortens a file path to its last folders; give it a
  // file-shaped path so the category folder itself is what is kept.
  return path ? parentFolderLabel(`${path}/x`) : "";
});

const extensionsLine = computed(() => {
  const all = props.category?.extension_rules ?? [];
  if (all.length === 0) return "";
  const named = all.slice(0, NAMED_EXTENSIONS).map((e) => `.${e}`);
  const rest = all.length - named.length;
  const list =
    rest > 0
      ? t("settings.categoriesDeleteExtensionsMore", { list: named.join(", "), n: rest }, rest)
      : named.join(", ");
  return t("settings.categoriesDeleteExtensions", {
    list,
    fallback: FALLBACK_CATEGORY_NAME,
  });
});

async function confirm() {
  if (!props.category || deleting.value) return;
  deleting.value = true;
  error.value = null;
  try {
    await store.remove(props.category.id);
    // The backend re-files the category's downloads under Other without
    // emitting an event per row; reload so the list and counts follow.
    await downloads.refresh();
    emit("deleted");
    emit("close");
  } catch (e: unknown) {
    error.value = (e as { message?: string })?.message ?? String(e);
  } finally {
    deleting.value = false;
  }
}
</script>

<template>
  <Dialog
    :open="category != null"
    :title="category ? t('settings.categoriesDeleteTitle', { name: category.name }) : ''"
    alert
    @close="emit('close')"
  >
    <div v-if="category" class="flex items-start gap-3">
      <span
        class="flex h-9 w-9 shrink-0 items-center justify-center rounded-[10px]"
        :class="colorClass(colorFor(category))"
      >
        <component :is="iconFor(category.icon).icon" class="h-[18px] w-[18px]" aria-hidden="true" />
      </span>
      <span class="flex min-w-0 flex-col gap-2 leading-normal">
        <span>
          {{
            t(
              "settings.categoriesDeleteBody",
              { n: downloadCount, fallback: FALLBACK_CATEGORY_NAME },
              downloadCount,
            )
          }}
          <template v-if="folderLabel">
            {{ t("settings.categoriesDeleteFilesStay") }}
            <span class="break-all font-mono text-xs">{{ folderLabel }}</span>.
          </template>
        </span>
        <span v-if="extensionsLine" class="text-muted-foreground">{{ extensionsLine }}</span>
      </span>
    </div>

    <p v-if="error" role="alert" class="text-xs leading-[1.45] text-danger">{{ error }}</p>

    <template #footer>
      <Button :disabled="deleting" @click="emit('close')">{{ t("common.cancel") }}</Button>
      <Button variant="danger" :disabled="deleting" @click="confirm">
        <Trash2 class="h-[15px] w-[15px]" aria-hidden="true" />
        {{ t("settings.categoriesDeleteAction") }}
      </Button>
    </template>
  </Dialog>
</template>
