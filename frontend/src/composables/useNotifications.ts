// useNotifications — subscribes to the Tauri event channel and surfaces
// native OS toasts for the three opt-in completion signals:
// `completed`, `failed`, and `queue_emptied`. Each is gated on the
// matching `notify_*` setting before firing.
//
// Permission for the OS notification surface is requested lazily on the
// first event we would actually fire — pestering the user with a system
// prompt at app launch would be rude.
//
// The Settings → Behaviour preview buttons keep using
// `useNotificationPreview` (sample copy, fires regardless of `notify_*`
// state) so the two paths don't need to share a debounce.
//
// Quiet-hours suppression: each handler short-circuits
// when the global `quiet_hours` window is active. The schedules store
// keeps its `quietHours` snapshot in sync via the `schedules_changed`
// event subscription wired in `useUnduhinEvents`, so reads here are a
// reactive store lookup — no Tauri round-trip per event.

import { onBeforeUnmount, onMounted } from "vue";

import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

import { onCoreEvent } from "@/types/tauri-bindings";
import type { CoreEvent } from "@/types/tauri-bindings";
import { useDownloadsStore } from "@/stores/downloads";
import { useSchedulesStore } from "@/stores/schedules";
import { useSettingsStore } from "@/stores/settings";
import { formatBytes } from "@/lib/format";
import { tGlobal } from "@/i18n";

// Permission state cache so we don't round-trip to the plugin on every
// event. The plugin caches internally too, but reading is async — this
// keeps the hot path synchronous after the first successful grant.
let cachedPermission: "granted" | "denied" | "default" | null = null;

async function ensurePermission(): Promise<boolean> {
  if (cachedPermission === "granted") return true;
  if (cachedPermission === "denied") return false;
  const granted = await isPermissionGranted();
  if (granted) {
    cachedPermission = "granted";
    return true;
  }
  const result = await requestPermission();
  cachedPermission = result;
  return result === "granted";
}

function readBool(values: Record<string, unknown>, key: string, fallback: boolean): boolean {
  const v = values[key];
  return typeof v === "boolean" ? v : fallback;
}

export function useNotifications() {
  const settings = useSettingsStore();
  const downloads = useDownloadsStore();
  const schedules = useSchedulesStore();

  let unlisten: (() => void) | undefined;

  /** True when a global `quiet_hours` window is currently active. The
   *  schedules store keeps this fresh via the `schedules_changed`
   *  subscription — see `useUnduhinEvents`. */
  function isQuiet(): boolean {
    return schedules.quietHours?.active === true;
  }

  async function handleCompleted(id: number, bytes: number) {
    if (!readBool(settings.values, "notify_complete", true)) return;
    if (isQuiet()) return;
    if (!(await ensurePermission())) return;
    const rec = downloads.records.get(id);
    const title = tGlobal("notify.completedTitle");
    const body = rec
      ? tGlobal("notify.completedBodyNamed", {
          filename: rec.filename,
          size: formatBytes(bytes),
        })
      : tGlobal("notify.completedBodyAnonymous", {
          id,
          size: formatBytes(bytes),
        });
    sendNotification({ title, body });
    // Tauri's notification plugin doesn't yet expose action buttons on
    // Windows, so we wire the "Open folder" affordance as the toast's
    // click action by reusing the existing `revealItemInDir`. If the
    // user clicks the toast quickly we'll reveal; otherwise this is a
    // no-op silently absorbed by the OS.
    //
    // The plugin's click event listener API isn't stable across desktops
    // either — leaving this as a TODO for the cross-platform follow-up;
    // for Windows the path is at least logged so power users can grep.
    if (rec) {
      // eslint-disable-next-line no-console
      console.info("[notify] completed", { id, path: rec.output_path });
    }
  }

  async function handleFailed(id: number, error: string) {
    if (!readBool(settings.values, "notify_fail", true)) return;
    if (isQuiet()) return;
    if (!(await ensurePermission())) return;
    const rec = downloads.records.get(id);
    const title = tGlobal("notify.failedTitle");
    const body = rec
      ? tGlobal("notify.failedBodyNamed", {
          filename: rec.filename,
          error: truncate(error, 120),
        })
      : tGlobal("notify.failedBodyAnonymous", {
          id,
          error: truncate(error, 120),
        });
    sendNotification({ title, body });
  }

  async function handleQueueEmptied() {
    if (!readBool(settings.values, "notify_queue_empty", false)) return;
    if (isQuiet()) return;
    if (!(await ensurePermission())) return;
    sendNotification({
      title: tGlobal("notify.queueEmptyTitle"),
      body: tGlobal("notify.queueEmptyBody"),
    });
  }

  function route(event: CoreEvent) {
    switch (event.type) {
      case "completed":
        void handleCompleted(event.id, event.bytes);
        break;
      case "failed":
        void handleFailed(event.id, event.error);
        break;
      case "queue_emptied":
        void handleQueueEmptied();
        break;
      default:
        break;
    }
  }

  onMounted(async () => {
    unlisten = await onCoreEvent(route);
  });

  onBeforeUnmount(() => {
    unlisten?.();
  });

  // Exposed so the "Open folder" affordance from a completed toast can
  // still be invoked manually (e.g. via the row's context menu) without
  // duplicating the plugin import.
  return { revealFolder: revealItemInDir };
}

function truncate(s: string, max: number): string {
  return s.length <= max ? s : s.slice(0, max - 1) + "…";
}
