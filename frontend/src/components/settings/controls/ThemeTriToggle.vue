<script setup lang="ts">
import { Sun, Moon, Monitor } from "lucide-vue-next";

export type ThemeMode = "light" | "dark" | "system";

defineProps<{
  modelValue: ThemeMode;
  disabled?: boolean;
}>();
const emit = defineEmits<{ "update:modelValue": [value: ThemeMode] }>();

const options: { value: ThemeMode; label: string; icon: typeof Sun }[] = [
  { value: "light", label: "Light", icon: Sun },
  { value: "dark", label: "Dark", icon: Moon },
  { value: "system", label: "System", icon: Monitor },
];

function pick(v: ThemeMode) {
  emit("update:modelValue", v);
}
</script>

<template>
  <div class="flex items-center gap-2">
    <button
      v-for="opt in options"
      :key="opt.value"
      type="button"
      :disabled="disabled"
      :aria-pressed="modelValue === opt.value"
      class="flex h-16 w-20 flex-col items-center justify-center gap-1.5 rounded-lg border text-xs font-medium transition-colors"
      :class="
        modelValue === opt.value
          ? 'border-primary bg-primary/5 text-primary ring-2 ring-primary/30'
          : 'border-border bg-card text-foreground hover:bg-accent hover:text-accent-foreground'
      "
      @click="pick(opt.value)"
    >
      <component :is="opt.icon" class="h-5 w-5" />
      <span>{{ opt.label }}</span>
    </button>
  </div>
</template>
