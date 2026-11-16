// State behind the Start control: now, at a set time, or when the queue is
// empty. Shared by New download and Schedule download so both ask the same
// question the same way and write the same schedule rows.

import { computed, onBeforeUnmount, ref, type ComputedRef, type Ref } from "vue";

import {
  combineLocal,
  defaultStartTime,
  isSameDay,
  nextDay,
  timeUntil,
  toDateInput,
  toTimeInput,
  type StartMode,
  type TimeUntil,
} from "@/lib/startPlan";
import { useSchedulesStore } from "@/stores/schedules";
import type { DownloadId, Schedule } from "@/types/tauri-bindings";

/** How often "in 4 h 55 min" and the "already passed" check re-evaluate. */
const CLOCK_TICK_MS = 15_000;

export interface StartPlan {
  mode: Ref<StartMode>;
  date: Ref<string>;
  time: Ref<string>;
  /** The chosen instant in "at" mode, or null when the fields are unusable. */
  target: ComputedRef<Date | null>;
  /** "At a time" is picked and the time is not in the future. */
  isPast: ComputedRef<boolean>;
  /** Wait until the chosen time; null unless it is ahead. */
  until: ComputedRef<TimeUntil | null>;
  /** The same clock time tomorrow, offered when the time has passed. */
  tomorrow: ComputedRef<Date | null>;
  /** Whether the chosen date is today. */
  isToday: ComputedRef<boolean>;
  now: Ref<Date>;
  /** False while "at a time" has no usable future time. */
  isValid: ComputedRef<boolean>;
  reset: (existing?: Schedule[]) => void;
  useTomorrow: () => void;
  /** Make `id`'s schedule rows match the plan. */
  apply: (id: DownloadId, existing?: Schedule[]) => Promise<void>;
}

export function useStartPlan(): StartPlan {
  const schedules = useSchedulesStore();

  const mode = ref<StartMode>("now");
  const date = ref("");
  const time = ref("");
  const now = ref(new Date());

  const timer = setInterval(() => {
    now.value = new Date();
  }, CLOCK_TICK_MS);
  onBeforeUnmount(() => clearInterval(timer));

  const target = computed(() =>
    mode.value === "at" ? combineLocal(date.value, time.value) : null,
  );
  const until = computed(() =>
    target.value ? timeUntil(target.value, now.value) : null,
  );
  const isPast = computed(
    () => mode.value === "at" && target.value != null && until.value == null,
  );
  const tomorrow = computed(() => (target.value ? nextDay(target.value) : null));
  const isToday = computed(
    () => target.value != null && isSameDay(target.value, now.value),
  );
  const isValid = computed(() => mode.value !== "at" || until.value != null);

  function setFields(instant: Date) {
    date.value = toDateInput(instant);
    time.value = toTimeInput(instant);
  }

  /** Start from the download's existing schedule rows, or from "now". */
  function reset(existing: Schedule[] = []) {
    now.value = new Date();
    const startAt = existing.find((s) => s.kind === "start_at" && s.start_iso);
    const afterQueue = existing.some((s) => s.kind === "after_queue");
    const parsed = startAt?.start_iso ? new Date(startAt.start_iso) : null;
    if (parsed && !Number.isNaN(parsed.getTime())) {
      mode.value = "at";
      setFields(parsed);
      return;
    }
    mode.value = afterQueue ? "after_queue" : "now";
    setFields(defaultStartTime(now.value));
  }

  function useTomorrow() {
    if (tomorrow.value) setFields(tomorrow.value);
  }

  async function apply(id: DownloadId, existing: Schedule[] = []) {
    const startRow = existing.find((s) => s.kind === "start_at");
    const queueRow = existing.find((s) => s.kind === "after_queue");

    if (mode.value === "at" && target.value) {
      const input = {
        kind: "start_at" as const,
        download_id: id,
        start_iso: target.value.toISOString(),
      };
      if (startRow) await schedules.update(startRow.id, input);
      else await schedules.add(input);
    } else if (startRow) {
      await schedules.remove(startRow.id);
    }

    if (mode.value === "after_queue") {
      if (!queueRow) await schedules.add({ kind: "after_queue", download_id: id });
    } else if (queueRow) {
      await schedules.remove(queueRow.id);
    }
  }

  reset();

  return {
    mode,
    date,
    time,
    target,
    isPast,
    until,
    tomorrow,
    isToday,
    now,
    isValid,
    reset,
    useTomorrow,
    apply,
  };
}
