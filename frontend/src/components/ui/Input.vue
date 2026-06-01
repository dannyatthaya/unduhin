<script setup lang="ts">
import { computed, useAttrs } from "vue";

import { cn } from "@/lib/utils";

defineOptions({ inheritAttrs: false });

const props = defineProps<{
  modelValue: string | number;
  class?: string;
}>();

const emit = defineEmits<{ "update:modelValue": [value: string] }>();

const attrs = useAttrs();

const classes = computed(() =>
  cn(
    "flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
    props.class,
  ),
);
</script>

<template>
  <input
    v-bind="attrs"
    :value="modelValue"
    :class="classes"
    @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
  />
</template>
