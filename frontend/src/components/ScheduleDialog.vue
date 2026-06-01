<script setup lang="ts">
// ScheduleDialog — UI for the three schedule kinds (start_at, after_queue,
// quiet_hours). Two modes:
//
//   - `scope = { kind: "download", downloadId }` — shows start_at +
//     after_queue forms. Reads existing rows from the schedules store,
//     applies a CRUD diff on save.
//   - `scope = { kind: "global" }` — shows the quiet_hours form. Operates
//     on the singleton global row (creates it on first save).
//
// All persistence goes through `useSchedulesStore` so other windows pick
// up the change via the `schedules_changed` broadcast.

import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import Dialog from "@/components/ui/Dialog.vue";
import Button from "@/components/ui/Button.vue";
import Input from "@/components/ui/Input.vue";
import Switch from "@/components/ui/Switch.vue";

import { useSchedulesStore } from "@/stores/schedules";
import type { DownloadId, NewSchedule, Schedule } from "@/types/tauri-bindings";

const { t } = useI18n();

type DialogScope =
  | { kind: "download"; downloadId: DownloadId }
  | { kind: "global" };

const props = defineProps<{
  open: boolean;
  scope: DialogScope;
}>();

const emit = defineEmits<{ close: [] }>();

const schedules = useSchedulesStore();

// Per-download state.
const startAtEnabled = ref(false);
const startAtIso = ref("");
const afterQueueEnabled = ref(false);

// Global / quiet-hours state.
const quietEnabled = ref(false);
const quietStart = ref("22:00");
const quietEnd = ref("07:00");
// Mon..Sun mask. Bit 0 = Mon.
const days = ref<boolean[]>([true, true, true, true, true, true, true]);

const saving = ref(false);
const errorMessage = ref<string | null>(null);

const isDownloadScope = computed(() => props.scope.kind === "download");

const existingForDownload = computed<Schedule[]>(() => {
  if (props.scope.kind !== "download") return [];
  return schedules.byDownload.get(props.scope.downloadId) ?? [];
});

const existingGlobalQuiet = computed<Schedule | null>(
  () => schedules.globalQuietHours,
);

watch(
  () => props.open,
  (v) => {
    if (!v) return;
    errorMessage.value = null;
    if (props.scope.kind === "download") {
      const start = existingForDownload.value.find((s) => s.kind === "start_at");
      const afterQ = existingForDownload.value.find(
        (s) => s.kind === "after_queue",
      );
      startAtEnabled.value = !!start;
      startAtIso.value = start?.start_iso
        ? rfcToDatetimeLocal(start.start_iso)
        : defaultStartAt();
      afterQueueEnabled.value = !!afterQ;
    } else {
      const q = existingGlobalQuiet.value;
      quietEnabled.value = !!q;
      quietStart.value = (q?.start_iso as string | null) ?? "22:00";
      quietEnd.value = (q?.end_iso as string | null) ?? "07:00";
      const mask = q?.days_mask ?? 127;
      for (let i = 0; i < 7; i++) days.value[i] = (mask & (1 << i)) !== 0;
    }
  },
  { immediate: true },
);

function defaultStartAt(): string {
  const d = new Date(Date.now() + 60_000); // +1 minute
  return toDatetimeLocal(d);
}

/** Convert an RFC3339 instant into the `<input type=datetime-local>` shape. */
function rfcToDatetimeLocal(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return defaultStartAt();
  return toDatetimeLocal(d);
}

function toDatetimeLocal(d: Date): string {
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** `datetime-local` is unzoned local time; serialize as RFC3339 UTC. */
function datetimeLocalToRfc(value: string): string {
  // The Date constructor parses "YYYY-MM-DDTHH:MM" as local time, which
  // is what we want.
  const d = new Date(value);
  return d.toISOString();
}

function maskFromDays(): number {
  let m = 0;
  for (let i = 0; i < 7; i++) if (days.value[i]) m |= 1 << i;
  return m;
}

async function save() {
  errorMessage.value = null;
  saving.value = true;
  try {
    if (props.scope.kind === "download") {
      const id = props.scope.downloadId;
      const existing = existingForDownload.value;

      // start_at row diff
      const startRow = existing.find((s) => s.kind === "start_at");
      if (startAtEnabled.value) {
        const iso = datetimeLocalToRfc(startAtIso.value);
        const next: NewSchedule = {
          kind: "start_at",
          download_id: id,
          start_iso: iso,
        };
        if (startRow) await schedules.update(startRow.id, next);
        else await schedules.add(next);
      } else if (startRow) {
        await schedules.remove(startRow.id);
      }

      // after_queue row diff
      const aqRow = existing.find((s) => s.kind === "after_queue");
      if (afterQueueEnabled.value && !aqRow) {
        await schedules.add({ kind: "after_queue", download_id: id });
      } else if (!afterQueueEnabled.value && aqRow) {
        await schedules.remove(aqRow.id);
      }
    } else {
      const existing = existingGlobalQuiet.value;
      if (quietEnabled.value) {
        if (maskFromDays() === 0) {
          errorMessage.value = t("downloads.scheduleErrorEmptyDays");
          return;
        }
        const next: NewSchedule = {
          kind: "quiet_hours",
          start_iso: quietStart.value,
          end_iso: quietEnd.value,
          days_mask: maskFromDays(),
        };
        if (existing) await schedules.update(existing.id, next);
        else await schedules.add(next);
      } else if (existing) {
        await schedules.remove(existing.id);
      }
    }
    emit("close");
  } catch (e: unknown) {
    errorMessage.value =
      (e as { message?: string })?.message ??
      t("errors.saveSetting", { error: "" });
  } finally {
    saving.value = false;
  }
}

const dayLabels = computed(() => [
  t("common.dayMon"),
  t("common.dayTue"),
  t("common.dayWed"),
  t("common.dayThu"),
  t("common.dayFri"),
  t("common.daySat"),
  t("common.daySun"),
]);

const title = computed(() =>
  isDownloadScope.value
    ? t("downloads.scheduleDialogTitleDownload")
    : t("downloads.scheduleDialogTitleGlobal"),
);
</script>

<template>
  <Dialog :open="open" :title="title" size="md" @close="emit('close')">
    <div v-if="isDownloadScope" class="space-y-5">
      <section class="space-y-2">
        <header class="flex items-center justify-between gap-3">
          <div>
            <h3 class="text-sm font-medium">{{ t("downloads.scheduleStartAt") }}</h3>
            <p class="text-xs text-muted-foreground">
              {{ t("addUrl.startAtHint") }}
            </p>
          </div>
          <Switch v-model="startAtEnabled" :aria-label="t('downloads.scheduleStartAt')" />
        </header>
        <Input
          v-if="startAtEnabled"
          v-model="startAtIso"
          type="datetime-local"
        />
      </section>

      <section class="space-y-2">
        <header class="flex items-center justify-between gap-3">
          <div>
            <h3 class="text-sm font-medium">{{ t("downloads.scheduleAfterQueue") }}</h3>
            <p class="text-xs text-muted-foreground">
              {{ t("downloads.scheduleAfterQueueHint") }}
            </p>
          </div>
          <Switch
            v-model="afterQueueEnabled"
            :aria-label="t('downloads.scheduleAfterQueue')"
          />
        </header>
      </section>
    </div>

    <div v-else class="space-y-4">
      <header class="flex items-center justify-between gap-3">
        <div>
          <h3 class="text-sm font-medium">{{ t("downloads.scheduleDialogTitleGlobal") }}</h3>
          <p class="text-xs text-muted-foreground">
            {{ t("settings.cardQuietHoursDesc") }}
          </p>
        </div>
        <Switch v-model="quietEnabled" :aria-label="t('downloads.scheduleEnableQuiet')" />
      </header>

      <div v-if="quietEnabled" class="space-y-3">
        <div class="grid grid-cols-2 gap-3">
          <div>
            <label class="mb-1 block text-xs font-medium text-muted-foreground">
              {{ t("downloads.scheduleStartTime") }}
            </label>
            <Input v-model="quietStart" type="time" />
          </div>
          <div>
            <label class="mb-1 block text-xs font-medium text-muted-foreground">
              {{ t("downloads.scheduleEndTime") }}
            </label>
            <Input v-model="quietEnd" type="time" />
          </div>
        </div>
        <div>
          <label class="mb-1 block text-xs font-medium text-muted-foreground">
            {{ t("downloads.scheduleDaysLabel") }}
          </label>
          <div class="flex flex-wrap gap-1.5">
            <button
              v-for="(label, i) in dayLabels"
              :key="label"
              type="button"
              class="rounded-md border border-border px-2.5 py-1 text-xs font-medium transition-colors"
              :class="
                days[i]
                  ? 'bg-primary text-primary-foreground border-primary'
                  : 'bg-card text-muted-foreground hover:bg-accent'
              "
              @click="days[i] = !days[i]"
            >
              {{ label }}
            </button>
          </div>
        </div>
        <p class="text-xs text-muted-foreground">
          {{ t("downloads.scheduleMidnightWrapHint") }}
        </p>
      </div>
    </div>

    <p v-if="errorMessage" class="mt-3 text-xs text-danger">{{ errorMessage }}</p>

    <template #footer>
      <Button variant="ghost" @click="emit('close')">{{ t("common.cancel") }}</Button>
      <Button variant="primary" :disabled="saving" @click="save">
        {{ saving ? t("common.saving") : t("common.save") }}
      </Button>
    </template>
  </Dialog>
</template>
