<script setup lang="ts">
import { computed } from "vue";
import {
  ContextMenuSeparator,
  type ContextMenuSeparatorProps,
  useForwardProps,
} from "reka-ui";

import { cn } from "@/lib/utils";

const props = defineProps<ContextMenuSeparatorProps & { class?: string }>();

const delegated = computed(() => {
  const { class: _, ...rest } = props;
  return rest;
});

const forwarded = useForwardProps(delegated);
</script>

<template>
  <ContextMenuSeparator
    v-bind="forwarded"
    :class="cn('-mx-1 my-1 h-px bg-border', props.class)"
  />
</template>
