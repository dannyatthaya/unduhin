<script setup lang="ts">
import Switch from "@/components/ui/Switch.vue";

defineProps<{
  modelValue: boolean;
  disabled?: boolean;
  ariaLabel?: string;
  /** Id of the visible text that names this switch. */
  labelledby?: string;
}>();
defineEmits<{ "update:modelValue": [value: boolean] }>();
</script>

<template>
  <Switch
    :model-value="modelValue"
    :disabled="disabled"
    :aria-label="ariaLabel"
    :labelledby="labelledby"
    @update:model-value="$emit('update:modelValue', $event)"
  />
</template>
