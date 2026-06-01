<script setup lang="ts">
import Switch from "@/components/ui/Switch.vue";

defineProps<{
  modelValue: boolean;
  disabled?: boolean;
  ariaLabel?: string;
}>();
defineEmits<{ "update:modelValue": [value: boolean] }>();
</script>

<template>
  <Switch
    :model-value="modelValue"
    :disabled="disabled"
    :aria-label="ariaLabel"
    @update:model-value="$emit('update:modelValue', $event)"
  />
</template>
