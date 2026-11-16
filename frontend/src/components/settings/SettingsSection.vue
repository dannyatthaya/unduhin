<script setup lang="ts">
// One settings page: a heading, one line of what the page is for, then
// its groups. Embedded in search results it renders the groups only.

import { useSettingsEmbed } from "./settingsEmbed";

defineProps<{
  title: string;
  description?: string;
}>();

const embed = useSettingsEmbed();
</script>

<template>
  <div v-if="embed" class="settings-embedded contents">
    <slot />
  </div>
  <div v-else class="flex flex-col gap-7">
    <header
      class="flex gap-6"
      :class="$slots.actions ? 'items-end justify-between' : 'flex-col'"
    >
      <div class="flex flex-col gap-1.5">
        <h1 class="text-2xl font-semibold tracking-[-0.02em]">{{ title }}</h1>
        <p v-if="description" class="max-w-[620px] text-sm leading-normal text-muted-foreground">
          {{ description }}
        </p>
      </div>
      <div v-if="$slots.actions" class="shrink-0">
        <slot name="actions" />
      </div>
    </header>
    <slot />
  </div>
</template>
