// Tracks which transient overlay (RowMenu, ContextMenu, future popovers)
// is currently open. Each overlay claims the slot when it opens and
// releases it when it closes. Watching `activeId` lets a component
// self-dismiss when something else takes over — without this, a row's
// three-dots popover stays open while a right-click ContextMenu opens
// on top because each component's `onClickOutside` only fires for
// clicks outside its own ref.

import { defineStore } from "pinia";
import { ref } from "vue";

export type OverlayId = symbol;

export const useOverlaysStore = defineStore("overlays", () => {
  const activeId = ref<OverlayId | null>(null);

  function claim(id: OverlayId) {
    activeId.value = id;
  }

  function release(id: OverlayId) {
    if (activeId.value === id) activeId.value = null;
  }

  return { activeId, claim, release };
});
