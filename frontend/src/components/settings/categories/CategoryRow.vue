<script lang="ts">
/** Column template shared by the header row and every category row. */
export const CATEGORY_GRID =
  "grid grid-cols-[32px_minmax(0,1.4fr)_minmax(0,1fr)_56px_72px] items-center gap-x-3.5 pl-2 pr-3";
</script>

<script setup lang="ts">
// One category in Settings → Categories: reorder handle, tinted icon, name
// and the extensions it owns, its folder, how many downloads it holds, and
// Edit / Delete. The fallback category is locked in last place.

import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Folder, GripVertical, Lock, Pencil, Trash2 } from "lucide-vue-next";

import { useGeneralSettings } from "@/composables/useGeneralSettings";
import { colorClass, colorFor, iconFor } from "@/lib/categoryIcons";
import type { Category } from "@/types/tauri-bindings";

const props = defineProps<{
  category: Category;
  /** Downloads filed under this category. */
  count: number;
  isFallback?: boolean;
}>();
const emit = defineEmits<{
  edit: [category: Category];
  remove: [category: Category];
  /** Keyboard reorder: -1 is up, 1 is down. */
  move: [category: Category, step: -1 | 1];
}>();

const { t } = useI18n();
const general = useGeneralSettings();

const SHOWN_EXTENSIONS = 5;

const icon = computed(() => iconFor(props.category.icon));
const tint = computed(() => colorClass(colorFor(props.category)));

/** ".mp4  .mkv  .webm  +6": spaces, not dots, between them. */
const extensions = computed(() => {
  const all = props.category.extension_rules;
  if (all.length === 0) {
    return props.isFallback ? t("settings.categoriesFallbackOwns") : t("settings.categoriesNoExtensions");
  }
  const shown = all.slice(0, SHOWN_EXTENSIONS).map((ext) => `.${ext}`);
  const rest = all.length - shown.length;
  return rest > 0 ? [...shown, `+${rest}`].join("  ") : shown.join("  ");
});

/** The folder relative to the download folder when it is inside it. */
const folder = computed(() => {
  const full = props.category.default_output_path ?? "";
  const base = general.defaultOutputPath.value.replace(/[\\/]+$/, "");
  if (full === "" || full.replace(/[\\/]+$/, "") === base) {
    return { text: t("settings.categoriesFolderRoot"), full: base, isPath: false };
  }
  const inside = base !== "" && full.toLowerCase().startsWith(base.toLowerCase());
  const tail = inside ? full.slice(base.length) : "";
  return /^[\\/]/.test(tail)
    ? { text: tail, full, isPath: true }
    : { text: full, full, isPath: true };
});

function onGripKeydown(e: KeyboardEvent) {
  if (!e.altKey || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
  e.preventDefault();
  emit("move", props.category, e.key === "ArrowUp" ? -1 : 1);
}

const ICON_BUTTON =
  "flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
</script>

<template>
  <div
    role="row"
    :class="CATEGORY_GRID"
    class="h-[60px] border-b border-dl-line-soft bg-card last:border-b-0 hover:bg-dl-hover"
  >
    <span role="cell" class="flex justify-center">
      <Lock
        v-if="isFallback"
        class="h-3.5 w-3.5 text-dl-dim"
        role="img"
        :aria-label="t('settings.categoriesAlwaysLast')"
      />
      <button
        v-else
        type="button"
        class="drag-handle flex h-8 w-6 shrink-0 cursor-grab items-center justify-center rounded-md text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring active:cursor-grabbing"
        :aria-label="t('settings.categoriesReorder', { name: category.name })"
        :title="t('settings.categoriesReorderHint')"
        @keydown="onGripKeydown"
      >
        <GripVertical class="h-3.5 w-3.5" aria-hidden="true" />
      </button>
    </span>

    <span role="cell" class="flex min-w-0 items-center gap-3">
      <span
        class="flex h-8 w-8 shrink-0 items-center justify-center rounded-[9px]"
        :class="tint"
      >
        <component :is="icon.icon" class="h-[17px] w-[17px]" aria-hidden="true" />
      </span>
      <span class="flex min-w-0 flex-col gap-0.5">
        <span class="flex items-center gap-2">
          <span class="truncate font-medium">{{ category.name }}</span>
          <span
            v-if="isFallback"
            class="flex h-5 shrink-0 items-center rounded-[5px] bg-muted px-[7px] text-xs font-medium text-dl-soft"
          >
            {{ t("settings.categoriesFallback") }}
          </span>
        </span>
        <span class="truncate whitespace-pre font-mono text-xs text-muted-foreground">{{ extensions }}</span>
      </span>
    </span>

    <span
      role="cell"
      class="flex min-w-0 items-center gap-2 whitespace-nowrap text-xs"
      :class="folder.isPath ? 'font-mono text-dl-soft' : 'text-muted-foreground'"
      :title="folder.full"
    >
      <Folder class="h-3.5 w-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
      <span class="min-w-0 truncate">{{ folder.text }}</span>
    </span>

    <span
      role="cell"
      class="text-right font-mono text-xs"
      :class="count > 0 ? 'text-foreground' : 'text-dl-dim'"
    >
      {{ count }}
    </span>

    <span role="cell" class="flex justify-end gap-0.5">
      <button
        type="button"
        :class="ICON_BUTTON"
        :aria-label="t('settings.categoriesEditNamed', { name: category.name })"
        :title="t('common.edit')"
        @click="emit('edit', category)"
      >
        <Pencil class="h-[15px] w-[15px]" aria-hidden="true" />
      </button>
      <button
        type="button"
        :class="[ICON_BUTTON, isFallback ? 'cursor-not-allowed opacity-35' : '']"
        :aria-label="t('settings.categoriesDeleteNamed', { name: category.name })"
        :aria-disabled="isFallback || undefined"
        :title="isFallback ? t('settings.categoriesFallbackLocked') : t('common.delete')"
        @click="!isFallback && emit('remove', category)"
      >
        <Trash2 class="h-[15px] w-[15px]" aria-hidden="true" />
      </button>
    </span>
  </div>
</template>
