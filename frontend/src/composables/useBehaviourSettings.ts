// Typed accessors for the Behaviour section. Mirrors the shape of
// `useGeneralSettings.ts`.

import { computed, type WritableComputedRef } from "vue";

import { useSettingsStore } from "@/stores/settings";

export type CloseBehavior = "minimize" | "exit" | "ask";

function typedBool(key: string, fallback: boolean): WritableComputedRef<boolean> {
  const s = useSettingsStore();
  return computed({
    get() {
      const v = s.values[key];
      return typeof v === "boolean" ? v : fallback;
    },
    set(next) {
      void s.set(key, next);
    },
  });
}

function typedClose(key: string): WritableComputedRef<CloseBehavior> {
  const s = useSettingsStore();
  return computed({
    get() {
      const v = s.values[key];
      if (v === "minimize" || v === "exit" || v === "ask") return v;
      return "ask";
    },
    set(next) {
      void s.set(key, next);
    },
  });
}

export function useBehaviourSettings() {
  return {
    autostart: typedBool("autostart", false),
    startMinimized: typedBool("start_minimized", false),
    closeBehavior: typedClose("close_behavior"),
    confirmOnQuit: typedBool("confirm_on_quit", true),
    notifyComplete: typedBool("notify_complete", true),
    notifyFail: typedBool("notify_fail", true),
    notifyQueueEmpty: typedBool("notify_queue_empty", false),
  };
}
