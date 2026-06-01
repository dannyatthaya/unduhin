// Fires a sample OS notification — used by the Preview buttons next to
// the notification toggles in Settings → Behaviour.

import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";

export type NotificationKind = "complete" | "fail" | "queue-empty";

const SAMPLES: Record<NotificationKind, { title: string; body: string }> = {
  complete: {
    title: "Download complete",
    body: "sample-archive.zip · 124 MB",
  },
  fail: {
    title: "Download failed",
    body: "sample-archive.zip · network error",
  },
  "queue-empty": {
    title: "Queue empty",
    body: "All downloads have finished.",
  },
};

export async function previewNotification(kind: NotificationKind): Promise<boolean> {
  let allowed = await isPermissionGranted();
  if (!allowed) {
    const result = await requestPermission();
    allowed = result === "granted";
  }
  if (!allowed) return false;
  sendNotification(SAMPLES[kind]);
  return true;
}
