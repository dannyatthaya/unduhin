// Holds host-level info (app version, disk free/total) that the UI
// shows in the sidebar footer and the welcome screen. Refreshes on
// demand; we don't need a watcher since disk usage doesn't change
// fast enough to require live updates.

import { defineStore } from "pinia";
import { ref } from "vue";

import { api } from "@/types/tauri-bindings";
import type { AppInfo, DiskInfo } from "@/types/tauri-bindings";

export const useSystemStore = defineStore("system", () => {
  const appInfo = ref<AppInfo | null>(null);
  const disk = ref<DiskInfo | null>(null);

  async function refresh() {
    try {
      const [info, d] = await Promise.all([api.appInfo(), api.getDiskInfo()]);
      appInfo.value = info;
      disk.value = d;
    } catch {
      // The frontend can still be useful without these; swallow.
    }
  }

  return { appInfo, disk, refresh };
});
