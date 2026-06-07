<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { ArrowLeft, Search, X } from "lucide-vue-next";
import {
  Settings,
  Folder,
  Globe,
  ArrowRight,
  Code2,
  Film,
  Info,
  Magnet,
} from "lucide-vue-next";
import type { Component } from "vue";

import { useSystemStore } from "@/stores/system";
import { useSettingsFilter } from "@/composables/useSettingsFilter";
import {
  SECTION_LABELS,
  SECTION_ROUTES,
  type SettingsSectionKey,
} from "@/lib/settingsManifest";

const system = useSystemStore();
const filter = useSettingsFilter();

interface NavItem {
  key: SettingsSectionKey;
  label: string;
  icon: Component;
  badge?: string;
}

const items: NavItem[] = [
  { key: "general", label: SECTION_LABELS.general, icon: Settings },
  { key: "categories", label: SECTION_LABELS.categories, icon: Folder },
  { key: "behaviour", label: SECTION_LABELS.behaviour, icon: Globe },
  { key: "network", label: SECTION_LABELS.network, icon: ArrowRight },
  { key: "torrent", label: SECTION_LABELS.torrent, icon: Magnet },
];

const integrationItems: NavItem[] = [
  { key: "media", label: SECTION_LABELS.media, icon: Film },
  { key: "browser", label: SECTION_LABELS.browser, icon: Globe, badge: "NEW" },
];

interface DisabledItem {
  label: string;
  icon: Component;
  badge: string;
}

const integrations: DisabledItem[] = [
  { label: "Advanced", icon: Code2, badge: "SOON" },
];

const systemNavItems: NavItem[] = [
  { key: "about", label: SECTION_LABELS.about, icon: Info },
];

const searchInput = ref<HTMLInputElement | null>(null);
const searchFocused = ref(false);

function focusSearch() {
  searchInput.value?.focus();
  searchInput.value?.select();
}

function onKeyDown(e: KeyboardEvent) {
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
    const t = e.target as HTMLElement | null;
    // Don't steal Ctrl-F from inside other text inputs — the user might be
    // searching within a textarea (e.g. the user-agent override).
    if (
      t &&
      (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable) &&
      t !== searchInput.value
    ) {
      return;
    }
    e.preventDefault();
    focusSearch();
  }
}

onMounted(() => {
  window.addEventListener("keydown", onKeyDown);
});
onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeyDown);
});

const version = computed(() => system.appInfo?.version ?? "—");
const channel = computed(() => system.appInfo?.channel ?? "stable");
</script>

<template>
  <aside
    class="flex h-full w-64 shrink-0 flex-col border-r border-border bg-card text-card-foreground"
  >
    <div class="flex flex-col gap-4 px-4 py-4">
      <RouterLink
        to="/"
        class="inline-flex items-center gap-2 text-sm text-muted-foreground transition-colors hover:text-foreground"
      >
        <ArrowLeft class="h-4 w-4" />
        <span>Downloads</span>
      </RouterLink>

      <div class="relative">
        <Search
          class="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground"
        />
        <input
          ref="searchInput"
          v-model="filter.query.value"
          type="text"
          placeholder="Filter settings…"
          class="h-9 w-full rounded-md border border-input bg-background pl-8 pr-12 text-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          @focus="searchFocused = true"
          @blur="searchFocused = false"
        />
        <button
          v-if="filter.query.value"
          type="button"
          title="Clear filter"
          class="absolute right-2 top-1/2 -translate-y-1/2 flex h-5 w-5 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          @mousedown.prevent
          @click="filter.reset(); focusSearch()"
        >
          <X class="h-3.5 w-3.5" />
        </button>
        <span
          v-else-if="!searchFocused"
          class="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 rounded border border-border bg-muted px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
        >
          Ctrl-F
        </span>
      </div>
    </div>

    <nav class="flex flex-1 flex-col gap-4 overflow-y-auto px-2 pb-4">
      <div class="flex flex-col gap-0.5">
        <span
          class="px-3 py-1.5 text-[10px] font-semibold uppercase tracking-[0.15em] text-muted-foreground"
        >
          Preferences
        </span>
        <RouterLink
          v-for="item in items"
          :key="item.key"
          :to="SECTION_ROUTES[item.key]"
          class="flex items-center gap-2.5 rounded-md px-3 py-2 text-sm transition-colors"
          active-class="bg-primary/10 text-primary"
          :class="
            filter.query.value && !filter.matchedSections.value.has(item.key)
              ? 'opacity-40'
              : 'text-foreground hover:bg-accent hover:text-accent-foreground'
          "
        >
          <component :is="item.icon" class="h-4 w-4" />
          <span>{{ item.label }}</span>
        </RouterLink>
      </div>

      <div class="flex flex-col gap-0.5">
        <span
          class="px-3 py-1.5 text-[10px] font-semibold uppercase tracking-[0.15em] text-muted-foreground"
        >
          Integrations
        </span>
        <RouterLink
          v-for="item in integrationItems"
          :key="item.key"
          :to="SECTION_ROUTES[item.key]"
          class="flex items-center justify-between gap-2 rounded-md px-3 py-2 text-sm transition-colors"
          active-class="bg-primary/10 text-primary"
          :class="
            filter.query.value && !filter.matchedSections.value.has(item.key)
              ? 'opacity-40'
              : 'text-foreground hover:bg-accent hover:text-accent-foreground'
          "
        >
          <span class="flex items-center gap-2.5">
            <component :is="item.icon" class="h-4 w-4" />
            <span>{{ item.label }}</span>
          </span>
          <span
            v-if="item.badge"
            class="rounded bg-primary/15 px-1.5 py-0.5 font-mono text-[10px] text-primary"
          >
            {{ item.badge }}
          </span>
        </RouterLink>
        <button
          v-for="item in integrations"
          :key="item.label"
          type="button"
          disabled
          class="flex cursor-not-allowed items-center justify-between gap-2 rounded-md px-3 py-2 text-sm text-muted-foreground"
        >
          <span class="flex items-center gap-2.5">
            <component :is="item.icon" class="h-4 w-4" />
            <span>{{ item.label }}</span>
          </span>
          <span class="rounded bg-muted px-1.5 py-0.5 font-mono text-[10px]">
            {{ item.badge }}
          </span>
        </button>
      </div>

      <div class="flex flex-col gap-0.5">
        <span
          class="px-3 py-1.5 text-[10px] font-semibold uppercase tracking-[0.15em] text-muted-foreground"
        >
          System
        </span>
        <RouterLink
          v-for="item in systemNavItems"
          :key="item.key"
          :to="SECTION_ROUTES[item.key]"
          class="flex items-center gap-2.5 rounded-md px-3 py-2 text-sm transition-colors"
          active-class="bg-primary/10 text-primary"
          :class="
            filter.query.value && !filter.matchedSections.value.has(item.key)
              ? 'opacity-40'
              : 'text-foreground hover:bg-accent hover:text-accent-foreground'
          "
        >
          <component :is="item.icon" class="h-4 w-4" />
          <span>{{ item.label }}</span>
        </RouterLink>
      </div>
    </nav>

    <footer class="border-t border-border px-4 py-3 text-[11px] text-muted-foreground">
      <div>
        <span class="font-mono">v{{ version }}</span>
        <span> · {{ channel }} channel</span>
      </div>
    </footer>
  </aside>
</template>
