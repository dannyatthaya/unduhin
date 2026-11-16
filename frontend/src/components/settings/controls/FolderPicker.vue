<script setup lang="ts">
// A folder setting: the path, Browse…, and a button that opens the folder.
// A typed or picked path is checked before it is saved. One that cannot be
// used (a drive that is not connected) is not saved: the field turns red
// and says so, with Try again and Undo.

import { computed, ref, useId, watch } from "vue";
import { useI18n } from "vue-i18n";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import { AlertCircle, ExternalLink, Folder } from "lucide-vue-next";

import Button from "@/components/ui/Button.vue";
import Field from "@/components/ui/Field.vue";
import { useToast } from "@/composables/useToast";
import { api } from "@/types/tauri-bindings";

const props = defineProps<{
  modelValue: string;
  placeholder?: string;
  disabled?: boolean;
  /** Id for the row's <label for>. */
  id?: string;
  /** Fill the row instead of the default fixed width. */
  full?: boolean;
  /** An empty path is a valid choice ("use the default"). */
  allowEmpty?: boolean;
}>();
const emit = defineEmits<{ "update:modelValue": [value: string] }>();

const { t } = useI18n();
const toast = useToast();
const errorId = useId();

const draft = ref(props.modelValue);
const rejected = ref(false);
const checking = ref(false);

watch(
  () => props.modelValue,
  (value) => {
    draft.value = value;
    rejected.value = false;
  },
);

const isDirty = computed(() => draft.value.trim() !== props.modelValue);

async function commit(): Promise<void> {
  const path = draft.value.trim();
  if (path === props.modelValue && !rejected.value) return;
  if (path === "") {
    if (props.allowEmpty) emit("update:modelValue", "");
    else undo();
    return;
  }
  checking.value = true;
  try {
    rejected.value = !(await api.checkFolder(path));
  } catch {
    rejected.value = true;
  } finally {
    checking.value = false;
  }
  if (!rejected.value) emit("update:modelValue", path);
}

function undo(): void {
  draft.value = props.modelValue;
  rejected.value = false;
}

async function browse(): Promise<void> {
  if (props.disabled) return;
  const selected = await open({
    directory: true,
    multiple: false,
    defaultPath: props.modelValue || undefined,
  });
  if (typeof selected !== "string") return;
  draft.value = selected;
  await commit();
}

async function openFolder(): Promise<void> {
  try {
    await openPath(props.modelValue);
  } catch {
    toast.push(t("settings.folderOpenFailed"), "error");
  }
}
</script>

<template>
  <div class="flex max-w-full flex-col gap-2.5" :class="full ? 'w-full' : 'w-[26rem]'">
    <div class="flex gap-2">
      <Field
        :id="id"
        v-model="draft"
        mono
        type="text"
        spellcheck="false"
        class="flex-1"
        :placeholder="placeholder ?? t('settings.folderPlaceholder')"
        :disabled="disabled"
        :invalid="rejected"
        :aria-describedby="rejected ? errorId : undefined"
        @blur="isDirty && commit()"
        @keydown.enter.prevent="commit"
        @keydown.esc.stop="undo"
      >
        <template #leading>
          <Folder
            class="h-[15px] w-[15px] shrink-0"
            :class="rejected ? 'text-danger' : 'text-muted-foreground'"
            aria-hidden="true"
          />
        </template>
      </Field>
      <Button :disabled="disabled || checking" @click="browse">{{ t("common.browse") }}</Button>
      <Button
        variant="ghost"
        size="icon"
        class="text-muted-foreground"
        :disabled="modelValue === ''"
        :aria-label="t('settings.folderOpen')"
        :title="t('settings.folderOpen')"
        @click="openFolder"
      >
        <ExternalLink class="h-4 w-4" aria-hidden="true" />
      </Button>
    </div>
    <div
      v-if="rejected"
      :id="errorId"
      role="alert"
      class="flex items-start gap-2 rounded-lg bg-dl-fail-soft px-2.5 py-2 text-xs leading-[1.45]"
    >
      <AlertCircle class="mt-px h-3.5 w-3.5 shrink-0 text-danger" aria-hidden="true" />
      <span class="min-w-0 flex-1 break-words">
        {{
          modelValue
            ? t("settings.folderNotSaved", { path: modelValue })
            : t("settings.folderNotSavedNoFallback")
        }}
      </span>
      <button type="button" class="settings-inline-action" :disabled="checking" @click="commit">
        {{ t("settings.tryAgain") }}
      </button>
      <button type="button" class="settings-inline-action" @click="undo">
        {{ t("settings.undo") }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.settings-inline-action {
  flex: none;
  height: 24px;
  margin: -3px 0;
  padding: 0 8px;
  border-radius: 6px;
  font-weight: 600;
  text-decoration: underline;
  text-underline-offset: 3px;
}

.settings-inline-action:hover {
  background: hsl(var(--accent));
}

.settings-inline-action:focus-visible {
  outline: 2px solid hsl(var(--ring));
  outline-offset: 2px;
}
</style>
