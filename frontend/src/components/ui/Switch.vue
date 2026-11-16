<script setup lang="ts">
import { SwitchRoot, SwitchThumb } from "reka-ui";

import { cn } from "@/lib/utils";

const props = defineProps<{
  modelValue: boolean;
  disabled?: boolean;
  ariaLabel?: string;
  /** Id of the visible text that names this switch. */
  labelledby?: string;
  class?: string;
}>();

const emit = defineEmits<{ "update:modelValue": [value: boolean] }>();
</script>

<template>
  <SwitchRoot
    :model-value="modelValue"
    :disabled="disabled"
    :aria-label="ariaLabel"
    :aria-labelledby="labelledby"
    :class="
      cn(
        'relative inline-flex h-6 w-10 shrink-0 cursor-pointer items-center rounded-full px-[3px] transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:cursor-not-allowed disabled:opacity-45 data-[state=checked]:bg-primary data-[state=unchecked]:bg-input',
        props.class,
      )
    "
    @update:model-value="(v) => emit('update:modelValue', v)"
  >
    <SwitchThumb
      class="pointer-events-none block h-[18px] w-[18px] rounded-full transition-transform data-[state=checked]:translate-x-4 data-[state=checked]:bg-(--dl-switch-knob) data-[state=unchecked]:translate-x-0 data-[state=unchecked]:bg-dl-knob-off"
    />
  </SwitchRoot>
</template>
