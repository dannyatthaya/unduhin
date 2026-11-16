<script setup lang="ts">
// Text with the current settings search marked in it.

import { computed } from "vue";

import { useSettingsFilter } from "@/composables/useSettingsFilter";
import { splitByQuery } from "@/lib/settingsSearch";

const props = defineProps<{ text: string }>();

const filter = useSettingsFilter();
const pieces = computed(() => splitByQuery(props.text, filter.query.value));
</script>

<template>
  <template v-for="(piece, index) in pieces" :key="index">
    <mark v-if="piece.match" class="rounded-[3px] bg-dl-mark px-px text-inherit">{{ piece.text }}</mark>
    <template v-else>{{ piece.text }}</template>
  </template>
</template>
