// Singleton delete-confirmation dialog state.
//
// The Remove action lives in many places (row More menu, batch action
// bar, detail-pane footer, future keyboard shortcut). Each call site
// shouldn't reinvent the prompt UI, and they shouldn't have to read the
// `delete_default_action` setting themselves. They call
// `requestDelete(ids)` and we handle the rest:
//
// - When the setting is "row_only" or "row_and_data", the corresponding
//   delete runs immediately with no prompt.
// - When the setting is "ask" (the default), we expose state for the
//   <DeleteConfirmDialog/> component (mounted once in App.vue) and
//   resolve the returned promise after the user picks an option.

import { ref } from "vue";

import { api, type DownloadId } from "@/types/tauri-bindings";
import { useSettingsStore } from "@/stores/settings";
import { useToast } from "@/composables/useToast";

export type DeleteChoice = "cancel" | "row_only" | "row_and_data";

interface PendingRequest {
  ids: DownloadId[];
  resolve: (choice: DeleteChoice) => void;
}

const pending = ref<PendingRequest | null>(null);

function readSetting(): "ask" | "row_only" | "row_and_data" {
  const v = useSettingsStore().values["delete_default_action"];
  if (v === "row_only" || v === "row_and_data") return v;
  return "ask";
}

async function performDelete(ids: DownloadId[], withData: boolean) {
  const toast = useToast();
  const failures: string[] = [];
  await Promise.all(
    ids.map(async (id) => {
      try {
        await api.removeDownload(id, withData);
      } catch (e: unknown) {
        failures.push((e as { message?: string })?.message ?? "unknown error");
      }
    }),
  );
  if (failures.length > 0) {
    toast.push(
      withData
        ? `Removed ${ids.length - failures.length} of ${ids.length}; ${failures[0]}`
        : `Couldn't remove ${failures.length} item${failures.length === 1 ? "" : "s"}: ${failures[0]}`,
      "error",
    );
  }
}

export function useDeleteConfirm() {
  function answer(choice: DeleteChoice) {
    const req = pending.value;
    pending.value = null;
    req?.resolve(choice);
  }

  async function requestDelete(ids: DownloadId[]): Promise<void> {
    if (ids.length === 0) return;
    const setting = readSetting();
    if (setting === "row_only") {
      await performDelete(ids, false);
      return;
    }
    if (setting === "row_and_data") {
      await performDelete(ids, true);
      return;
    }
    const choice = await new Promise<DeleteChoice>((resolve) => {
      pending.value = { ids, resolve };
    });
    if (choice === "cancel") return;
    await performDelete(ids, choice === "row_and_data");
  }

  return { pending, requestDelete, answer };
}
