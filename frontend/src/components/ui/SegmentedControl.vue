<script setup lang="ts" generic="V extends string">
// One control for every "pick one of a few" choice: Start in New download,
// the theme, what the close button does. A radio group, so arrow keys move
// the choice and Tab moves past it.

import { ref } from "vue";

import { cn } from "@/lib/utils";

export interface SegmentOption<T extends string> {
  value: T;
  label: string;
  /** CSS background for a small round swatch before the label. */
  swatch?: string;
}

const props = withDefaults(
  defineProps<{
    modelValue: V;
    options: SegmentOption<V>[];
    /** Id of the element that labels the group. */
    labelledby?: string;
    ariaLabel?: string;
    disabled?: boolean;
    /** Segments share the full width (dialogs) instead of hugging labels. */
    stretch?: boolean;
    /** Quieter selected state, for a secondary choice such as a unit. */
    muted?: boolean;
    class?: string;
  }>(),
  {},
);

const emit = defineEmits<{ "update:modelValue": [value: V] }>();

const buttons = ref<HTMLButtonElement[]>([]);

function pick(value: V) {
  if (!props.disabled && value !== props.modelValue) {
    emit("update:modelValue", value);
  }
}

function onKeydown(e: KeyboardEvent, index: number) {
  const step =
    e.key === "ArrowRight" || e.key === "ArrowDown"
      ? 1
      : e.key === "ArrowLeft" || e.key === "ArrowUp"
        ? -1
        : 0;
  if (step === 0) return;
  e.preventDefault();
  const count = props.options.length;
  const next = (index + step + count) % count;
  pick(props.options[next].value);
  buttons.value[next]?.focus();
}
</script>

<template>
  <div
    role="radiogroup"
    :aria-labelledby="labelledby"
    :aria-label="ariaLabel"
    :class="
      cn(
        'flex gap-0.5 rounded-[9px] border border-input bg-dl-group p-0.5',
        stretch ? 'w-full' : 'shrink-0',
        disabled && 'opacity-45',
        props.class,
      )
    "
  >
    <button
      v-for="(option, index) in options"
      :key="option.value"
      :ref="(el) => { if (el) buttons[index] = el as HTMLButtonElement; }"
      type="button"
      role="radio"
      :aria-checked="option.value === modelValue"
      :tabindex="option.value === modelValue ? 0 : -1"
      :disabled="disabled"
      class="flex h-[30px] items-center justify-center gap-[7px] rounded-[7px] px-3 text-xs font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:cursor-not-allowed"
      :class="[
        stretch ? 'min-w-0 flex-1' : '',
        option.value === modelValue
          ? muted
            ? 'bg-card text-foreground shadow-[0_0_0_1px_hsl(var(--input))]'
            : 'bg-card text-foreground shadow-[0_0_0_1px_hsl(var(--primary))]'
          : 'text-dl-soft hover:text-foreground',
      ]"
      @click="pick(option.value)"
      @keydown="onKeydown($event, index)"
    >
      <span
        v-if="option.swatch"
        aria-hidden="true"
        class="h-3.5 w-3.5 shrink-0 rounded-full shadow-[inset_0_0_0_1px_hsl(var(--input))]"
        :style="{ background: option.swatch }"
      />
      <span class="truncate">{{ option.label }}</span>
    </button>
  </div>
</template>
