<script setup lang="ts">
// The total speed limit: a switch, then the number and its unit. The
// number stays visible (dimmed) while the limit is off, so switching it
// back on restores the same value.

import { ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import Field from "@/components/ui/Field.vue";
import SegmentedControl from "@/components/ui/SegmentedControl.vue";
import Switch from "@/components/ui/Switch.vue";

const props = defineProps<{
  /** Bytes per second; 0 = no limit. */
  modelValue: number;
  /** Id of the text that names the switch. */
  labelledby?: string;
}>();
const emit = defineEmits<{ "update:modelValue": [value: number] }>();

const { t } = useI18n();

type Unit = "mb" | "kb";
const BYTES: Record<Unit, number> = { mb: 1024 * 1024, kb: 1024 };
const DEFAULT_LIMIT = 10;

const UNIT_OPTIONS: { value: Unit; label: string }[] = [
  { value: "mb", label: "MB/s" },
  { value: "kb", label: "KB/s" },
];

function unitFor(bps: number): Unit {
  return bps > 0 && bps < BYTES.mb ? "kb" : "mb";
}

/** One decimal place is enough for a limit. */
function toDisplay(bps: number, unit: Unit): string {
  return String(Math.round((bps / BYTES[unit]) * 10) / 10);
}

const unit = ref<Unit>(unitFor(props.modelValue));
const amount = ref(props.modelValue > 0 ? toDisplay(props.modelValue, unit.value) : String(DEFAULT_LIMIT));

watch(
  () => props.modelValue,
  (bps) => {
    if (bps <= 0) return;
    // Keep the unit the user chose unless the value no longer fits it.
    if (bps < BYTES.mb && unit.value === "mb") unit.value = "kb";
    amount.value = toDisplay(bps, unit.value);
  },
);

function currentBps(): number {
  const n = Number(amount.value);
  const valid = Number.isFinite(n) && n > 0 ? n : DEFAULT_LIMIT;
  return Math.round(valid * BYTES[unit.value]);
}

function setEnabled(on: boolean): void {
  emit("update:modelValue", on ? currentBps() : 0);
}

function commitAmount(): void {
  const n = Number(amount.value);
  if (!Number.isFinite(n) || n <= 0) {
    amount.value = toDisplay(props.modelValue, unit.value);
    return;
  }
  if (props.modelValue > 0) emit("update:modelValue", currentBps());
}

function setUnit(next: Unit): void {
  unit.value = next;
  if (props.modelValue > 0) emit("update:modelValue", currentBps());
}
</script>

<template>
  <div class="flex items-center gap-2.5">
    <Switch
      :model-value="modelValue > 0"
      :labelledby="labelledby"
      @update:model-value="setEnabled"
    />
    <div class="flex items-center gap-1.5" :class="modelValue > 0 ? '' : 'opacity-45'">
      <Field
        v-model="amount"
        mono
        type="text"
        inputmode="decimal"
        class="w-16"
        :aria-label="t('settings.bandwidthMax')"
        :disabled="modelValue <= 0"
        @blur="commitAmount"
        @keydown.enter.prevent="commitAmount"
      />
      <SegmentedControl
        :model-value="unit"
        :options="UNIT_OPTIONS"
        :aria-label="t('settings.bandwidthUnit')"
        :disabled="modelValue <= 0"
        muted
        class="opacity-100"
        @update:model-value="setUnit"
      />
    </div>
  </div>
</template>
