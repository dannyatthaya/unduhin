// One rename dialog for the whole downloads screen. The list row, its
// right-click menu and the details panel all ask for it by download id;
// DownloadsView mounts the dialog itself.

import { ref } from "vue";

import type { DownloadId } from "@/types/tauri-bindings";

const targetId = ref<DownloadId | null>(null);

export function useRenameDialog() {
  function request(id: DownloadId): void {
    targetId.value = id;
  }

  function close(): void {
    targetId.value = null;
  }

  return { targetId, request, close };
}
