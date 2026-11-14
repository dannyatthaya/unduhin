<script setup lang="ts">
// Title bar for the downloads page: brand, the search field, and the
// window controls in one 44px strip. The Settings routes keep the shared
// `AppTitleBar`.
//
// Platform handling mirrors `AppTitleBar`: on macOS the system draws its
// traffic lights over the left edge, so our own buttons are hidden and
// the brand is inset to clear them. Dragging is Tauri's
// `data-tauri-drag-region`, which applies to the marked element only —
// the search field stays clickable.

import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { Copy, Minus, Search, Square, X } from "lucide-vue-next";

import { useWindowControls } from "@/composables/useWindowControls";
import { isMacOS } from "@/lib/platform";

const props = defineProps<{
  search: string;
  /** Narrow window: the brand collapses to its mark. */
  compact?: boolean;
}>();

const emit = defineEmits<{ "update:search": [value: string] }>();

const { t } = useI18n();
const { isMaximized, minimize, toggleMaximize, close } = useWindowControls();

const inputRef = ref<HTMLInputElement | null>(null);
const hasSearch = computed(() => props.search !== "");
const shortcutLabel = isMacOS ? "⌘F" : "Ctrl F";

function focusSearch() {
  inputRef.value?.focus();
  inputRef.value?.select();
}

function clearSearch() {
  emit("update:search", "");
  inputRef.value?.focus();
}

// Esc inside the field belongs to the field: clear the query first, then
// give focus back to the list. Stopping propagation keeps the page-level
// Esc chain (selection → details) from also firing.
function onEscape(e: KeyboardEvent) {
  e.stopPropagation();
  if (hasSearch.value) emit("update:search", "");
  else inputRef.value?.blur();
}

defineExpose({ focusSearch });

const WINDOW_BUTTON =
  "flex h-11 w-[46px] items-center justify-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring";
</script>

<template>
  <header
    data-tauri-drag-region
    class="flex h-11 shrink-0 select-none items-center gap-4 border-b border-dl-line bg-dl-chrome"
    :class="isMacOS ? 'pl-[78px] pr-4' : 'pl-4'"
  >
    <div
      data-tauri-drag-region
      class="flex shrink-0 items-center gap-2.5 font-semibold tracking-[-0.01em]"
      :class="compact ? 'basis-6' : 'basis-[200px]'"
    >
      <span
        class="pointer-events-none flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-[7px] bg-dl-accent text-dl-on-accent"
      >
        <svg
          width="13"
          height="13"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="3"
          stroke-linecap="round"
          aria-hidden="true"
        >
          <path d="M6 4v9a6 6 0 0 0 12 0V4" />
        </svg>
      </span>
      <span v-if="!compact" class="pointer-events-none">Unduhin</span>
    </div>

    <label
      class="mx-auto flex h-[30px] min-w-0 shrink items-center gap-2 rounded-lg border border-input bg-dl-field pl-2.5 pr-1.5 text-muted-foreground focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-ring"
      :class="compact ? 'basis-[340px]' : 'basis-[460px]'"
    >
      <Search class="h-[15px] w-[15px] shrink-0" aria-hidden="true" />
      <input
        ref="inputRef"
        type="search"
        :value="search"
        :placeholder="t('downloads.searchPlaceholder')"
        :aria-label="t('downloads.searchAria')"
        class="min-w-0 flex-1 select-text border-0 bg-transparent text-foreground outline-none placeholder:text-muted-foreground [&::-webkit-search-cancel-button]:appearance-none"
        @input="emit('update:search', ($event.target as HTMLInputElement).value)"
        @keydown.esc="onEscape"
      />
      <button
        v-if="hasSearch"
        type="button"
        class="flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-md bg-muted text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
        :aria-label="t('downloads.searchClear')"
        :title="t('downloads.searchClearHint')"
        @click="clearSearch"
      >
        <X class="h-[11px] w-[11px]" aria-hidden="true" />
      </button>
      <kbd
        v-else
        class="shrink-0 rounded border border-border px-[5px] text-[11px] text-muted-foreground"
      >
        {{ shortcutLabel }}
      </kbd>
    </label>

    <!-- macOS draws its own traffic lights over this bar. -->
    <div v-if="!isMacOS" class="flex shrink-0">
      <button
        type="button"
        :class="WINDOW_BUTTON"
        :aria-label="t('downloads.titlebarMinimize')"
        :title="t('downloads.titlebarMinimize')"
        @click="minimize"
      >
        <Minus class="h-3.5 w-3.5" aria-hidden="true" />
      </button>
      <button
        type="button"
        :class="WINDOW_BUTTON"
        :aria-label="isMaximized ? t('downloads.titlebarRestore') : t('downloads.titlebarMaximize')"
        :title="isMaximized ? t('downloads.titlebarRestore') : t('downloads.titlebarMaximize')"
        @click="toggleMaximize"
      >
        <Copy v-if="isMaximized" class="h-3 w-3 -scale-x-100" aria-hidden="true" />
        <Square v-else class="h-3 w-3" aria-hidden="true" />
      </button>
      <button
        type="button"
        :class="WINDOW_BUTTON"
        :aria-label="t('downloads.titlebarClose')"
        :title="t('downloads.titlebarClose')"
        @click="close"
      >
        <X class="h-3.5 w-3.5" aria-hidden="true" />
      </button>
    </div>
  </header>
</template>
