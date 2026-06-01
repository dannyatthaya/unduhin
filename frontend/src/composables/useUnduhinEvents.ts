// Subscribes once on app mount to the Tauri event channel and forwards
// every CoreEvent to the stores. The actual reducer logic lives in
// `useDownloadsStore.handleEvent` so it can be unit-tested without
// touching Tauri.

import { onBeforeUnmount, onMounted } from "vue";

import { onCoreEvent } from "@/types/tauri-bindings";
import { useCategoriesStore } from "@/stores/categories";
import { useDownloadsStore } from "@/stores/downloads";
import { useSchedulesStore } from "@/stores/schedules";
import { useSettingsStore } from "@/stores/settings";
import { useSystemStore } from "@/stores/system";
import { useLocale } from "@/composables/useLocale";

export function useUnduhinEvents() {
  const downloads = useDownloadsStore();
  const categories = useCategoriesStore();
  const schedules = useSchedulesStore();
  const settings = useSettingsStore();
  const system = useSystemStore();
  // Touch the locale composable so its watcher on `settings.values.language`
  // is wired before the first `setting_changed` event arrives — without
  // this, cross-window language switches lag a tick behind.
  useLocale();

  let unlisten: (() => void) | undefined;

  onMounted(async () => {
    await Promise.all([
      downloads.refresh(),
      categories.refresh(),
      schedules.refresh(),
      settings.refresh(),
      system.refresh(),
    ]);

    unlisten = await onCoreEvent((event) => {
      downloads.handleEvent(event);
      if (event.type === "setting_changed") {
        void settings.refresh();
        // The default output path setting can change which disk we
        // report on — refresh disk info opportunistically.
        if (event.key === "default_output_path") void system.refresh();
      }
      if (event.type === "schedules_changed") {
        void schedules.refresh();
      }
    });
  });

  onBeforeUnmount(() => {
    unlisten?.();
  });
}
