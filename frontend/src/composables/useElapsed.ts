import { computed, onBeforeUnmount, ref, watch, type ComputedRef } from "vue";

import type { DownloadRecord } from "@/types/tauri-bindings";

const TICK_MS = 1000;

function isLive(status: DownloadRecord["status"]): boolean {
  return (
    status === "active" ||
    status === "muxing" ||
    status === "queued" ||
    status === "paused"
  );
}

/**
 * Reactive elapsed-seconds value for a download. Ticks once per second
 * while the download is live; freezes at `completed_at - created_at`
 * once finished so the value doesn't drift after completion.
 */
export function useElapsedSeconds(
  record: () => DownloadRecord | null | undefined,
): ComputedRef<number | null> {
  const now = ref(Date.now());
  let handle: ReturnType<typeof setInterval> | null = null;

  function startTicking() {
    if (handle != null) return;
    handle = setInterval(() => {
      now.value = Date.now();
    }, TICK_MS);
  }

  function stopTicking() {
    if (handle == null) return;
    clearInterval(handle);
    handle = null;
  }

  watch(
    () => record()?.status,
    (status) => {
      if (status && isLive(status)) startTicking();
      else stopTicking();
    },
    { immediate: true },
  );

  onBeforeUnmount(stopTicking);

  return computed(() => {
    const r = record();
    if (!r) return null;
    const startedMs = Date.parse(r.created_at);
    if (Number.isNaN(startedMs)) return null;
    const endMs =
      r.completed_at && !isLive(r.status)
        ? Date.parse(r.completed_at)
        : now.value;
    if (Number.isNaN(endMs)) return null;
    return Math.max(0, (endMs - startedMs) / 1000);
  });
}
