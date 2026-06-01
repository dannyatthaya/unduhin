<script setup lang="ts">
import { ICON_PICKER_OPTIONS } from "@/lib/categoryIcons";

defineProps<{
  modelValue: string;
  disabled?: boolean;
}>();
const emit = defineEmits<{ "update:modelValue": [value: string] }>();
</script>

<template>
  <div class="flex flex-wrap gap-2">
    <button
      v-for="opt in ICON_PICKER_OPTIONS"
      :key="opt.key"
      type="button"
      :disabled="disabled"
      :aria-pressed="modelValue === opt.key"
      :title="opt.label"
      class="flex h-10 w-10 items-center justify-center rounded-md border transition-colors disabled:cursor-not-allowed disabled:opacity-50"
      :class="[
        opt.background,
        modelValue === opt.key
          ? 'border-primary ring-2 ring-primary/30'
          : 'border-transparent hover:border-border',
      ]"
      @click="emit('update:modelValue', opt.key)"
    >
      <component :is="opt.icon" class="h-4 w-4" :class="opt.tone" />
    </button>
  </div>
</template>
