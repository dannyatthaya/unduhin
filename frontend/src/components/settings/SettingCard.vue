<script setup lang="ts">
// A group of settings: a small heading over one bordered card of rows.
// In search results the heading becomes a breadcrumb back to its section,
// and a group with no matching row hides itself (see `.settings-group`
// in style.css).

import { computed, useId } from "vue";
import { useI18n } from "vue-i18n";
import { ChevronRight } from "lucide-vue-next";

import { useSettingsEmbed } from "./settingsEmbed";
import { useSettingsFilter } from "@/composables/useSettingsFilter";
import { SECTION_LABEL_KEYS, SECTION_ROUTES } from "@/lib/settingsManifest";

defineProps<{
  title: string;
  description?: string;
  /** One muted line under the card. */
  note?: string;
}>();

const { t } = useI18n();
const embed = useSettingsEmbed();
const filter = useSettingsFilter();
const headingId = useId();

const crumb = computed(() =>
  embed
    ? { label: t(SECTION_LABEL_KEYS[embed.section]), route: SECTION_ROUTES[embed.section] }
    : null,
);
</script>

<template>
  <section class="settings-group flex flex-col gap-2.5" :aria-labelledby="headingId">
    <div
      v-if="crumb"
      class="flex items-center gap-1.5 px-0.5 text-xs text-muted-foreground"
    >
      <RouterLink
        :to="crumb.route"
        class="rounded font-medium text-dl-soft hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
        @click="filter.reset()"
      >
        {{ crumb.label }}
      </RouterLink>
      <ChevronRight class="h-[11px] w-[11px]" aria-hidden="true" />
      <span :id="headingId">{{ title }}</span>
    </div>
    <template v-else>
      <div
        class="flex justify-between gap-4 px-0.5"
        :class="$slots.actions ? '-my-1 items-center' : 'items-baseline'"
      >
        <h2
          :id="headingId"
          class="text-xs font-semibold uppercase tracking-[0.06em] text-muted-foreground"
        >
          {{ title }}
        </h2>
        <slot name="actions" />
      </div>
      <p v-if="description" class="-mt-1 px-0.5 text-xs leading-[1.45] text-muted-foreground">
        {{ description }}
      </p>
    </template>
    <div class="flex flex-col overflow-hidden rounded-xl border border-dl-line bg-card">
      <slot />
    </div>
    <p v-if="note && !crumb" class="px-0.5 text-xs text-muted-foreground">{{ note }}</p>
  </section>
</template>
