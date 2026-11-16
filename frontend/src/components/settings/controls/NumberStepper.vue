<script setup lang="ts">
// One stepper for every count: connections in New download, in Settings
// and in the details panel. The value can also be typed; it is clamped
// when the field loses focus.

import { ref, watch } from "vue";
import { Minus, Plus } from "lucide-vue-next";

const props = withDefaults(
  defineProps<{
    modelValue: number;
    min?: number;
    max?: number;
    step?: number;
    disabled?: boolean;
    suffix?: string;
    size?: "md" | "sm";
    /** Id of the visible text that names the group. */
    labelledby?: string;
    decreaseLabel?: string;
    increaseLabel?: string;
    /** Room for longer numbers (ports, milliseconds). */
    wide?: boolean;
  }>(),
  { size: "md" },
);
const emit = defineEmits<{ "update:modelValue": [value: number] }>();

const draft = ref(String(props.modelValue));
watch(
  () => props.modelValue,
  (value) => {
    draft.value = String(value);
  },
);

function clamp(n: number): number {
  let v = n;
  if (props.min != null) v = Math.max(props.min, v);
  if (props.max != null) v = Math.min(props.max, v);
  return v;
}

function bump(delta: number) {
  if (props.disabled) return;
  const step = props.step ?? 1;
  emit("update:modelValue", clamp(props.modelValue + delta * step));
}

function commitDraft() {
  const n = Number(draft.value);
  if (draft.value.trim() === "" || !Number.isFinite(n)) {
    draft.value = String(props.modelValue);
    return;
  }
  const next = clamp(n);
  draft.value = String(next);
  if (next !== props.modelValue) emit("update:modelValue", next);
}
</script>

<template>
  <div class="flex shrink-0 items-center gap-2.5">
    <div
      role="group"
      :aria-labelledby="labelledby"
      class="flex items-center border border-input bg-dl-field focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-ring"
      :class="[
        size === 'sm' ? 'h-8 rounded-lg' : 'h-9 rounded-[9px]',
        disabled ? 'opacity-45' : '',
      ]"
    >
      <button
        type="button"
        :disabled="disabled || (min != null && modelValue <= min)"
        :aria-label="decreaseLabel ?? '−'"
        class="flex items-center justify-center text-foreground transition-colors hover:bg-accent focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring disabled:cursor-not-allowed disabled:opacity-40"
        :class="size === 'sm' ? 'h-[30px] w-8 rounded-[7px]' : 'h-[34px] w-[34px] rounded-lg'"
        @click="bump(-1)"
      >
        <Minus class="h-[13px] w-[13px]" aria-hidden="true" />
      </button>
      <input
        v-model="draft"
        type="text"
        inputmode="numeric"
        :disabled="disabled"
        :aria-labelledby="labelledby"
        class="border-0 bg-transparent p-0 text-center font-mono text-[13px] text-foreground outline-none disabled:cursor-not-allowed"
        :class="wide ? 'w-14' : size === 'sm' ? 'w-7' : 'w-8'"
        @blur="commitDraft"
        @keydown.enter.prevent="commitDraft"
        @keydown.up.prevent="bump(1)"
        @keydown.down.prevent="bump(-1)"
      />
      <button
        type="button"
        :disabled="disabled || (max != null && modelValue >= max)"
        :aria-label="increaseLabel ?? '+'"
        class="flex items-center justify-center text-foreground transition-colors hover:bg-accent focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring disabled:cursor-not-allowed disabled:opacity-40"
        :class="size === 'sm' ? 'h-[30px] w-8 rounded-[7px]' : 'h-[34px] w-[34px] rounded-lg'"
        @click="bump(1)"
      >
        <Plus class="h-[13px] w-[13px]" aria-hidden="true" />
      </button>
    </div>
    <span v-if="suffix" class="text-xs text-muted-foreground">{{ suffix }}</span>
  </div>
</template>
