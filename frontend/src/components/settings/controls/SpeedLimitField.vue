<script setup lang="ts">
import { computed, ref, watch } from "vue";

import Switch from "@/components/ui/Switch.vue";

const props = defineProps<{
  /** Bytes per second; 0 = unlimited. */
  modelValue: number;
  disabled?: boolean;
}>();
const emit = defineEmits<{ "update:modelValue": [value: number] }>();

type Unit = "kb" | "mb";

const unit = ref<Unit>(props.modelValue >= 1024 * 1024 ? "mb" : "kb");

function bpsToDisplay(bps: number, u: Unit): number {
  if (bps <= 0) return 0;
  const divisor = u === "mb" ? 1024 * 1024 : 1024;
  // 1 decimal place is enough for the limit picker.
  return Math.round((bps / divisor) * 10) / 10;
}

function displayToBps(n: number, u: Unit): number {
  const factor = u === "mb" ? 1024 * 1024 : 1024;
  return Math.round(n * factor);
}

const value = ref<number>(bpsToDisplay(props.modelValue || 0, unit.value));

watch(
  () => props.modelValue,
  (bps) => {
    value.value = bpsToDisplay(bps || 0, unit.value);
  },
);

const unlimited = computed({
  get: () => props.modelValue <= 0,
  set: (v) => {
    if (v) emit("update:modelValue", 0);
    else emit("update:modelValue", displayToBps(value.value || 1, unit.value));
  },
});

function setUnit(u: Unit) {
  if (u === unit.value) return;
  unit.value = u;
  if (props.modelValue > 0) emit("update:modelValue", displayToBps(value.value, u));
}

function onInputChange(event: Event) {
  const target = event.target as HTMLInputElement;
  const n = Number(target.value);
  if (!Number.isFinite(n) || n < 0) return;
  value.value = n;
  if (!unlimited.value) emit("update:modelValue", displayToBps(n, unit.value));
}
</script>

<template>
  <div class="flex items-center gap-3">
    <span class="text-xs text-muted-foreground">Unlimited</span>
    <Switch v-model="unlimited" :disabled="disabled" aria-label="Unlimited speed" />
    <input
      type="number"
      min="0"
      step="0.1"
      :value="value"
      :disabled="disabled || unlimited"
      class="h-9 w-24 rounded-md border border-input bg-background px-3 text-right text-sm tabular-nums focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
      @input="onInputChange"
    />
    <div class="flex h-9 overflow-hidden rounded-md border border-border">
      <button
        type="button"
        :disabled="disabled"
        class="px-3 text-xs font-medium transition-colors"
        :class="
          unit === 'kb'
            ? 'bg-primary text-primary-foreground'
            : 'bg-background text-foreground hover:bg-accent'
        "
        @click="setUnit('kb')"
      >
        KB/s
      </button>
      <button
        type="button"
        :disabled="disabled"
        class="px-3 text-xs font-medium transition-colors"
        :class="
          unit === 'mb'
            ? 'bg-primary text-primary-foreground'
            : 'bg-background text-foreground hover:bg-accent'
        "
        @click="setUnit('mb')"
      >
        MB/s
      </button>
    </div>
  </div>
</template>
