<script setup lang="ts">
import { ref } from "vue";
import { X } from "lucide-vue-next";

const props = defineProps<{
  modelValue: string[];
  placeholder?: string;
  disabled?: boolean;
  /** Prefix each new chip with this string after trimming (e.g. "."). */
  prefix?: string;
}>();
const emit = defineEmits<{ "update:modelValue": [value: string[]] }>();

const draft = ref("");

function normalize(raw: string): string {
  let s = raw.trim().toLowerCase();
  if (!s) return s;
  if (props.prefix && !s.startsWith(props.prefix)) s = props.prefix + s.replace(/^\.+/, "");
  return s;
}

function commit(raw: string) {
  const v = normalize(raw);
  if (!v) return;
  if (props.modelValue.includes(v)) return;
  emit("update:modelValue", [...props.modelValue, v]);
}

function remove(index: number) {
  const next = props.modelValue.slice();
  next.splice(index, 1);
  emit("update:modelValue", next);
}

function onKey(event: KeyboardEvent) {
  if (event.key === "Enter" || event.key === ",") {
    event.preventDefault();
    commit(draft.value);
    draft.value = "";
  } else if (event.key === "Backspace" && draft.value === "" && props.modelValue.length > 0) {
    remove(props.modelValue.length - 1);
  }
}

function onBlur() {
  if (draft.value) {
    commit(draft.value);
    draft.value = "";
  }
}
</script>

<template>
  <div
    class="flex min-h-9 flex-wrap items-center gap-1.5 rounded-md border border-input bg-background px-2 py-1.5 focus-within:ring-2 focus-within:ring-ring"
    :class="disabled ? 'cursor-not-allowed opacity-50' : ''"
  >
    <span
      v-for="(chip, i) in modelValue"
      :key="chip"
      class="inline-flex items-center gap-1 rounded bg-muted px-2 py-0.5 font-mono text-[11px]"
    >
      {{ chip }}
      <button
        v-if="!disabled"
        type="button"
        class="rounded p-0.5 text-muted-foreground hover:bg-background hover:text-foreground"
        @click="remove(i)"
      >
        <X class="h-3 w-3" />
      </button>
    </span>
    <input
      v-model="draft"
      :placeholder="modelValue.length === 0 ? placeholder ?? 'add…' : 'add…'"
      :disabled="disabled"
      class="min-w-[5rem] flex-1 bg-transparent text-sm placeholder:text-muted-foreground focus:outline-none disabled:cursor-not-allowed"
      @keydown="onKey"
      @blur="onBlur"
    />
  </div>
</template>
