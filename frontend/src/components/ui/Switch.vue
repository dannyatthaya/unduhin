<script setup lang="ts">
import { SwitchRoot, SwitchThumb } from "reka-ui";

import { cn } from "@/lib/utils";

const props = defineProps<{
  modelValue: boolean;
  disabled?: boolean;
  ariaLabel?: string;
  class?: string;
}>();

const emit = defineEmits<{ "update:modelValue": [value: boolean] }>();
</script>

<template>
  <SwitchRoot
    :model-value="modelValue"
    :disabled="disabled"
    :aria-label="ariaLabel"
    :class="
      cn(
        'peer relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:cursor-not-allowed disabled:opacity-50 data-[state=checked]:bg-primary data-[state=unchecked]:bg-muted',
        props.class,
      )
    "
    @update:model-value="(v) => emit('update:modelValue', v)"
  >
    <SwitchThumb
      class="pointer-events-none block h-4 w-4 rounded-full bg-white shadow-lg ring-0 transition-transform data-[state=checked]:translate-x-4 data-[state=unchecked]:translate-x-0"
    />
  </SwitchRoot>
</template>
