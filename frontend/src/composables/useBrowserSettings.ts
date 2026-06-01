// Typed accessors over the cached `ExtensionSettings` the Tauri pipe
// server holds for the browser extension. Reads on mount via
// `get_extension_settings`, writes via debounced
// `apply_extension_settings_patch`. The extension and the Tauri panel
// both consume the same canonical store — every edit fans out to all
// connected pipe clients as an unsolicited `SettingsChanged` frame,
// so a change in the panel reaches the extension within a single
// round-trip.
//
// Mirrors `useGeneralSettings`'s shape: each binding is a writable
// computed; writes go to a per-binding patch that's coalesced into a
// 400 ms autosave window so a fast toggle doesn't fire one Tauri
// command per keystroke.

import { computed, onScopeDispose, ref, type WritableComputedRef } from "vue";

import { invoke } from "@tauri-apps/api/core";

import { onCoreEvent, type CoreEvent } from "@/types/tauri-bindings";
import type {
  ExtensionSettings,
  HandoffMode,
  HostRule,
  SettingsPatch,
} from "@/types/wire";

const DEBOUNCE_MS = 400;

/** Same defaults the engine returns when nothing has pushed yet. */
function defaults(): ExtensionSettings {
  return {
    enabled: true,
    nativeHostName: "com.unduhin.host",
    minSizeMb: 1,
    extensionAllowlist: [],
    extensionBlocklist: ["html", "pdf", "txt", "json"],
    blockedHosts: [],
    alwaysInterceptHosts: [],
    detectHls: true,
    detectDash: true,
    verboseLogging: false,
    mode: "catch-all",
    installContextMenu: true,
    hideShelf: true,
    forwardCookies: true,
    fileTypes: [],
  };
}

function emptyPatch(): SettingsPatch {
  return {
    enabled: null,
    nativeHostName: null,
    minSizeMb: null,
    extensionAllowlist: null,
    extensionBlocklist: null,
    blockedHosts: null,
    alwaysInterceptHosts: null,
    detectHls: null,
    detectDash: null,
    verboseLogging: null,
    mode: null,
    installContextMenu: null,
    hideShelf: null,
    forwardCookies: null,
    fileTypes: null,
  };
}

export function useBrowserSettings() {
  const snapshot = ref<ExtensionSettings>(defaults());
  const loading = ref(true);
  // Per-edit overlay so the UI tracks user intent before the pipe
  // round-trips. Reset to {} whenever a fresh snapshot lands from the
  // engine — at that point the overlay's been merged in.
  const optimistic = ref<Partial<ExtensionSettings>>({});

  let pendingPatch: SettingsPatch = emptyPatch();
  let patchDirty = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let flushing: Promise<void> | null = null;

  async function refresh(): Promise<void> {
    try {
      snapshot.value = await invoke<ExtensionSettings>("get_extension_settings");
      optimistic.value = {};
    } catch (err) {
      console.warn("get_extension_settings failed", err);
    } finally {
      loading.value = false;
    }
  }

  async function flush(): Promise<void> {
    if (!patchDirty) return;
    const patch = pendingPatch;
    pendingPatch = emptyPatch();
    patchDirty = false;
    try {
      const next = await invoke<ExtensionSettings>(
        "apply_extension_settings_patch",
        { patch },
      );
      snapshot.value = next;
      optimistic.value = {};
    } catch (err) {
      console.warn("apply_extension_settings_patch failed", err);
    }
  }

  function scheduleFlush(): void {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      flushing = flush().finally(() => {
        flushing = null;
      });
    }, DEBOUNCE_MS);
  }

  function setField<K extends keyof ExtensionSettings>(
    key: K,
    value: ExtensionSettings[K],
  ): void {
    optimistic.value = { ...optimistic.value, [key]: value };
    (pendingPatch as Record<string, unknown>)[key] = value;
    patchDirty = true;
    scheduleFlush();
  }

  // Effective view = engine snapshot, with any in-flight user edits
  // layered on top. Stops the UI from "snapping back" between a click
  // and the autosave landing.
  const view = computed<ExtensionSettings>(() => ({
    ...snapshot.value,
    ...optimistic.value,
  }));

  function bind<K extends keyof ExtensionSettings>(
    key: K,
  ): WritableComputedRef<ExtensionSettings[K]> {
    return computed({
      get: () => view.value[key],
      set: (next) => setField(key, next),
    });
  }

  // Refresh on `SettingsChanged` echoes — the extension push lands as
  // a fresh `Outbound::SettingsChanged`, which the pipe server caches
  // and is then visible via `get_extension_settings`. The pipe doesn't
  // emit its own Tauri-side event today; rather than add one, we
  // refresh on `pipe_listening` (panel reopen) and rely on the
  // explicit `apply_extension_settings_patch` reply for the panel's
  // own writes. The extension's chrome.storage.onChanged → setSettings
  // push reaches the cache; the panel re-syncs on next mount.
  let unlisten: (() => void) | null = null;
  function handle(event: CoreEvent): void {
    if (event.type === "pipe_listening") void refresh();
  }
  void (async () => {
    unlisten = await onCoreEvent(handle);
  })();

  void refresh();

  onScopeDispose(() => {
    if (timer) {
      clearTimeout(timer);
      timer = null;
      void flush();
    }
    if (unlisten) unlisten();
  });

  return {
    snapshot,
    view,
    loading,
    refresh,
    flush: async () => {
      if (timer) {
        clearTimeout(timer);
        timer = null;
      }
      await (flushing ?? flush());
    },
    bindings: {
      mode: bind("mode") as WritableComputedRef<HandoffMode>,
      installContextMenu: bind("installContextMenu") as WritableComputedRef<boolean>,
      hideShelf: bind("hideShelf") as WritableComputedRef<boolean>,
      forwardCookies: bind("forwardCookies") as WritableComputedRef<boolean>,
      enabled: bind("enabled") as WritableComputedRef<boolean>,
      minSizeMb: bind("minSizeMb") as WritableComputedRef<number>,
      fileTypes: bind("fileTypes") as WritableComputedRef<string[]>,
      blockedHosts: bind("blockedHosts") as WritableComputedRef<HostRule[]>,
      alwaysInterceptHosts: bind("alwaysInterceptHosts") as WritableComputedRef<HostRule[]>,
    },
    /** Convenience writers for the domain-rules card. */
    setRules(kind: "block" | "allow", rules: HostRule[]): void {
      const key = kind === "block" ? "blockedHosts" : "alwaysInterceptHosts";
      setField(key, rules);
    },
    /** Convenience for the file-types pill grid. */
    toggleFileType(ext: string): void {
      const normalized = ext.trim().toLowerCase().replace(/^\./, "");
      if (!normalized) return;
      const current = view.value.fileTypes;
      const next = current.includes(normalized)
        ? current.filter((x) => x !== normalized)
        : [...current, normalized];
      setField("fileTypes", next);
    },
  };
}
