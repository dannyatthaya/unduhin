// Language management for the UI. Mirrors `useTheme.ts`:
// localStorage holds the most recent value so the first paint is in
// the right language; the persisted `language` setting in core is the
// long-term source of truth; the two are kept in sync via a watcher.
//
// `language` settings are one of `en | id | system`; `system` resolves
// via `navigator.language` on boot and any time the setting changes.

import { computed, ref, watch } from "vue";

import {
  bootLocale,
  persistLocale,
  readStoredLocale,
  resolveLocale,
  setI18nLocale,
} from "@/i18n";
import type { LocaleSetting, ResolvedLocale } from "@/i18n";
import { useSettingsStore } from "@/stores/settings";

const setting = ref<LocaleSetting>(readStoredLocale());
const resolved = computed<ResolvedLocale>(() => resolveLocale(setting.value));

// Persist + apply on every change so cross-window flips land in both
// surfaces.
watch(
  setting,
  (v) => {
    persistLocale(v);
  },
  { immediate: false },
);

watch(
  resolved,
  (v) => {
    setI18nLocale(v);
  },
  { immediate: true },
);

let _wired = false;

function wireToSettings() {
  if (_wired) return;
  _wired = true;
  const settings = useSettingsStore();
  watch(
    () => settings.values["language"],
    (v) => {
      if (v === "en" || v === "id" || v === "system") {
        if (setting.value !== v) setting.value = v;
      }
    },
    { immediate: true },
  );
}

export function useLocale() {
  wireToSettings();

  function setLocale(next: LocaleSetting) {
    if (setting.value === next) return;
    setting.value = next;
    const settings = useSettingsStore();
    void settings.set("language", next);
  }

  return {
    setting,
    resolved,
    bootLocale,
    setLocale,
  };
}
