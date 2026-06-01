<script setup lang="ts">
import { computed } from "vue";
import {
  SliderRange,
  SliderRoot,
  SliderThumb,
  SliderTrack,
} from "reka-ui";

import { cn } from "@/lib/utils";

const props = withDefaults(
  defineProps<{
    modelValue: number;
    min?: number;
    max?: number;
    step?: number;
    disabled?: boolean;
    class?: string;
  }>(),
  { min: 0, max: 100, step: 1 },
);

const emit = defineEmits<{ "update:modelValue": [value: number] }>();

// reka-ui's Slider is multi-thumb by design (`modelValue` is an array
// of numbers). Lift our scalar API into the array shape it expects, and
// unwrap on the way back out.
const arrayValue = computed(() => [props.modelValue]);
</script>

<template>
  <SliderRoot
    :model-value="arrayValue"
    :min="min"
    :max="max"
    :step="step"
    :disabled="disabled"
    :class="
      cn(
        'relative flex w-full touch-none select-none items-center',
        props.class,
      )
    "
    @update:model-value="(v) => v != null && emit('update:modelValue', v[0])"
  >
    <SliderTrack
      class="relative h-1.5 w-full grow overflow-hidden rounded-full bg-muted"
    >
      <SliderRange class="absolute h-full bg-primary" />
    </SliderTrack>
    <SliderThumb
      class="block h-4 w-4 rounded-full border-2 border-primary bg-background shadow transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50"
      aria-label="Value"
    />
  </SliderRoot>
</template>
