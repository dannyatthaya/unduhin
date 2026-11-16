<script setup lang="ts">
// Renders one section's page inside the search results. See settingsEmbed.ts.

import { provide } from "vue";

import { SETTINGS_EMBED_KEY } from "./settingsEmbed";
import type { SettingsSectionKey } from "@/lib/settingsManifest";

const props = defineProps<{ section: SettingsSectionKey }>();

provide(SETTINGS_EMBED_KEY, { section: props.section });
</script>

<template>
  <slot />
</template>
