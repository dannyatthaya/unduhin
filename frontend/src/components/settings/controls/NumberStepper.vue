<script setup lang="ts">
import { Minus, Plus } from "lucide-vue-next";

const props = defineProps<{
  modelValue: number;
  min?: number;
  max?: number;
  step?: number;
  disabled?: boolean;
  suffix?: string;
}>();
const emit = defineEmits<{ "update:modelValue": [value: number] }>();

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

function onInput(event: Event) {
  const target = event.target as HTMLInputElement;
  const n = Number(target.value);
  if (Number.isFinite(n)) emit("update:modelValue", clamp(n));
}
</script>

<template>
  <div class="flex items-center gap-1">
    <button
      type="button"
      :disabled="disabled || (min != null && modelValue <= min)"
      class="flex h-9 w-9 items-center justify-center rounded-md border border-input bg-background text-foreground transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
      @click="bump(-1)"
    >
      <Minus class="h-3.5 w-3.5" />
    </button>
    <input
      type="number"
      :value="modelValue"
      :min="min"
      :max="max"
      :step="step ?? 1"
      :disabled="disabled"
      class="no-spinner h-9 w-20 rounded-md border border-input bg-background px-3 text-center text-sm tabular-nums focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      @input="onInput"
    />
    <button
      type="button"
      :disabled="disabled || (max != null && modelValue >= max)"
      class="flex h-9 w-9 items-center justify-center rounded-md border border-input bg-background text-foreground transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
      @click="bump(1)"
    >
      <Plus class="h-3.5 w-3.5" />
    </button>
    <span v-if="suffix" class="ml-2 text-xs text-muted-foreground">{{ suffix }}</span>
  </div>
</template>

<style scoped>
.no-spinner::-webkit-outer-spin-button,
.no-spinner::-webkit-inner-spin-button {
  -webkit-appearance: none;
  margin: 0;
}
.no-spinner {
  -moz-appearance: textfield;
  appearance: textfield;
}
</style>
