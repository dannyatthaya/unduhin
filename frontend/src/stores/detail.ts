// useDetailStore — tracks which single download (if any) is open in
// the right-side detail pane, and which tab is active. Decoupled from
// the multi-select store on purpose: a user can have many rows
// multi-selected while the detail pane still pins a single download
// they're inspecting.

import { defineStore } from "pinia";
import { ref } from "vue";

import type { DownloadId } from "@/types/tauri-bindings";

export type DetailTab = "overview" | "segments" | "history";

export const useDetailStore = defineStore("detail", () => {
  const openId = ref<DownloadId | null>(null);
  const tab = ref<DetailTab>("overview");

  function open(id: DownloadId, t: DetailTab = "overview") {
    openId.value = id;
    tab.value = t;
  }

  function close() {
    openId.value = null;
  }

  function setTab(t: DetailTab) {
    tab.value = t;
  }

  return { openId, tab, open, close, setTab };
});
