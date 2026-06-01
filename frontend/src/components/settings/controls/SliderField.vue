<script setup lang="ts">
import { computed } from "vue";

import Slider from "@/components/ui/Slider.vue";

const props = defineProps<{
  modelValue: number;
  min?: number;
  max?: number;
  step?: number;
  disabled?: boolean;
  /** Render the trailing value as a short string (e.g. "8" or "1.5s"). */
  format?: (value: number) => string;
}>();

defineEmits<{ "update:modelValue": [value: number] }>();

const display = computed(() =>
  props.format ? props.format(props.modelValue) : String(props.modelValue),
);
</script>

<template>
  <div class="flex w-[20rem] max-w-full items-center gap-4">
    <Slider
      :model-value="modelValue"
      :min="min"
      :max="max"
      :step="step"
      :disabled="disabled"
      @update:model-value="$emit('update:modelValue', $event)"
    />
    <span class="w-12 shrink-0 text-right text-sm font-semibold tabular-nums">
      {{ display }}
    </span>
  </div>
</template>
