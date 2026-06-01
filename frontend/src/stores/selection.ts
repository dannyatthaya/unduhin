// useSelectionStore — multi-select state for the downloads list.
//
// Lives in its own store so the floating batch-action bar, the status
// bar, individual rows, and keyboard handlers can all subscribe without
// prop-drilling. `anchorId` powers shift-click range selection; the
// caller is responsible for handing us an ordered id list (the visible
// list, in display order) at the moment of the click.

import { defineStore } from "pinia";
import { computed, ref } from "vue";

import type { DownloadId } from "@/types/tauri-bindings";

export const useSelectionStore = defineStore("selection", () => {
  const ids = ref(new Set<DownloadId>());
  const anchorId = ref<DownloadId | null>(null);

  const count = computed(() => ids.value.size);
  const empty = computed(() => ids.value.size === 0);

  function has(id: DownloadId): boolean {
    return ids.value.has(id);
  }

  function set(next: Iterable<DownloadId>, anchor: DownloadId | null = null) {
    ids.value = new Set(next);
    anchorId.value = anchor;
  }

  function clear() {
    if (ids.value.size === 0 && anchorId.value == null) return;
    ids.value = new Set();
    anchorId.value = null;
  }

  function toggle(id: DownloadId) {
    const next = new Set(ids.value);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    ids.value = next;
    anchorId.value = id;
  }

  function selectOnly(id: DownloadId) {
    ids.value = new Set([id]);
    anchorId.value = id;
  }

  /** Shift-click range selection. `ordered` must be the currently
   *  visible ids in display order at the moment the user clicked. */
  function extendTo(id: DownloadId, ordered: DownloadId[]) {
    if (anchorId.value == null || !ordered.includes(anchorId.value)) {
      selectOnly(id);
      return;
    }
    const a = ordered.indexOf(anchorId.value);
    const b = ordered.indexOf(id);
    if (a < 0 || b < 0) {
      selectOnly(id);
      return;
    }
    const [lo, hi] = a <= b ? [a, b] : [b, a];
    const next = new Set<DownloadId>();
    for (let i = lo; i <= hi; i++) next.add(ordered[i]);
    ids.value = next;
  }

  return {
    ids,
    anchorId,
    count,
    empty,
    has,
    set,
    clear,
    toggle,
    selectOnly,
    extendTo,
  };
});
