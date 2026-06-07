<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import SettingsSection from "@/components/settings/SettingsSection.vue";
import SettingCard from "@/components/settings/SettingCard.vue";
import SettingRow from "@/components/settings/SettingRow.vue";
import FolderPicker from "@/components/settings/controls/FolderPicker.vue";
import SliderField from "@/components/settings/controls/SliderField.vue";
import SpeedLimitField from "@/components/settings/controls/SpeedLimitField.vue";
import ThemeTriToggle from "@/components/settings/controls/ThemeTriToggle.vue";
import ToggleSwitch from "@/components/settings/controls/ToggleSwitch.vue";
import Select from "@/components/ui/Select.vue";

import { useGeneralSettings } from "@/composables/useGeneralSettings";
import { useSettingsFilter } from "@/composables/useSettingsFilter";
import { useLocale } from "@/composables/useLocale";
import type { LocaleSetting } from "@/i18n";

const { t } = useI18n();
const settings = useGeneralSettings();
const filter = useSettingsFilter();
const locale = useLocale();
const isHidden = (id: string) => filter.isHidden(id);

const languageOptions = computed(() => [
  { value: "system", label: t("settings.languageSystem") },
  { value: "en", label: t("settings.languageEnglish") },
  { value: "id", label: t("settings.languageIndonesian") },
]);

const deleteActionOptions = computed(() => [
  { value: "ask", label: t("settings.removeActionAsk") },
  { value: "row_only", label: t("settings.removeActionRowOnly") },
  { value: "row_and_data", label: t("settings.removeActionRowAndData") },
]);

const showRestartHint = computed(
  () => locale.resolved.value !== locale.bootLocale,
);

function onLanguage(value: string) {
  if (value === "en" || value === "id" || value === "system") {
    locale.setLocale(value satisfies LocaleSetting);
  }
}
</script>

<template>
  <SettingsSection
    :eyebrow="t('downloads.settings')"
    :title="t('settings.sectionGeneral')"
    description="Default behaviour for new downloads and the app shell. These values are used unless a specific download overrides them."
  >
    <SettingCard
      :title="t('settings.cardFiles')"
      :description="t('settings.cardFilesDesc')"
    >
      <SettingRow
        id="general/default-output-path"
        :label="t('settings.fileDefaultFolder')"
        :description="t('settings.fileDefaultFolderDesc')"
        :hidden="isHidden('general/default-output-path')"
      >
        <FolderPicker v-model="settings.defaultOutputPath.value" />
      </SettingRow>
      <SettingRow
        id="general/default-segments"
        :label="t('settings.fileDefaultSegments')"
        :description="t('settings.fileDefaultSegmentsDesc')"
        :hidden="isHidden('general/default-segments')"
      >
        <SliderField
          v-model="settings.defaultSegments.value"
          :min="1"
          :max="32"
        />
      </SettingRow>
      <SettingRow
        id="general/max-concurrent"
        :label="t('settings.fileMaxConcurrent')"
        :description="t('settings.fileMaxConcurrentDesc')"
        :hidden="isHidden('general/max-concurrent')"
      >
        <SliderField
          v-model="settings.maxConcurrent.value"
          :min="1"
          :max="16"
        />
      </SettingRow>
      <SettingRow
        id="general/always-ask-filename"
        :label="t('settings.fileAlwaysAskFilename')"
        :description="t('settings.fileAlwaysAskFilenameDesc')"
        :hidden="isHidden('general/always-ask-filename')"
      >
        <ToggleSwitch
          v-model="settings.alwaysAskFilename.value"
          :aria-label="t('settings.fileAlwaysAskFilename')"
        />
      </SettingRow>
    </SettingCard>

    <SettingCard
      :title="t('settings.cardRemoving')"
      :description="t('settings.cardRemovingDesc')"
    >
      <SettingRow
        id="general/delete-default-action"
        :label="t('settings.removeAction')"
        :description="t('settings.removeActionDesc')"
        :hidden="isHidden('general/delete-default-action')"
      >
        <div class="w-72">
          <Select
            v-model="settings.deleteDefaultAction.value"
            :options="deleteActionOptions"
          />
        </div>
      </SettingRow>
    </SettingCard>

    <SettingCard
      :title="t('settings.cardBandwidth')"
      :description="t('settings.cardBandwidthDesc')"
    >
      <SettingRow
        id="general/global-speed-limit"
        :label="t('settings.bandwidthLimit')"
        :description="t('settings.bandwidthLimitDesc')"
        :hidden="isHidden('general/global-speed-limit')"
      >
        <SpeedLimitField v-model="settings.globalSpeedBps.value" />
      </SettingRow>
    </SettingCard>

    <SettingCard
      :title="t('settings.cardAppearance')"
      :description="t('settings.cardAppearanceDesc')"
    >
      <SettingRow
        id="general/theme"
        :label="t('settings.themeLabel')"
        :description="t('settings.themeDesc')"
        :hidden="isHidden('general/theme')"
      >
        <ThemeTriToggle v-model="settings.themeMode.value" />
      </SettingRow>
      <SettingRow
        id="general/language"
        :label="t('settings.languageLabel')"
        :description="t('settings.languageDesc')"
        :hidden="isHidden('general/language')"
      >
        <div class="space-y-2">
          <div class="w-56">
            <Select
              :model-value="locale.setting.value"
              :options="languageOptions"
              @update:model-value="onLanguage"
            />
          </div>
          <p
            v-if="showRestartHint"
            class="max-w-xs text-xs text-muted-foreground"
          >
            {{ t("settings.languageRestartHint") }}
          </p>
        </div>
      </SettingRow>
    </SettingCard>

    <p class="px-1 pt-2 text-xs text-muted-foreground">
      {{ t("common.saveHint") }}
    </p>
  </SettingsSection>
</template>
