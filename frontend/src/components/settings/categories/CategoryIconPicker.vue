<script setup lang="ts">
// Icon and colour for a category: two radio groups. The picked icon is
// drawn in the picked colour, so the pair previews itself.

import { useI18n } from "vue-i18n";

import { colorClass, ICON_PICKER_OPTIONS } from "@/lib/categoryIcons";
import { CATEGORY_COLORS, type CategoryColor } from "@/types/tauri-bindings";

defineProps<{
  icon: string;
  color: CategoryColor;
  disabled?: boolean;
  iconLabelId: string;
  colorLabelId: string;
}>();

const emit = defineEmits<{
  "update:icon": [value: string];
  "update:color": [value: CategoryColor];
}>();

const { t } = useI18n();

const COLOR_LABEL_KEYS: Record<CategoryColor, string> = {
  violet: "settings.colorViolet",
  orange: "settings.colorOrange",
  green: "settings.colorGreen",
  blue: "settings.colorBlue",
  pink: "settings.colorPink",
  yellow: "settings.colorYellow",
  neutral: "settings.colorGrey",
};

const FOCUS_RING =
  "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
</script>

<template>
  <div class="flex min-w-0 flex-col gap-1.5">
    <span :id="iconLabelId" class="text-xs font-medium text-dl-soft">
      {{ t("settings.categoriesIconLabel") }}
    </span>
    <div role="radiogroup" :aria-labelledby="iconLabelId" class="flex flex-wrap gap-1.5">
      <button
        v-for="option in ICON_PICKER_OPTIONS"
        :key="option.key"
        type="button"
        role="radio"
        :aria-checked="icon === option.key"
        :aria-label="t(option.labelKey)"
        :title="t(option.labelKey)"
        :disabled="disabled"
        class="flex h-9 w-9 items-center justify-center rounded-[9px] transition-colors disabled:cursor-not-allowed disabled:opacity-45"
        :class="[
          FOCUS_RING,
          icon === option.key
            ? [colorClass(color), 'border-2 border-current']
            : 'border border-input text-dl-soft hover:bg-accent',
        ]"
        @click="emit('update:icon', option.key)"
      >
        <component :is="option.icon" class="h-[17px] w-[17px]" aria-hidden="true" />
      </button>
    </div>
  </div>

  <div class="flex min-w-0 flex-col gap-1.5">
    <span :id="colorLabelId" class="text-xs font-medium text-dl-soft">
      {{ t("settings.categoriesColorLabel") }}
    </span>
    <div role="radiogroup" :aria-labelledby="colorLabelId" class="flex gap-2">
      <button
        v-for="name in CATEGORY_COLORS"
        :key="name"
        type="button"
        role="radio"
        :aria-checked="color === name"
        :aria-label="t(COLOR_LABEL_KEYS[name])"
        :title="t(COLOR_LABEL_KEYS[name])"
        :disabled="disabled"
        class="flex h-8 w-8 items-center justify-center rounded-full disabled:cursor-not-allowed disabled:opacity-45"
        :class="[
          FOCUS_RING,
          color === name ? 'border-2 border-foreground' : 'border border-transparent',
        ]"
        @click="emit('update:color', name)"
      >
        <span
          class="h-[22px] w-[22px] rounded-full"
          :style="{ background: `var(--dl-cat-${name}-fg)` }"
        />
      </button>
    </div>
  </div>
</template>
