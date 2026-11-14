// Tree-scoped state the downloads list shares with its rows, so rows don't
// need it drilled through the group component. Provided by DownloadsView.

import type { InjectionKey, Ref } from "vue";

import type { DownloadId } from "@/types/tauri-bindings";

export interface DownloadsListContext {
  /** Ids of the rows currently rendered, in display order (range select). */
  orderedIds: () => DownloadId[];
  /** Size and Added columns are hidden (narrow window or details open). */
  compact: Readonly<Ref<boolean>>;
  /** The row that owns the list's single tab stop (roving tabindex). */
  activeRowId: Readonly<Ref<DownloadId | null>>;
  setActiveRow: (id: DownloadId) => void;
}

export const DOWNLOADS_LIST_KEY: InjectionKey<DownloadsListContext> =
  Symbol("downloads-list");
