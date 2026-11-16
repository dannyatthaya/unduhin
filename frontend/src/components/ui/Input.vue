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
    "flex h-9 w-full rounded-[9px] border border-input bg-dl-field px-2.5 text-[13px] text-foreground transition-colors placeholder:text-muted-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:cursor-not-allowed disabled:opacity-45 aria-invalid:border-danger aria-invalid:shadow-[0_0_0_1px_hsl(var(--danger))]",
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
