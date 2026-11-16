// Search results show each matching section's own page, so the controls
// are the live ones. A page rendered that way is "embedded": it drops its
// heading, and each group shows a breadcrumb instead of a title.

import { inject, type InjectionKey } from "vue";

import type { SettingsSectionKey } from "@/lib/settingsManifest";

export interface SettingsEmbed {
  section: SettingsSectionKey;
}

export const SETTINGS_EMBED_KEY: InjectionKey<SettingsEmbed> = Symbol("settings-embed");

/** The section this page is embedded for, or `null` on its own route. */
export function useSettingsEmbed(): SettingsEmbed | null {
  return inject(SETTINGS_EMBED_KEY, null);
}
