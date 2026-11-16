<script setup lang="ts" generic="T extends string | number">
import { computed } from "vue";
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
  SelectViewport,
} from "reka-ui";

export interface SelectOption<V> {
  value: V;
  label: string;
  /** Text for the closed control when it should differ from the list item. */
  triggerLabel?: string;
  icon?: string;
  disabled?: boolean;
}

const props = defineProps<{
  modelValue: T;
  options: SelectOption<T>[];
  disabled?: boolean;
  placeholder?: string;
  /** Id for a <label for>. */
  id?: string;
  ariaLabel?: string;
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

// Reactively look up the selected option's label so the trigger updates when
// option labels change (e.g. switching app language). reka-ui's <SelectValue>
// captures the selected item's text once at selection time and would otherwise
// show a stale (English) label after a locale switch.
const selectedLabel = computed(() => {
  const match = props.options.find(
    (o) => String(o.value) === String(props.modelValue),
  );
  return match ? (match.triggerLabel ?? match.label) : (props.placeholder ?? "");
});
</script>

<template>
  <SelectRoot
    :model-value="String(modelValue)"
    :disabled="disabled"
    @update:model-value="(v) => v != null && emitValue(v as string)"
  >
    <SelectTrigger
      :id="id"
      :aria-label="ariaLabel"
      class="inline-flex h-9 w-full items-center justify-between gap-2 rounded-[9px] border border-input bg-dl-field px-2.5 text-[13px] text-foreground transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring data-disabled:cursor-not-allowed data-disabled:opacity-45"
    >
      <span class="min-w-0 flex-1 truncate text-left">{{ selectedLabel }}</span>
      <slot name="tag" />
      <SelectIcon as-child>
        <ChevronDown class="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      </SelectIcon>
    </SelectTrigger>

    <SelectPortal>
      <SelectContent
        position="popper"
        :side-offset="4"
        class="dl-surface z-[60] min-w-(--reka-select-trigger-width) overflow-hidden rounded-[10px] border border-border bg-card text-card-foreground shadow-dl-menu data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0"
      >
        <SelectViewport class="max-h-70 overflow-auto p-1">
          <SelectItem
            v-for="opt in options"
            :key="String(opt.value)"
            :value="String(opt.value)"
            :disabled="opt.disabled"
            class="relative flex h-8 w-full cursor-pointer select-none items-center gap-2 rounded-[7px] pl-2.5 pr-7 text-[13px] outline-none transition-colors data-highlighted:bg-accent data-highlighted:text-accent-foreground data-disabled:cursor-not-allowed data-disabled:opacity-45"
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
