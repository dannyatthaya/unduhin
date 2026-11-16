<script setup lang="ts">
// A field of removable chips with a text box at the end. Enter or a comma
// turns what was typed into a chip; Backspace on an empty box removes the
// last one. Used for a category's file extensions.

import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { AlertTriangle, X } from "lucide-vue-next";

const props = defineProps<{
  modelValue: string[];
  placeholder?: string;
  disabled?: boolean;
  /** Shown before each chip (e.g. "."); stored values do not include it. */
  prefix?: string;
  /** Chips to draw as "needs a decision": warning tint and icon. */
  flagged?: readonly string[];
  /** Accessible name of the text box. */
  inputLabel?: string;
  /** Id of the hint that says how to add a chip. */
  describedby?: string;
}>();
const emit = defineEmits<{ "update:modelValue": [value: string[]] }>();

const { t } = useI18n();
const draft = ref("");

/** Stored form: lower-case, without the prefix or surrounding space. */
function normalize(raw: string): string {
  const trimmed = raw.trim().toLowerCase();
  if (!props.prefix) return trimmed;
  let value = trimmed;
  while (value.startsWith(props.prefix)) value = value.slice(props.prefix.length);
  return value;
}

/** Several values may arrive at once (a paste of "mp4, mkv webm"). */
function commit(raw: string) {
  const additions = raw
    .split(/[\s,]+/)
    .map(normalize)
    .filter((value) => value !== "");
  const next = [...props.modelValue];
  for (const value of additions) {
    if (!next.includes(value)) next.push(value);
  }
  if (next.length !== props.modelValue.length) emit("update:modelValue", next);
}

function remove(value: string) {
  emit(
    "update:modelValue",
    props.modelValue.filter((chip) => chip !== value),
  );
}

function commitDraft() {
  if (draft.value.trim() === "") return;
  commit(draft.value);
  draft.value = "";
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === "Enter" || event.key === ",") {
    // Always swallow Enter here: in a dialog it must add the chip, never
    // submit the form around it.
    event.preventDefault();
    commitDraft();
  } else if (event.key === "Backspace" && draft.value === "" && props.modelValue.length > 0) {
    remove(props.modelValue[props.modelValue.length - 1]);
  }
}
</script>

<template>
  <div
    class="flex min-h-10 flex-wrap items-center gap-1.5 rounded-[9px] border border-input bg-dl-field px-1.5 py-[5px] focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-ring"
    :class="disabled ? 'cursor-not-allowed opacity-45' : ''"
  >
    <span
      v-for="chip in modelValue"
      :key="chip"
      class="flex h-7 items-center gap-0.5 rounded-[7px] pr-0.5 font-mono text-xs text-foreground"
      :class="
        flagged?.includes(chip)
          ? 'bg-dl-warn-soft pl-[7px] shadow-[inset_0_0_0_1px_hsl(var(--warning))]'
          : 'bg-muted pl-[9px]'
      "
    >
      <AlertTriangle
        v-if="flagged?.includes(chip)"
        class="mr-1 h-3 w-3 text-warning"
        aria-hidden="true"
      />
      {{ prefix }}{{ chip }}
      <button
        v-if="!disabled"
        type="button"
        class="flex h-6 w-6 items-center justify-center rounded-[5px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
        :aria-label="t('settings.chipRemove', { value: `${prefix ?? ''}${chip}` })"
        @click="remove(chip)"
      >
        <X class="h-[11px] w-[11px]" aria-hidden="true" />
      </button>
    </span>
    <input
      v-model="draft"
      type="text"
      spellcheck="false"
      autocomplete="off"
      :placeholder="placeholder"
      :disabled="disabled"
      :aria-label="inputLabel"
      :aria-describedby="describedby"
      class="h-7 min-w-[90px] flex-1 border-0 bg-transparent px-1 font-mono text-xs text-foreground outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed"
      @keydown="onKeydown"
      @blur="commitDraft"
    />
  </div>
</template>
