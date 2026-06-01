<script setup lang="ts">
import { computed, type WritableComputedRef } from "vue";
import { useI18n } from "vue-i18n";

import SettingCard from "@/components/settings/SettingCard.vue";
import SettingRow from "@/components/settings/SettingRow.vue";
import ToggleSwitch from "@/components/settings/controls/ToggleSwitch.vue";

import { useSettingsStore } from "@/stores/settings";
import type { useBrowserSettings } from "@/composables/useBrowserSettings";

// Clipboard watcher toggle.
//
// The toggle binds to the Tauri-canonical `watch_clipboard` setting
// (not the extension's `chrome.storage`) because the OS clipboard
// belongs to the desktop session, not the browser tab. The actual
// polling lives in `useClipboardCapture` mounted in `App.vue`; this
// card is just the toggle + a helper hint when the user hasn't yet
// configured any file types to match against.

const props = defineProps<{
  settings: ReturnType<typeof useBrowserSettings>;
}>();

const { t } = useI18n();
const store = useSettingsStore();

const watchClipboard: WritableComputedRef<boolean> = computed({
  get(): boolean {
    return store.values["watch_clipboard"] === true;
  },
  set(next: boolean) {
    void store.set("watch_clipboard", next);
  },
});

const fileTypesEmpty = computed<boolean>(
  () => props.settings.view.value.fileTypes.length === 0,
);
</script>

<template>
  <SettingCard
    :title="t('settings.cardBrowserClipboard')"
    :description="t('settings.cardBrowserClipboardDesc')"
  >
    <SettingRow
      id="browser/watch-clipboard"
      :label="t('settings.browserClipboardToggle')"
      :description="t('settings.browserClipboardToggleDesc')"
    >
      <ToggleSwitch v-model="watchClipboard" />
    </SettingRow>
    <p
      v-if="watchClipboard && fileTypesEmpty"
      class="px-5 pb-4 text-xs text-warn"
    >
      {{ t("settings.browserClipboardEmptyFileTypes") }}
    </p>
  </SettingCard>
</template>
