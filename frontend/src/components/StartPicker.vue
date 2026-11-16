<script setup lang="ts">
// The Start control: one segmented choice everywhere (Now / At a time /
// When the queue is empty), with the date and time fields under it when a
// time is wanted. State lives in `useStartPlan`, owned by the dialog.

import { computed, useId } from "vue";
import { useI18n } from "vue-i18n";
import { AlertCircle, Calendar, Clock } from "lucide-vue-next";

import Field from "@/components/ui/Field.vue";
import SegmentedControl, {
  type SegmentOption,
} from "@/components/ui/SegmentedControl.vue";

import type { StartPlan } from "@/composables/useStartPlan";
import { toTimeInput, type StartMode, type TimeUntil } from "@/lib/startPlan";

const props = defineProps<{
  plan: StartPlan;
  /** Help line under the fields while a time is set. */
  hint?: string;
}>();

const { t, locale } = useI18n();
const labelId = useId();
const errorId = useId();

const options = computed<SegmentOption<StartMode>[]>(() => [
  { value: "now", label: t("start.now") },
  { value: "at", label: t("start.atTime") },
  { value: "after_queue", label: t("start.afterQueue") },
]);

function untilLabel(until: TimeUntil): string {
  if (until.days > 0) {
    return t("start.inDaysHours", { days: until.days, hours: until.hours });
  }
  if (until.hours > 0) {
    return t("start.inHoursMinutes", { hours: until.hours, minutes: until.minutes });
  }
  return t("start.inMinutes", { minutes: until.minutes });
}

const relativeLabel = computed(() =>
  props.plan.until.value ? untilLabel(props.plan.until.value) : "",
);

const tomorrowLabel = computed(() => {
  const tomorrow = props.plan.tomorrow.value;
  return tomorrow ? t("start.tomorrowAt", { time: toTimeInput(tomorrow) }) : "";
});

const pastMessage = computed(() => {
  const target = props.plan.target.value;
  if (!target) return t("start.errorIncomplete");
  const nowTime = toTimeInput(props.plan.now.value);
  if (props.plan.isToday.value) {
    return t("start.errorPastToday", { time: toTimeInput(target), now: nowTime });
  }
  const day = target.toLocaleDateString(locale.value, { day: "numeric", month: "short" });
  return t("start.errorPastDate", { date: day });
});

const showError = computed(
  () => props.plan.mode.value === "at" && !props.plan.isValid.value,
);
</script>

<template>
  <div class="flex min-w-0 flex-col gap-1.5">
    <span :id="labelId" class="text-xs font-medium text-dl-soft">
      {{ t("start.label") }}
    </span>
    <SegmentedControl
      v-model="plan.mode.value"
      :options="options"
      :labelledby="labelId"
      stretch
    />

    <template v-if="plan.mode.value === 'at'">
      <div class="flex flex-wrap items-center gap-2">
        <Field
          v-model="plan.date.value"
          type="date"
          class="w-[168px]"
          :aria-label="t('start.dateAria')"
        >
          <template #leading>
            <Calendar class="h-[15px] w-[15px] shrink-0 text-muted-foreground" aria-hidden="true" />
          </template>
        </Field>
        <Field
          v-model="plan.time.value"
          type="time"
          mono
          class="w-[124px]"
          :invalid="showError"
          :aria-label="t('start.timeAria')"
          :aria-describedby="showError ? errorId : undefined"
        >
          <template #leading>
            <Clock
              class="h-[15px] w-[15px] shrink-0"
              :class="showError ? 'text-danger' : 'text-muted-foreground'"
              aria-hidden="true"
            />
          </template>
        </Field>
        <span
          v-if="relativeLabel"
          class="flex h-[26px] shrink-0 items-center rounded-[7px] bg-dl-accent-soft px-[9px] font-mono text-xs text-foreground shadow-[inset_0_0_0_1px_var(--dl-accent-ring)]"
        >
          {{ relativeLabel }}
        </span>
        <button
          v-else-if="plan.isPast.value && tomorrowLabel"
          type="button"
          class="h-[30px] shrink-0 rounded-[7px] border border-dashed border-input px-2.5 text-xs font-medium text-foreground transition-colors hover:bg-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          @click="plan.useTomorrow()"
        >
          {{ tomorrowLabel }}
        </button>
      </div>
      <span
        v-if="showError"
        :id="errorId"
        role="alert"
        class="flex items-start gap-1.5 text-xs leading-[1.45] text-danger"
      >
        <AlertCircle class="mt-0.5 h-[13px] w-[13px] shrink-0" aria-hidden="true" />
        {{ pastMessage }}
      </span>
      <span v-else-if="hint" class="text-xs leading-[1.45] text-muted-foreground">
        {{ hint }}
      </span>
    </template>
    <span
      v-else-if="plan.mode.value === 'after_queue'"
      class="text-xs leading-[1.45] text-muted-foreground"
    >
      {{ t("start.afterQueueHint") }}
    </span>
  </div>
</template>
