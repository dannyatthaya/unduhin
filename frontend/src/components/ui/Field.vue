<script setup lang="ts">
// A 36px text field with room for a leading icon and trailing content (a
// tag, a suffix) inside its border. The wrapper carries the border and the
// focus ring; the <input> inside is bare.

import { computed, ref, useAttrs } from "vue";

import { cn } from "@/lib/utils";

defineOptions({ inheritAttrs: false });

const props = withDefaults(
  defineProps<{
    modelValue: string | number;
    /** Red border and ring; pair with `aria-describedby` on the caller. */
    invalid?: boolean;
    /** Monospace 12px text, for links, paths and times. */
    mono?: boolean;
    size?: "md" | "sm";
    disabled?: boolean;
    class?: string;
  }>(),
  { size: "md" },
);

const emit = defineEmits<{ "update:modelValue": [value: string] }>();

const attrs = useAttrs();
const inputRef = ref<HTMLInputElement | null>(null);

const wrapperClass = computed(() =>
  cn(
    "flex min-w-0 items-center gap-2 border bg-dl-field px-2.5 text-foreground transition-colors focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-ring",
    props.size === "sm" ? "h-8 rounded-lg" : "h-9 rounded-[9px]",
    props.invalid
      ? "border-danger shadow-[0_0_0_1px_hsl(var(--danger))]"
      : "border-input",
    props.disabled && "cursor-not-allowed opacity-45",
    props.class,
  ),
);

function focus() {
  inputRef.value?.focus();
}

function select() {
  inputRef.value?.select();
}

defineExpose({ focus, select });
</script>

<template>
  <div :class="wrapperClass">
    <slot name="leading" />
    <input
      ref="inputRef"
      v-bind="attrs"
      :value="modelValue"
      :disabled="disabled"
      :aria-invalid="invalid || undefined"
      class="min-w-0 flex-1 border-0 bg-transparent p-0 text-foreground outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed"
      :class="mono ? 'font-mono text-xs' : 'text-[13px]'"
      @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
    />
    <slot name="trailing" />
  </div>
</template>
