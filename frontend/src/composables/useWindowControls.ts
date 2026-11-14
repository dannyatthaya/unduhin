// Minimize / maximize / close for a custom title bar, plus the live
// "is the window maximized" flag the maximize button needs for its icon.
//
// macOS draws its own traffic lights, so the listener is skipped there:
// nothing reads the flag and every resize would cost an IPC round trip.

import { onBeforeUnmount, onMounted, ref, type Ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { isMacOS } from "@/lib/platform";

export interface WindowControls {
  isMaximized: Ref<boolean>;
  minimize: () => void;
  toggleMaximize: () => void;
  close: () => void;
}

export function useWindowControls(): WindowControls {
  const win = getCurrentWindow();
  const isMaximized = ref(false);
  let unlisten: (() => void) | undefined;
  let disposed = false;

  async function syncMaximized() {
    try {
      isMaximized.value = await win.isMaximized();
    } catch {
      // Only the maximize button's icon depends on this; keep the last value.
    }
  }

  onMounted(async () => {
    if (isMacOS) return;
    await syncMaximized();
    const stop = await win.onResized(() => void syncMaximized());
    // The component can unmount while the subscription is still being set
    // up (a quick hop to Settings); release it instead of leaking it.
    if (disposed) stop();
    else unlisten = stop;
  });

  onBeforeUnmount(() => {
    disposed = true;
    unlisten?.();
  });

  return {
    isMaximized,
    minimize: () => void win.minimize(),
    toggleMaximize: () => void win.toggleMaximize(),
    close: () => void win.close(),
  };
}
