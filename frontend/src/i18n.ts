// vue-i18n instance shared between the main and popover entries.
// Locale boot order:
//   1. `main.ts` (or `popover/main.ts`) reads localStorage for the most
//      recent value so the very first paint is in the right language.
//   2. `useLocale().wireToSettings()` adopts the persisted `language`
//      setting from the backend on app mount and live-switches on
//      `SettingChanged{key:"language"}`.
// The fallback locale is `en` so a missing key surfaces obviously.

import { createI18n } from "vue-i18n";

import en from "./locales/en.json";
import id from "./locales/id.json";

export type LocaleSetting = "en" | "id" | "system";
export type ResolvedLocale = "en" | "id";

const STORAGE_KEY = "unduhin:locale";

export function resolveSystemLocale(): ResolvedLocale {
  try {
    const tag = (navigator?.language ?? "en").toLowerCase();
    if (tag === "id" || tag.startsWith("id-")) return "id";
    return "en";
  } catch {
    return "en";
  }
}

export function readStoredLocale(): LocaleSetting {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === "en" || raw === "id" || raw === "system") return raw;
  } catch {
    /* ignore */
  }
  return "system";
}

export function persistLocale(value: LocaleSetting): void {
  try {
    localStorage.setItem(STORAGE_KEY, value);
  } catch {
    /* ignore */
  }
}

export function resolveLocale(value: LocaleSetting): ResolvedLocale {
  return value === "system" ? resolveSystemLocale() : value;
}

const initialSetting = readStoredLocale();

export const i18n = createI18n({
  legacy: false,
  globalInjection: true,
  locale: resolveLocale(initialSetting),
  fallbackLocale: "en",
  messages: { en, id },
  missingWarn: false,
  fallbackWarn: false,
});

// The active LocaleSetting (en/id/system) at module load time. The
// "restart to fully apply" hint in Settings only shows when the active
// resolved locale diverges from this boot value.
export const bootLocale: ResolvedLocale = resolveLocale(initialSetting);

/** Programmatically switch the active locale. Idempotent. */
export function setI18nLocale(next: ResolvedLocale): void {
  i18n.global.locale.value = next;
}

/** Look up a translation key from outside a Vue component
 *  (background event handlers, etc.). Mirrors `t()` for the active
 *  locale; falls back to `en` automatically via the i18n instance. */
export function tGlobal(key: string, params?: Record<string, unknown>): string {
  return i18n.global.t(key, params ?? {});
}
