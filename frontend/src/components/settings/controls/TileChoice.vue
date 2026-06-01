<script setup lang="ts" generic="V extends string">
import { Check } from "lucide-vue-next";
import type { Component } from "vue";

export interface TileOption<V extends string> {
  value: V;
  label: string;
  hint?: string;
  icon?: Component;
}

defineProps<{
  modelValue: V;
  options: TileOption<V>[];
  disabled?: boolean;
}>();
const emit = defineEmits<{ "update:modelValue": [value: V] }>();
</script>

<template>
  <div class="grid grid-cols-3 gap-3">
    <button
      v-for="opt in options"
      :key="opt.value"
      type="button"
      :disabled="disabled"
      :aria-pressed="modelValue === opt.value"
      class="relative flex flex-col items-start gap-1 rounded-lg border p-3 text-left transition-colors disabled:cursor-not-allowed disabled:opacity-50"
      :class="
        modelValue === opt.value
          ? 'border-primary bg-primary/5 ring-1 ring-primary/40'
          : 'border-border bg-card hover:bg-accent'
      "
      @click="emit('update:modelValue', opt.value)"
    >
      <span class="absolute right-2 top-2 flex h-4 w-4 items-center justify-center">
        <Check
          v-if="modelValue === opt.value"
          class="h-3.5 w-3.5 text-primary"
        />
        <span
          v-else
          class="h-3.5 w-3.5 rounded-full border border-muted-foreground/40"
        />
      </span>
      <component
        v-if="opt.icon"
        :is="opt.icon"
        class="h-4 w-4 text-muted-foreground"
      />
      <span class="text-sm font-medium">{{ opt.label }}</span>
      <span v-if="opt.hint" class="font-mono text-[10px] uppercase tracking-wide text-muted-foreground">
        {{ opt.hint }}
      </span>
    </button>
  </div>
</template>
