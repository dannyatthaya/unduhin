<script setup lang="ts">
// One setting: its name and a line about it on the left, the control on
// the right. `stacked` puts a wide control (a folder field) underneath.
// The row hides itself while a search is on and it is not a match.

import { computed } from "vue";

import HighlightText from "./HighlightText.vue";
import { useSettingsFilter } from "@/composables/useSettingsFilter";

const props = defineProps<{
  id: string;
  label: string;
  description?: string;
  /** Extra reason to hide the row, on top of the search. */
  hidden?: boolean;
  stacked?: boolean;
  /** Id of the control, when it is one a <label> can point at. */
  labelFor?: string;
}>();

const filter = useSettingsFilter();
const isHidden = computed(() => props.hidden || filter.isHidden(props.id));
/** For `aria-labelledby` on controls that are not form fields. */
const labelId = computed(() => `setting-${props.id.replace(/[^a-z0-9]+/gi, "-")}`);
</script>

<template>
  <div
    v-show="!isHidden"
    :data-setting-id="id"
    class="border-b border-dl-line-soft last:border-b-0"
    :class="
      stacked
        ? 'flex flex-col gap-2.5 px-[18px] py-3'
        : 'flex min-h-14 items-center justify-between gap-6 px-[18px] py-2.5'
    "
  >
    <div class="flex min-w-0 flex-1 flex-col gap-0.5">
      <component
        :is="labelFor ? 'label' : 'span'"
        :id="labelId"
        :for="labelFor"
        class="text-[13px] font-medium"
      >
        <HighlightText :text="label" />
      </component>
      <span v-if="description" class="text-xs leading-[1.45] text-muted-foreground">
        <HighlightText :text="description" />
      </span>
    </div>
    <slot v-if="stacked" :label-id="labelId" />
    <div v-else class="flex shrink-0 items-center gap-2.5">
      <slot :label-id="labelId" />
    </div>
  </div>
</template>
