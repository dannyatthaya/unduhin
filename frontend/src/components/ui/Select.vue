<script setup lang="ts" generic="T extends string | number">
import { Check, ChevronDown } from "lucide-vue-next";
import {
  SelectContent,
  SelectIcon,
  SelectItem,
  SelectItemIndicator,
  SelectItemText,
  SelectPortal,
  SelectRoot,
  SelectTrigger,
  SelectValue,
  SelectViewport,
} from "reka-ui";

export interface SelectOption<V> {
  value: V;
  label: string;
  icon?: string;
  disabled?: boolean;
}

const props = defineProps<{
  modelValue: T;
  options: SelectOption<T>[];
  disabled?: boolean;
  placeholder?: string;
}>();

const emit = defineEmits<{ "update:modelValue": [value: T] }>();

// reka-ui's SelectItem serializes its `value` prop to a string for the
// underlying ARIA combobox. Numeric option values therefore round-trip
// as strings — coerce them back to numbers on the way out so the
// parent's `modelValue` keeps its declared type.
function emitValue(raw: string | number) {
  if (typeof raw === "number") {
    emit("update:modelValue", raw as T);
    return;
  }
  if (props.options.length > 0 && typeof props.options[0].value === "number") {
    const n = Number(raw);
    if (Number.isFinite(n) && String(n) === raw) {
      emit("update:modelValue", n as T);
      return;
    }
  }
  emit("update:modelValue", raw as T);
}
</script>

<template>
  <SelectRoot
    :model-value="String(modelValue)"
    :disabled="disabled"
    @update:model-value="(v) => v != null && emitValue(v as string)"
  >
    <SelectTrigger
      class="inline-flex h-9 w-full items-center justify-between rounded-md border border-input bg-background px-3 text-sm transition-colors hover:bg-accent/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring data-disabled:cursor-not-allowed data-disabled:opacity-50"
    >
      <SelectValue :placeholder="placeholder ?? ''" />
      <SelectIcon as-child>
        <ChevronDown class="h-4 w-4 shrink-0 text-muted-foreground" />
      </SelectIcon>
    </SelectTrigger>

    <SelectPortal>
      <SelectContent
        position="popper"
        :side-offset="4"
        class="z-50 min-w-(--reka-select-trigger-width) overflow-hidden rounded-md border border-border bg-card text-card-foreground shadow-lg data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0"
      >
        <SelectViewport class="max-h-70 overflow-auto p-1">
          <SelectItem
            v-for="opt in options"
            :key="String(opt.value)"
            :value="String(opt.value)"
            :disabled="opt.disabled"
            class="relative flex w-full cursor-pointer select-none items-center gap-2 rounded-sm py-1.5 pl-2 pr-7 text-sm outline-none transition-colors data-highlighted:bg-accent data-highlighted:text-accent-foreground data-disabled:cursor-not-allowed data-disabled:opacity-50"
          >
            <SelectItemText>{{ opt.label }}</SelectItemText>
            <SelectItemIndicator
              class="absolute right-2 inline-flex h-4 w-4 items-center justify-center"
            >
              <Check class="h-3.5 w-3.5" />
            </SelectItemIndicator>
          </SelectItem>
        </SelectViewport>
      </SelectContent>
    </SelectPortal>
  </SelectRoot>
</template>
