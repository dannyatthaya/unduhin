<script setup lang="ts">
// Centred empty states for the downloads list.
//
// - "welcome": no downloads at all (first run) — the two ways to start.
// - "search": a search with no matches — offers to clear it.
// - "filtered" / a status: the category or status tab is simply empty.

import { computed, type Component } from "vue";
import { useI18n } from "vue-i18n";
import {
  AlertTriangle,
  ArrowDownUp,
  CheckCircle2,
  Clock,
  Download,
  Filter,
  PauseCircle,
  Plug,
  Plus,
  Search,
  X,
  XCircle,
} from "lucide-vue-next";

import { useSettingsStore } from "@/stores/settings";
import { isMacOS } from "@/lib/platform";
import type { Status } from "@/types/tauri-bindings";

export type EmptyVariant = Status | "welcome" | "search" | "filtered";

const props = withDefaults(
  defineProps<{
    variant?: EmptyVariant;
    /** The search text, shown in the "search" variant's title. */
    query?: string;
  }>(),
  { variant: "welcome", query: "" },
);

const emit = defineEmits<{ "add-url": []; "clear-search": [] }>();

const { t } = useI18n();
const settings = useSettingsStore();

type FilterVariant = Exclude<EmptyVariant, "welcome" | "search">;

const FILTER_STATES: Record<FilterVariant, { icon: Component; key: string }> = {
  active: { icon: ArrowDownUp, key: "Active" },
  queued: { icon: Clock, key: "Queued" },
  paused: { icon: PauseCircle, key: "Paused" },
  completed: { icon: CheckCircle2, key: "Completed" },
  failed: { icon: AlertTriangle, key: "Failed" },
  cancelled: { icon: XCircle, key: "Cancelled" },
  muxing: { icon: ArrowDownUp, key: "Muxing" },
  filtered: { icon: Filter, key: "Filtered" },
};

const content = computed<{ icon: Component; title: string; body: string }>(() => {
  if (props.variant === "welcome") {
    return {
      icon: Download,
      title: t("downloads.emptyFirstTitle"),
      body: t("downloads.emptyFirstBody"),
    };
  }
  if (props.variant === "search") {
    return {
      icon: Search,
      title: t("downloads.emptySearchTitle", { query: props.query }),
      body: t("downloads.emptySearchBody"),
    };
  }
  const state = FILTER_STATES[props.variant];
  return {
    icon: state.icon,
    title: t(`downloads.empty${state.key}Title`),
    body: t(`downloads.empty${state.key}Hint`),
  };
});

// The tip describes the clipboard watcher, which is opt-in — only promise
// it when it is actually on.
const showClipboardTip = computed(
  () => props.variant === "welcome" && settings.values["watch_clipboard"] === true,
);

const newShortcut = isMacOS ? "⌘N" : "Ctrl N";

const BUTTON =
  "flex h-10 items-center gap-2 rounded-[10px] px-4 transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
const SECONDARY = "border border-border bg-dl-raised font-medium text-foreground hover:bg-accent";
</script>

<template>
  <div class="flex min-h-0 flex-1 items-center justify-center px-6 py-12">
    <section class="flex max-w-[440px] flex-col items-center gap-3.5 text-center">
      <span
        class="flex h-14 w-14 items-center justify-center rounded-2xl bg-dl-accent-soft text-primary shadow-[inset_0_0_0_1px_var(--dl-accent-ring)]"
      >
        <component :is="content.icon" class="h-[26px] w-[26px]" aria-hidden="true" />
      </span>
      <h2 class="mt-1 text-xl font-semibold tracking-[-0.01em]">
        {{ content.title }}
      </h2>
      <p class="text-sm leading-normal text-muted-foreground">
        {{ content.body }}
      </p>

      <div
        v-if="variant === 'welcome' || variant === 'search'"
        class="mt-2 flex flex-wrap justify-center gap-2"
      >
        <template v-if="variant === 'welcome'">
          <button
            type="button"
            :class="[BUTTON, 'bg-dl-accent font-semibold text-dl-on-accent hover:brightness-95']"
            @click="emit('add-url')"
          >
            <Plus class="h-4 w-4" aria-hidden="true" />
            {{ t("downloads.newDownload") }}
            <kbd class="text-[11px] opacity-75">{{ newShortcut }}</kbd>
          </button>
          <RouterLink :to="{ name: 'settings-browser' }" :class="[BUTTON, SECONDARY]">
            <Plug class="h-4 w-4" aria-hidden="true" />
            {{ t("downloads.emptyConnectExtension") }}
          </RouterLink>
        </template>
        <button
          v-else
          type="button"
          :class="[BUTTON, SECONDARY]"
          @click="emit('clear-search')"
        >
          <X class="h-3.5 w-3.5" aria-hidden="true" />
          {{ t("downloads.searchClear") }}
          <kbd class="text-[11px] text-muted-foreground">Esc</kbd>
        </button>
      </div>

      <p v-if="showClipboardTip" class="mt-2.5 text-xs text-muted-foreground">
        {{ t("downloads.emptyClipboardTip") }}
      </p>
    </section>
  </div>
</template>
