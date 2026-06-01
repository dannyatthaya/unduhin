import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { api } from "@/types/tauri-bindings";
import type {
  DownloadId,
  NewSchedule,
  QuietHoursState,
  Schedule,
  ScheduleId,
} from "@/types/tauri-bindings";

/**
 * Schedules store. Mirrors the persisted `schedules` table plus a
 * cached `QuietHoursState` snapshot. The `useNotifications` composable
 * reads `quietHours.active` synchronously as the suppression gate; the
 * `ScheduleDialog` component drives the CRUD wrappers.
 *
 * Both `list` and `quietHours` are kept fresh by App.vue's
 * `schedules_changed` event subscription — direct mutators here only
 * forward to the API; they never optimistically mutate the cache, so a
 * dropped event still results in the right state on next refresh.
 */
export const useSchedulesStore = defineStore("schedules", () => {
  const list = ref<Schedule[]>([]);
  const quietHours = ref<QuietHoursState>({ active: false, until: null });

  /** Per-download index for ScheduleDialog scope: `kind = "download"`. */
  const byDownload = computed(() => {
    const m = new Map<DownloadId, Schedule[]>();
    for (const s of list.value) {
      if (s.download_id == null) continue;
      const arr = m.get(s.download_id) ?? [];
      arr.push(s);
      m.set(s.download_id, arr);
    }
    return m;
  });

  /** Single global `quiet_hours` row when one exists. */
  const globalQuietHours = computed(() =>
    list.value.find((s) => s.kind === "quiet_hours" && s.download_id == null) ?? null,
  );

  async function refresh() {
    const [rows, state] = await Promise.all([
      api.listSchedules(),
      api.getQuietHoursState(),
    ]);
    list.value = rows;
    quietHours.value = state;
  }

  async function add(input: NewSchedule): Promise<ScheduleId> {
    const id = await api.addSchedule(input);
    await refresh();
    return id;
  }

  async function update(id: ScheduleId, input: NewSchedule): Promise<void> {
    await api.updateSchedule(id, input);
    await refresh();
  }

  async function remove(id: ScheduleId): Promise<void> {
    await api.removeSchedule(id);
    await refresh();
  }

  return {
    list,
    quietHours,
    byDownload,
    globalQuietHours,
    refresh,
    add,
    update,
    remove,
  };
});
