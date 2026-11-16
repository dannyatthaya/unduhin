<script setup lang="ts">
// Settings → General → Storage: leftover partial files, and the download
// list itself (export it, or forget every finished download).

import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import { Brush } from "lucide-vue-next";

import SettingCard from "./SettingCard.vue";
import SettingRow from "./SettingRow.vue";
import Button from "@/components/ui/Button.vue";
import Dialog from "@/components/ui/Dialog.vue";

import { useDownloadsStore } from "@/stores/downloads";
import { useToast } from "@/composables/useToast";
import { formatBytes } from "@/lib/format";
import { api } from "@/types/tauri-bindings";

const { t } = useI18n();
const downloads = useDownloadsStore();
const toast = useToast();

function messageOf(e: unknown): string {
  return (e as { message?: string })?.message ?? String(e);
}

// ── Leftover partial files ──────────────────────────────────────────────

/** Reclaimable scratch, in bytes. `null` until the first read lands. */
const tempBytes = ref<number | null>(null);
const clearingTemp = ref(false);

async function refreshTempSize(): Promise<void> {
  try {
    tempBytes.value = await api.getTemporaryDataSize();
  } catch {
    // Purely informational; a failed size read should not nag the user.
    tempBytes.value = null;
  }
}

onMounted(refreshTempSize);

async function clearTemporaryData(): Promise<void> {
  clearingTemp.value = true;
  try {
    const result = await api.clearTemporaryData();
    toast.push(
      result.removedDirs === 0
        ? t("settings.clearTempDataEmpty")
        : t("settings.clearTempDataDone", { size: formatBytes(result.freedBytes) }),
      "success",
    );
    await refreshTempSize();
  } catch (e: unknown) {
    toast.push(messageOf(e) || t("settings.clearTempDataFailed"), "error");
  } finally {
    clearingTemp.value = false;
  }
}

// ── Download history ────────────────────────────────────────────────────

const total = computed(() => downloads.all.length);
const finished = computed(
  () => downloads.all.filter((d) => d.status === "completed").length,
);

async function exportHistory(): Promise<void> {
  const path = await saveFileDialog({
    defaultPath: "unduhin-downloads.csv",
    filters: [
      { name: "CSV", extensions: ["csv"] },
      { name: "JSON", extensions: ["json"] },
    ],
  });
  if (!path) return;
  try {
    const rows = await api.exportHistory(path);
    toast.push(t("settings.historyExported", { n: rows }, rows), "success");
  } catch (e: unknown) {
    toast.push(t("settings.historyExportFailed", { error: messageOf(e) }), "error");
  }
}

const confirmOpen = ref(false);
const clearing = ref(false);

async function clearHistory(): Promise<void> {
  clearing.value = true;
  try {
    const removed = await api.clearHistory();
    await downloads.refresh();
    confirmOpen.value = false;
    toast.push(t("settings.historyCleared", { n: removed }, removed), "success");
  } catch (e: unknown) {
    toast.push(t("settings.historyClearFailed", { error: messageOf(e) }), "error");
  } finally {
    clearing.value = false;
  }
}
</script>

<template>
  <SettingCard :title="t('settings.cardStorage')">
    <SettingRow
      id="general/clear-temp-data"
      :label="t('settings.clearTempData')"
      :description="t('settings.clearTempDataDesc')"
    >
      <span v-if="tempBytes !== null" class="whitespace-nowrap font-mono text-xs">
        {{ formatBytes(tempBytes) }}
      </span>
      <Button :disabled="clearingTemp || tempBytes === 0" @click="clearTemporaryData">
        <Brush class="h-[15px] w-[15px]" aria-hidden="true" />
        {{ clearingTemp ? t("settings.clearTempDataBusy") : t("settings.clearTempDataAction") }}
      </Button>
    </SettingRow>
    <SettingRow
      id="general/history"
      :label="t('settings.historyLabel')"
      :description="t('settings.historyDesc', { n: total }, total)"
    >
      <Button :disabled="total === 0" @click="exportHistory">
        {{ t("settings.historyExport") }}
      </Button>
      <Button
        variant="dangerGhost"
        class="px-2.5"
        :disabled="finished === 0"
        @click="confirmOpen = true"
      >
        {{ t("settings.historyClear") }}
      </Button>
    </SettingRow>
  </SettingCard>

  <Dialog
    :open="confirmOpen"
    :title="t('settings.historyClearTitle', { n: finished }, finished)"
    :subtitle="t('settings.historyClearBody')"
    size="sm"
    alert
    @close="confirmOpen = false"
  >
    <template #footer>
      <Button :disabled="clearing" @click="confirmOpen = false">{{ t("common.cancel") }}</Button>
      <Button variant="danger" :disabled="clearing" @click="clearHistory">
        {{ t("settings.historyClearConfirm") }}
      </Button>
    </template>
  </Dialog>
</template>
