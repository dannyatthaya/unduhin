<script setup lang="ts">
// What the settings area shows while a search is typed: the matching rows
// from every section, each still the live control. Sections are the real
// pages, embedded; their rows and groups hide themselves when they do not
// match (SettingRow, and `.settings-group` in style.css).

import type { Component } from "vue";
import { useI18n } from "vue-i18n";
import { Search } from "lucide-vue-next";

import SettingsEmbed from "@/components/settings/SettingsEmbed.vue";
import Button from "@/components/ui/Button.vue";
import SettingsAbout from "./SettingsAbout.vue";
import SettingsBehaviour from "./SettingsBehaviour.vue";
import SettingsBrowser from "./SettingsBrowser.vue";
import SettingsCategories from "./SettingsCategories.vue";
import SettingsGeneral from "./SettingsGeneral.vue";
import SettingsMedia from "./SettingsMedia.vue";
import SettingsNetwork from "./SettingsNetwork.vue";
import SettingsTorrent from "./SettingsTorrent.vue";

import { useSettingsFilter } from "@/composables/useSettingsFilter";
import type { SettingsSectionKey } from "@/lib/settingsManifest";

const { t } = useI18n();
const filter = useSettingsFilter();

const PAGES: Record<SettingsSectionKey, Component> = {
  general: SettingsGeneral,
  categories: SettingsCategories,
  behaviour: SettingsBehaviour,
  network: SettingsNetwork,
  torrent: SettingsTorrent,
  media: SettingsMedia,
  browser: SettingsBrowser,
  about: SettingsAbout,
};
</script>

<template>
  <div class="settings-searching flex flex-col gap-7">
    <template v-if="filter.matched.value.length > 0">
      <header class="flex flex-col gap-1.5">
        <h1 class="text-2xl font-semibold tracking-[-0.02em]" aria-live="polite">
          {{
            t(
              "settings.searchCount",
              { n: filter.matched.value.length, query: filter.query.value.trim() },
              filter.matched.value.length,
            )
          }}
        </h1>
        <p class="max-w-[620px] text-sm leading-normal text-muted-foreground">
          {{ t("settings.searchLede") }}
        </p>
      </header>
      <SettingsEmbed
        v-for="section in filter.matchedSections.value"
        :key="section"
        :section="section"
      >
        <component :is="PAGES[section]" />
      </SettingsEmbed>
    </template>

    <template v-else>
      <header class="flex flex-col gap-1.5">
        <h1 class="text-2xl font-semibold tracking-[-0.02em]" aria-live="polite">
          {{ t("settings.searchNone", { query: filter.query.value.trim() }) }}
        </h1>
        <p class="max-w-[620px] text-sm leading-normal text-muted-foreground">
          {{ t("settings.searchNoneLede") }}
        </p>
      </header>
      <div class="flex items-start gap-3 rounded-xl border border-dl-line bg-card px-[18px] py-3.5">
        <Search class="mt-px h-[18px] w-[18px] shrink-0 text-muted-foreground" aria-hidden="true" />
        <div class="flex flex-1 flex-col gap-2.5">
          <i18n-t
            v-if="filter.suggestion.value"
            keypath="settings.searchDidYouMean"
            tag="span"
            :plural="filter.suggestion.value.count"
          >
            <template #word>
              <b class="font-semibold">{{ filter.suggestion.value.word }}</b>
            </template>
            <template #n>{{ filter.suggestion.value.count }}</template>
          </i18n-t>
          <span v-else>{{ t("settings.searchTryOther") }}</span>
          <span class="flex gap-2">
            <Button
              v-if="filter.suggestion.value"
              @click="filter.query.value = filter.suggestion.value.word"
            >
              {{ t("settings.searchFor", { word: filter.suggestion.value.word }) }}
            </Button>
            <Button variant="ghost" class="px-2.5" @click="filter.reset()">
              {{ t("downloads.searchClear") }}
            </Button>
          </span>
        </div>
      </div>
      <p class="px-0.5 text-xs text-muted-foreground">{{ t("settings.searchPickSection") }}</p>
    </template>
  </div>
</template>
