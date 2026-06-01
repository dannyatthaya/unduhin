<script setup lang="ts">
import { computed } from "vue";
import {
  ContextMenuContent,
  type ContextMenuContentEmits,
  type ContextMenuContentProps,
  ContextMenuPortal,
  useForwardPropsEmits,
} from "reka-ui";

import { cn } from "@/lib/utils";

const props = withDefaults(
  defineProps<ContextMenuContentProps & { class?: string }>(),
  {},
);

const emits = defineEmits<ContextMenuContentEmits>();

const delegated = computed(() => {
  const { class: _, ...rest } = props;
  return rest;
});

const forwarded = useForwardPropsEmits(delegated, emits);
</script>

<template>
  <ContextMenuPortal>
    <ContextMenuContent
      v-bind="forwarded"
      :class="
        cn(
          'z-50 min-w-[12rem] overflow-hidden rounded-md border border-border bg-card p-1 text-card-foreground shadow-md data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95',
          props.class,
        )
      "
    >
      <slot />
    </ContextMenuContent>
  </ContextMenuPortal>
</template>
