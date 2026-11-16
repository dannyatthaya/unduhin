<script setup lang="ts">
// The settings sections, grouped. While a search is on, each section shows
// how many settings match and the rest dim; picking a section clears the
// search and opens it.

import { computed, type Component } from "vue";
import { useI18n } from "vue-i18n";
import {
  ArrowLeft,
  CheckCircle2,
  Clapperboard,
  Globe,
  Info,
  LayoutGrid,
  Magnet,
  Power,
  Puzzle,
  Settings,
} from "lucide-vue-next";

import { useSystemStore } from "@/stores/system";
import { useSettingsFilter } from "@/composables/useSettingsFilter";
import {
  SECTION_LABEL_KEYS,
  SECTION_ROUTES,
  type SettingsSectionKey,
} from "@/lib/settingsManifest";

const { t } = useI18n();
const system = useSystemStore();
const filter = useSettingsFilter();

interface NavItem {
  key: SettingsSectionKey;
  icon: Component;
  /** A short tag after the name, e.g. "New". */
  tagKey?: string;
}

const GROUPS: { labelKey: string; items: NavItem[] }[] = [
  {
    labelKey: "settings.navPreferences",
    items: [
      { key: "general", icon: Settings },
      { key: "categories", icon: LayoutGrid },
      { key: "behaviour", icon: Power },
      { key: "network", icon: Globe },
      { key: "torrent", icon: Magnet },
    ],
  },
  {
    labelKey: "settings.navIntegrations",
    items: [
      { key: "media", icon: Clapperboard },
      { key: "browser", icon: Puzzle, tagKey: "settings.navNew" },
    ],
  },
  { labelKey: "settings.navSystem", items: [{ key: "about", icon: Info }] },
];

const versionLine = computed(() =>
  system.appInfo
    ? t("settings.versionLine", {
        version: system.appInfo.version,
        channel: system.appInfo.channel ?? "stable",
      })
    : "",
);

function matchCount(key: SettingsSectionKey): number {
  return filter.countBySection.value.get(key) ?? 0;
}

const FOCUS_RING =
  "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
</script>

<template>
  <aside
    :aria-label="t('settings.sidebarAria')"
    class="flex w-60 shrink-0 flex-col gap-[18px] border-r border-dl-line bg-dl-chrome px-3 pt-3.5"
  >
    <RouterLink
      to="/"
      class="flex h-9 shrink-0 items-center gap-2.5 rounded-lg px-2.5 font-medium text-foreground transition-colors hover:bg-accent"
      :class="FOCUS_RING"
    >
      <ArrowLeft class="h-4 w-4" aria-hidden="true" />
      <span class="flex-1">{{ t("settings.backToDownloads") }}</span>
      <kbd class="text-xs text-muted-foreground">Esc</kbd>
    </RouterLink>

    <nav :aria-label="t('downloads.settings')" class="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto">
      <div v-for="group in GROUPS" :key="group.labelKey" class="flex flex-col gap-0.5">
        <p class="mb-1.5 ml-2.5 text-xs font-semibold text-muted-foreground">
          {{ t(group.labelKey) }}
        </p>
        <RouterLink
          v-for="item in group.items"
          :key="item.key"
          v-slot="{ isActive, href, navigate }"
          :to="SECTION_ROUTES[item.key]"
          custom
        >
          <a
            :href="href"
            :aria-current="isActive && !filter.active.value ? 'page' : undefined"
            :title="
              filter.active.value
                ? t('settings.navClearAndOpen', { section: t(SECTION_LABEL_KEYS[item.key]) })
                : undefined
            "
            class="flex h-9 items-center gap-2.5 rounded-lg px-2.5 transition-colors"
            :class="[
              FOCUS_RING,
              isActive && !filter.active.value
                ? 'bg-dl-accent-soft font-semibold text-foreground shadow-[inset_0_0_0_1px_var(--dl-accent-ring)]'
                : filter.active.value && matchCount(item.key) === 0
                  ? 'text-dl-dim hover:bg-accent'
                  : 'text-dl-soft hover:bg-accent',
            ]"
            @click="
              (e) => {
                filter.reset();
                navigate(e);
              }
            "
          >
            <component :is="item.icon" class="h-[17px] w-[17px] shrink-0" aria-hidden="true" />
            <span class="flex-1 truncate">{{ t(SECTION_LABEL_KEYS[item.key]) }}</span>
            <span
              v-if="filter.active.value && matchCount(item.key) > 0"
              class="min-w-5 rounded-[10px] border border-input px-1.5 text-center font-mono text-xs text-dl-soft"
              :aria-label="t('settings.navMatches', { n: matchCount(item.key) }, matchCount(item.key))"
            >
              {{ matchCount(item.key) }}
            </span>
            <span
              v-else-if="!filter.active.value && item.tagKey"
              class="rounded-[5px] bg-dl-accent-soft px-[7px] py-px text-xs font-medium text-primary"
            >
              {{ t(item.tagKey) }}
            </span>
          </a>
        </RouterLink>
      </div>
    </nav>

    <div
      class="-mx-3 flex shrink-0 flex-col gap-1.5 border-t border-dl-line px-[22px] pb-3 pt-3 text-xs text-muted-foreground"
    >
      <span class="flex items-center gap-2">
        <CheckCircle2 class="h-3.5 w-3.5 text-success" aria-hidden="true" />
        {{ t("settings.savesAsYouGo") }}
      </span>
      <span v-if="versionLine" class="font-mono">{{ versionLine }}</span>
    </div>
  </aside>
</template>
