// Theme with three modes: light, dark, system. The persisted `theme_mode`
// setting key is the source of truth; the localStorage fallback
// (`unduhin:theme`) keeps the very first paint correct before the
// settings store has refreshed from the backend.

import { computed, effectScope, ref, watch } from "vue";
import { useDark, usePreferredDark } from "@vueuse/core";

import { useSettingsStore } from "@/stores/settings";

export type ThemeMode = "light" | "dark" | "system";

const STORAGE_KEY = "unduhin:theme-mode";

function readStored(): ThemeMode {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === "light" || raw === "dark" || raw === "system") return raw;
  } catch {
    /* ignore */
  }
  return "system";
}

// Module-singleton so every component sees the same state.
const mode = ref<ThemeMode>(readStored());

const isDarkRef = useDark({
  selector: "html",
  attribute: "class",
  valueDark: "dark",
  valueLight: "",
  storageKey: "unduhin:theme",
});

const systemPrefersDark = usePreferredDark();

const resolvedDark = computed(() => {
  if (mode.value === "dark") return true;
  if (mode.value === "light") return false;
  return systemPrefersDark.value;
});

watch(
  resolvedDark,
  (v) => {
    isDarkRef.value = v;
  },
  { immediate: true },
);

watch(mode, (v) => {
  try {
    localStorage.setItem(STORAGE_KEY, v);
  } catch {
    /* ignore */
  }
});

let _wired = false;

/**
 * Hook the theme to the persisted `theme_mode` setting. Safe to call from
 * multiple components; only the first call wires up the watcher.
 *
 * The watcher runs in a detached effect scope so it lives for the app's
 * lifetime regardless of which component first calls `useTheme()`. Binding
 * it to the caller's component scope was the bug: the only caller was
 * `AppTopBar`, which unmounts on the Settings route, so toggling the theme
 * there stopped propagating to the DOM until the next app start.
 */
function wireToSettings() {
  if (_wired) return;
  _wired = true;
  const settings = useSettingsStore();
  // First adoption: read the current value once the store has loaded.
  effectScope(true).run(() => {
    watch(
      () => settings.values["theme_mode"],
      (v) => {
        if (v === "light" || v === "dark" || v === "system") {
          if (mode.value !== v) mode.value = v;
        }
      },
      { immediate: true },
    );
  });
}

export function useTheme() {
  wireToSettings();

  function setMode(next: ThemeMode) {
    mode.value = next;
    const settings = useSettingsStore();
    void settings.set("theme_mode", next);
  }

  function toggle() {
    setMode(resolvedDark.value ? "light" : "dark");
  }

  return {
    mode,
    isDark: resolvedDark,
    setMode,
    toggle,
  };
}
