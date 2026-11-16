<script setup lang="ts">
// Schedule download — opened from a queued item's details or its ••• menu.
// The same Start control as New download: now, at a time, or when the
// queue is empty. Quiet hours are no longer here; they are edited inline
// in Settings › Behaviour.
//
// Persistence goes through `useSchedulesStore`, so other windows pick up
// the change via the `schedules_changed` broadcast.

import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { Clock } from "lucide-vue-next";

import Dialog from "@/components/ui/Dialog.vue";
import Button from "@/components/ui/Button.vue";
import StartPicker from "@/components/StartPicker.vue";

import { useStartPlan } from "@/composables/useStartPlan";
import { useDownloadsStore } from "@/stores/downloads";
import { useSchedulesStore } from "@/stores/schedules";
import { toTimeInput } from "@/lib/startPlan";
import type { DownloadId, Schedule } from "@/types/tauri-bindings";

const props = defineProps<{
  open: boolean;
  downloadId: DownloadId;
}>();

const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();
const schedules = useSchedulesStore();
const downloads = useDownloadsStore();
const plan = useStartPlan();

const saving = ref(false);
const errorMessage = ref<string | null>(null);

const record = computed(() => downloads.records.get(props.downloadId) ?? null);
const existing = computed<Schedule[]>(
  () => schedules.byDownload.get(props.downloadId) ?? [],
);

watch(
  () => props.open,
  (open) => {
    if (!open) return;
    errorMessage.value = null;
    plan.reset(existing.value);
  },
  { immediate: true },
);

const statusTag = computed(() => {
  switch (record.value?.status) {
    case "queued":
      return t("downloads.statusQueued");
    case "paused":
      return t("downloads.statusPaused");
    default:
      return "";
  }
});

const isTimed = computed(() => plan.mode.value === "at" && plan.target.value != null);

const confirmLabel = computed(() => {
  if (saving.value) return t("common.saving");
  if (isTimed.value && plan.target.value) {
    return t("addUrl.scheduleFor", { time: toTimeInput(plan.target.value) });
  }
  return t("common.save");
});

async function save() {
  if (saving.value || !plan.isValid.value) return;
  errorMessage.value = null;
  saving.value = true;
  try {
    await plan.apply(props.downloadId, existing.value);
    emit("close");
  } catch (e: unknown) {
    errorMessage.value =
      (e as { message?: string })?.message || t("errors.saveSetting", { error: "" });
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <Dialog
    :open="open"
    :title="t('downloads.scheduleDialogTitleDownload')"
    @close="emit('close')"
  >
    <template v-if="record" #subtitle>
      <span class="min-w-0 truncate font-mono" :title="record.filename">
        {{ record.filename }}
      </span>
      <span
        v-if="statusTag"
        class="flex h-[22px] shrink-0 items-center rounded-md bg-muted px-2 text-xs font-medium text-dl-soft"
      >
        {{ statusTag }}
      </span>
    </template>

    <StartPicker :plan="plan" :hint="t('downloads.scheduleStaysQueued')" />

    <p v-if="errorMessage" role="alert" class="text-xs leading-[1.45] text-danger">
      {{ errorMessage }}
    </p>

    <template #footer>
      <Button @click="emit('close')">{{ t("common.cancel") }}</Button>
      <Button variant="primary" :disabled="saving || !plan.isValid.value" @click="save">
        <Clock v-if="isTimed" class="h-[15px] w-[15px]" aria-hidden="true" />
        {{ confirmLabel }}
      </Button>
    </template>
  </Dialog>
</template>
