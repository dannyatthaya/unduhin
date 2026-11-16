<script setup lang="ts">
// The settings screen: the window title bar with the settings search, the
// section list, and one section (or the search results) on the right.

import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import { onKeyStroke } from "@vueuse/core";

import SettingsSearchResults from "./SettingsSearchResults.vue";
import SettingsSidebar from "@/components/settings/SettingsSidebar.vue";
import WindowTitleBar from "@/components/WindowTitleBar.vue";

import { useSettingsStore } from "@/stores/settings";
import { useCategoriesStore } from "@/stores/categories";
import { useSystemStore } from "@/stores/system";
import { useSettingsFilter } from "@/composables/useSettingsFilter";

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const filter = useSettingsFilter();

// Make sure the stores backing the Settings UI are populated when the
// user navigates here for the first time. These calls are idempotent:
// re-invoking them on later visits just re-fetches.
const settings = useSettingsStore();
const categories = useCategoriesStore();
const system = useSystemStore();

onMounted(() => {
  void settings.refresh();
  void categories.refresh();
  void system.refresh();
});

// The search belongs to one visit; coming back starts clean.
onBeforeUnmount(() => filter.reset());

const mainRef = ref<HTMLElement | null>(null);
const titleBarRef = ref<InstanceType<typeof WindowTitleBar> | null>(null);

watch([() => route.path, () => filter.active.value], () => {
  mainRef.value?.scrollTo({ top: 0, behavior: "instant" });
});

function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

/** A dialog, menu or list that is open owns Esc. */
function hasOpenOverlay(): boolean {
  return (
    document.querySelector(
      '[role="dialog"], [role="alertdialog"], [role="listbox"], [role="menu"]',
    ) !== null
  );
}

onKeyStroke("f", (e) => {
  if (!(e.ctrlKey || e.metaKey) || e.shiftKey || e.altKey) return;
  // Ctrl F inside another text box stays with that box.
  if (isTyping(e.target) && (e.target as HTMLElement).getAttribute("type") !== "search") return;
  e.preventDefault();
  titleBarRef.value?.focusSearch();
});

// Esc steps back: out of the search first, then out of Settings. The
// search field handles its own Esc and stops it from reaching here.
onKeyStroke("Escape", (e) => {
  if (e.defaultPrevented || isTyping(e.target) || hasOpenOverlay()) return;
  if (filter.active.value) filter.reset();
  else void router.push("/");
});
</script>

<template>
  <div class="dl-surface flex min-h-0 flex-1 flex-col bg-background text-[13px] text-foreground">
    <WindowTitleBar
      ref="titleBarRef"
      v-model:search="filter.query.value"
      :subtitle="t('downloads.settings')"
      :search-placeholder="t('settings.searchPlaceholder')"
      :search-label="t('settings.searchPlaceholder')"
      :clear-hint="t('downloads.searchClearHint')"
    />
    <div class="flex min-h-0 flex-1">
      <SettingsSidebar />
      <main ref="mainRef" class="min-w-0 flex-1 overflow-y-auto">
        <div class="mx-auto box-border max-w-[820px] px-12 pb-14 pt-8">
          <SettingsSearchResults v-if="filter.active.value" />
          <RouterView v-else />
        </div>
      </main>
    </div>
  </div>
</template>
