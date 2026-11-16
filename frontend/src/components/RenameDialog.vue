<script setup lang="ts">
// Rename a download's file. The extension stays outside the text box so
// the file still opens; a name that is already taken gets " (n)" appended
// by the backend rather than replacing anything.

import { computed, ref, useId, watch } from "vue";
import { useI18n } from "vue-i18n";

import Dialog from "@/components/ui/Dialog.vue";
import Button from "@/components/ui/Button.vue";
import Field from "@/components/ui/Field.vue";

import { useDownloadsStore } from "@/stores/downloads";
import { splitFileName } from "@/lib/linkInput";
import { api, type DownloadRecord } from "@/types/tauri-bindings";

const props = defineProps<{
  open: boolean;
  download: DownloadRecord;
}>();
const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();
const downloads = useDownloadsStore();
const fieldId = useId();

const stem = ref("");
const saving = ref(false);
const error = ref<string | null>(null);

const parts = computed(() => splitFileName(props.download.filename));

watch(
  () => props.open,
  (open) => {
    if (!open) return;
    stem.value = parts.value.stem;
    error.value = null;
    saving.value = false;
  },
  { immediate: true },
);

const canSave = computed(
  () => !saving.value && stem.value.trim() !== "" && stem.value.trim() !== parts.value.stem,
);

async function save() {
  if (!canSave.value) return;
  saving.value = true;
  error.value = null;
  try {
    const name = `${stem.value.trim()}${parts.value.extension}`;
    downloads.putRecord(await api.renameDownload(props.download.id, name));
    emit("close");
  } catch (e: unknown) {
    const message = (e as { message?: string })?.message ?? String(e);
    error.value = t("errors.renameDownload", { error: message });
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <Dialog :open="open" :title="t('downloads.renameTitle')" @close="emit('close')">
    <div class="flex min-w-0 flex-col gap-1.5">
      <label :for="fieldId" class="text-xs font-medium text-dl-soft">
        {{ t("addUrl.filenameLabel") }}
      </label>
      <Field
        :id="fieldId"
        v-model="stem"
        type="text"
        spellcheck="false"
        :invalid="error != null"
        @keydown.enter.prevent="save"
      >
        <template v-if="parts.extension" #trailing>
          <span class="shrink-0 font-mono text-xs text-muted-foreground">
            {{ parts.extension }}
          </span>
        </template>
      </Field>
      <span v-if="error" role="alert" class="text-xs leading-[1.45] text-danger">{{ error }}</span>
      <span v-else class="text-xs leading-[1.45] text-muted-foreground">
        {{ t("addUrl.extensionStays") }}
      </span>
    </div>

    <template #footer>
      <Button :disabled="saving" @click="emit('close')">{{ t("common.cancel") }}</Button>
      <Button variant="primary" :disabled="!canSave" @click="save">
        {{ saving ? t("common.saving") : t("downloads.renameAction") }}
      </Button>
    </template>
  </Dialog>
</template>
