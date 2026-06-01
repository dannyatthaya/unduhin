<script setup lang="ts">
import { computed } from "vue";
import {
  ContextMenuItem,
  type ContextMenuItemEmits,
  type ContextMenuItemProps,
  useForwardPropsEmits,
} from "reka-ui";

import { cn } from "@/lib/utils";

const props = defineProps<
  ContextMenuItemProps & {
    class?: string;
    /** Indent the item to align with rows that have a leading indicator. */
    inset?: boolean;
    /** Style as a destructive action (red text). */
    variant?: "default" | "danger";
  }
>();

const emits = defineEmits<ContextMenuItemEmits>();

const delegated = computed(() => {
  const { class: _, inset: __, variant: ___, ...rest } = props;
  return rest;
});

const forwarded = useForwardPropsEmits(delegated, emits);
</script>

<template>
  <ContextMenuItem
    v-bind="forwarded"
    :class="
      cn(
        'relative flex cursor-default select-none items-center gap-2 rounded-sm px-2 py-1.5 text-sm outline-none transition-colors',
        'data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground',
        'data-[disabled]:pointer-events-none data-[disabled]:opacity-50',
        inset && 'pl-8',
        variant === 'danger' &&
          'text-danger data-[highlighted]:bg-danger data-[highlighted]:text-white',
        props.class,
      )
    "
  >
    <slot />
  </ContextMenuItem>
</template>
