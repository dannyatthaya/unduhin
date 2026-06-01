<script setup lang="ts">
// Modal driven by the `useDeleteConfirm` composable. Mounted once at the
// app root so any caller can `requestDelete(ids)` and get a Promise back.

import { computed } from "vue";
import { useI18n } from "vue-i18n";

import Dialog from "@/components/ui/Dialog.vue";
import Button from "@/components/ui/Button.vue";

import { useDeleteConfirm } from "@/composables/useDeleteConfirm";
import { useDownloadsStore } from "@/stores/downloads";

const { t } = useI18n();
const { pending, answer } = useDeleteConfirm();
const downloads = useDownloadsStore();

const ids = computed(() => pending.value?.ids ?? []);
const open = computed(() => ids.value.length > 0);

const summary = computed(() => {
  if (ids.value.length === 0) return null;
  if (ids.value.length === 1) {
    const rec = downloads.records.get(ids.value[0]);
    return rec?.filename ?? t("downloads.thisDownload");
  }
  return t("downloads.nDownloads", { n: ids.value.length });
});
</script>

<template>
  <Dialog
    :open="open"
    :title="t('downloads.deleteTitle')"
    size="md"
    @close="answer('cancel')"
  >
    <p class="text-sm text-muted-foreground">
      {{ t("downloads.deleteDescription", { summary }) }}
    </p>
    <ul class="mt-3 space-y-1 text-xs text-muted-foreground">
      <li>
        <span class="font-medium text-foreground">{{ t("downloads.deleteOptionRowOnly") }}</span>
        — {{ t("downloads.deleteOptionRowOnlyHint") }}
      </li>
      <li>
        <span class="font-medium text-foreground">{{ t("downloads.deleteOptionRowAndData") }}</span>
        — {{ t("downloads.deleteOptionRowAndDataHint") }}
      </li>
    </ul>
    <p class="mt-3 text-[11px] text-muted-foreground">
      {{ t("downloads.deleteFooterHint") }}
    </p>

    <template #footer>
      <Button variant="ghost" @click="answer('cancel')">{{ t("common.cancel") }}</Button>
      <Button variant="secondary" @click="answer('row_only')">
        {{ t("downloads.deleteOptionRowOnly") }}
      </Button>
      <Button variant="danger" @click="answer('row_and_data')">
        {{ t("downloads.deleteOptionRowAndData") }}
      </Button>
    </template>
  </Dialog>
</template>
