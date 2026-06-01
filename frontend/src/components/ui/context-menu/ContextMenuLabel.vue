<script setup lang="ts">
import { computed } from "vue";
import { ContextMenuLabel, type ContextMenuLabelProps, useForwardProps } from "reka-ui";

import { cn } from "@/lib/utils";

const props = defineProps<
  ContextMenuLabelProps & { class?: string; inset?: boolean }
>();

const delegated = computed(() => {
  const { class: _, inset: __, ...rest } = props;
  return rest;
});

const forwarded = useForwardProps(delegated);
</script>

<template>
  <ContextMenuLabel
    v-bind="forwarded"
    :class="
      cn(
        'px-2 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground',
        inset && 'pl-8',
        props.class,
      )
    "
  >
    <slot />
  </ContextMenuLabel>
</template>
