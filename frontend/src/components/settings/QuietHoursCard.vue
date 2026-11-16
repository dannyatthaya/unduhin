<script setup lang="ts">
// Settings → Behaviour → Quiet hours, edited in place: on/off, the two
// times (typed, or dragged on the track), the days a window starts on,
// and whether failures still get through. Every change saves at once.

import { computed, onMounted, ref, useId, watch } from "vue";
import { useI18n } from "vue-i18n";
import { Moon, Sun } from "lucide-vue-next";

import QuietHoursTrack from "./QuietHoursTrack.vue";
import SavedTick from "./SavedTick.vue";
import SettingCard from "./SettingCard.vue";
import HighlightText from "./HighlightText.vue";
import Field from "@/components/ui/Field.vue";
import Switch from "@/components/ui/Switch.vue";

import { useSchedulesStore } from "@/stores/schedules";
import { useBehaviourSettings } from "@/composables/useBehaviourSettings";
import { useSavedFlash } from "@/composables/useSavedFlash";
import { useSettingsFilter } from "@/composables/useSettingsFilter";
import { useToast } from "@/composables/useToast";
import {
  ALL_DAYS_MASK,
  DEFAULT_QUIET_END,
  DEFAULT_QUIET_START,
  formatTime,
  hasDay,
  parseTime,
  toggleDay,
  windowLength,
  wrapsMidnight,
} from "@/lib/quietHours";

const ROW_ID = "behaviour/quiet-hours";

const { t } = useI18n();
const schedules = useSchedulesStore();
const behaviour = useBehaviourSettings();
const filter = useSettingsFilter();
const toast = useToast();
const saved = useSavedFlash();

const switchLabelId = useId();
const fromId = useId();
const untilId = useId();
const daysLabelId = useId();
const failuresId = useId();

onMounted(() => {
  // The page may be opened directly; make sure the schedules are loaded.
  if (schedules.list.length === 0) void schedules.refresh();
});

const row = computed(() => schedules.globalQuietHours);
const enabled = computed(() => row.value?.active === true);

const savedStart = computed(
  () => parseTime(row.value?.start_iso) ?? parseTime(DEFAULT_QUIET_START) ?? 0,
);
const savedEnd = computed(
  () => parseTime(row.value?.end_iso) ?? parseTime(DEFAULT_QUIET_END) ?? 0,
);
const daysMask = computed(() => row.value?.days_mask ?? ALL_DAYS_MASK);

/** The window while a handle is being dragged; `null` otherwise. */
const preview = ref<{ start: number; end: number } | null>(null);
const start = computed(() => preview.value?.start ?? savedStart.value);
const end = computed(() => preview.value?.end ?? savedEnd.value);

interface Change {
  start?: number;
  end?: number;
  daysMask?: number;
  active?: boolean;
}

/** Write the window; the row is created the first time anything is set. */
async function persist(change: Change): Promise<void> {
  const input = {
    kind: "quiet_hours" as const,
    download_id: null,
    start_iso: formatTime(change.start ?? savedStart.value),
    end_iso: formatTime(change.end ?? savedEnd.value),
    days_mask: change.daysMask ?? daysMask.value,
    active: change.active ?? enabled.value,
  };
  try {
    if (row.value) await schedules.update(row.value.id, input);
    else await schedules.add(input);
    saved.flash(ROW_ID);
  } catch (e: unknown) {
    const message = (e as { message?: string })?.message ?? String(e);
    toast.push(t("settings.saveFailed", { error: message }), "error");
  } finally {
    preview.value = null;
  }
}

// The time boxes are plain 24-hour text ("22:00"), the same in every
// locale. What is typed is kept as a draft until it is a whole time.
const drafts = ref({ start: "", end: "" });
watch(
  [start, end],
  () => {
    drafts.value = { start: formatTime(start.value), end: formatTime(end.value) };
  },
  { immediate: true },
);

function commitTime(which: "start" | "end"): void {
  const current = which === "start" ? savedStart.value : savedEnd.value;
  const minutes = parseTime(drafts.value[which]);
  if (minutes === null || minutes === current) {
    // Not a time, or no change: show the saved one again.
    drafts.value = { ...drafts.value, [which]: formatTime(current) };
    return;
  }
  void persist({ [which]: minutes });
}

function onPreview(which: "start" | "end", minutes: number): void {
  preview.value = { start: start.value, end: end.value, [which]: minutes };
}

const lengthText = computed(() => {
  const minutes = windowLength(start.value, end.value);
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  const duration =
    rest === 0
      ? t("settings.quietHours", { h: hours })
      : t("settings.quietHoursMinutes", { h: hours, m: rest });
  return wrapsMidnight(start.value, end.value)
    ? t("settings.quietLastsNextDay", { duration })
    : t("settings.quietLasts", { duration });
});

const DAY_KEYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] as const;
const days = computed(() =>
  DAY_KEYS.map((key, index) => ({
    index,
    short: t(`common.day${key}`),
    full: t(`settings.dayFull${key}`),
    on: hasDay(daysMask.value, index),
  })),
);

function onToggleDay(index: number): void {
  const next = toggleDay(daysMask.value, index);
  // A window with no day never starts; keep at least one.
  if (next !== 0) void persist({ daysMask: next });
}
</script>

<template>
  <SettingCard :title="t('settings.cardQuietHours')">
    <div v-show="!filter.isHidden(ROW_ID)" :data-setting-id="ROW_ID" class="flex flex-col">
      <div class="flex min-h-14 items-center justify-between gap-6 border-b border-dl-line-soft px-[18px] py-2.5">
        <div class="flex min-w-0 flex-1 items-center gap-3.5">
          <span
            class="flex h-9 w-9 shrink-0 items-center justify-center rounded-[10px] bg-dl-accent-soft text-primary shadow-[inset_0_0_0_1px_var(--dl-accent-ring)]"
          >
            <Moon class="h-[18px] w-[18px]" aria-hidden="true" />
          </span>
          <div class="flex min-w-0 flex-col gap-0.5">
            <span :id="switchLabelId" class="font-medium">
              <HighlightText :text="t('settings.quietUse')" />
            </span>
            <span class="text-xs leading-[1.45] text-muted-foreground">
              <HighlightText :text="t('settings.quietUseDesc')" />
            </span>
          </div>
        </div>
        <Switch
          :model-value="enabled"
          :labelledby="switchLabelId"
          @update:model-value="persist({ active: $event })"
        />
      </div>

      <div
        class="flex flex-col transition-opacity"
        :class="enabled ? '' : 'opacity-50'"
        :inert="!enabled"
      >
        <div class="flex flex-col gap-2.5 border-b border-dl-line-soft px-[18px] py-3">
          <div class="flex flex-wrap items-end gap-3">
            <div class="flex flex-col gap-1.5">
              <label :for="fromId" class="text-xs font-medium text-dl-soft">
                {{ t("settings.quietFrom") }}
              </label>
              <Field
                :id="fromId"
                v-model="drafts.start"
                type="text"
                inputmode="numeric"
                maxlength="5"
                placeholder="22:00"
                mono
                class="w-32"
                @blur="commitTime('start')"
                @keydown.enter.prevent="commitTime('start')"
              >
                <template #leading>
                  <Moon class="h-[15px] w-[15px] shrink-0 text-muted-foreground" aria-hidden="true" />
                </template>
              </Field>
            </div>
            <div class="flex flex-col gap-1.5">
              <label :for="untilId" class="text-xs font-medium text-dl-soft">
                {{ t("settings.quietUntil") }}
              </label>
              <Field
                :id="untilId"
                v-model="drafts.end"
                type="text"
                inputmode="numeric"
                maxlength="5"
                placeholder="07:00"
                mono
                class="w-32"
                @blur="commitTime('end')"
                @keydown.enter.prevent="commitTime('end')"
              >
                <template #leading>
                  <Sun class="h-[15px] w-[15px] shrink-0 text-muted-foreground" aria-hidden="true" />
                </template>
              </Field>
            </div>
            <span class="flex h-9 items-center text-xs leading-[1.45] text-muted-foreground">
              {{ lengthText }}
            </span>
            <SavedTick class="ml-auto h-9" :show="saved.isSaved(ROW_ID)" />
          </div>
          <div class="flex flex-col gap-2 pt-1.5">
            <QuietHoursTrack
              :start="start"
              :end="end"
              :disabled="!enabled"
              @preview="onPreview"
              @commit="(which, minutes) => persist({ [which]: minutes })"
            />
            <span class="text-xs leading-[1.45] text-muted-foreground">
              {{ t("settings.quietTrackHint") }}
            </span>
          </div>
        </div>

        <div class="flex flex-col gap-2.5 border-b border-dl-line-soft px-[18px] py-3">
          <span :id="daysLabelId" class="text-xs font-medium text-dl-soft">
            {{ t("settings.quietStartsOn") }}
          </span>
          <div role="group" :aria-labelledby="daysLabelId" class="flex gap-1.5">
            <button
              v-for="day in days"
              :key="day.index"
              type="button"
              :aria-pressed="day.on"
              :aria-label="day.full"
              class="h-[34px] min-w-0 flex-1 rounded-lg text-xs font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
              :class="
                day.on
                  ? 'border-[1.5px] border-primary bg-dl-accent-soft text-foreground'
                  : 'border border-input text-dl-soft hover:text-foreground'
              "
              @click="onToggleDay(day.index)"
            >
              {{ day.short }}
            </button>
          </div>
          <span class="text-xs leading-[1.45] text-muted-foreground">
            {{ t("settings.quietDaysHint") }}
          </span>
        </div>

        <div class="flex min-h-14 items-center px-[18px] py-2.5">
          <label :for="failuresId" class="flex cursor-pointer items-start gap-2.5">
            <input
              :id="failuresId"
              v-model="behaviour.quietHoursNotifyFailures.value"
              type="checkbox"
              class="mt-0.5 h-4 w-4 shrink-0 accent-[hsl(var(--primary))]"
            />
            <span class="flex flex-col gap-0.5">
              <span class="font-medium">{{ t("settings.quietFailures") }}</span>
              <span class="text-xs leading-[1.45] text-muted-foreground">
                {{ t("settings.quietFailuresDesc") }}
              </span>
            </span>
          </label>
        </div>
      </div>
    </div>
  </SettingCard>
</template>
