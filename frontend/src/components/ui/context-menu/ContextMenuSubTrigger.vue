<script setup lang="ts">
import { computed } from "vue";
import { ChevronRight } from "lucide-vue-next";
import {
  ContextMenuSubTrigger,
  type ContextMenuSubTriggerProps,
  useForwardProps,
} from "reka-ui";

import { cn } from "@/lib/utils";

const props = defineProps<
  ContextMenuSubTriggerProps & { class?: string; inset?: boolean }
>();

const delegated = computed(() => {
  const { class: _, inset: __, ...rest } = props;
  return rest;
});

const forwarded = useForwardProps(delegated);
</script>

<template>
  <ContextMenuSubTrigger
    v-bind="forwarded"
    :class="
      cn(
        'flex cursor-default select-none items-center gap-2 rounded-sm px-2 py-1.5 text-sm outline-none transition-colors',
        'focus:bg-accent focus:text-accent-foreground data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground',
        'data-[state=open]:bg-accent data-[state=open]:text-accent-foreground',
        inset && 'pl-8',
        props.class,
      )
    "
  >
    <slot />
    <ChevronRight class="ml-auto h-4 w-4" />
  </ContextMenuSubTrigger>
</template>
