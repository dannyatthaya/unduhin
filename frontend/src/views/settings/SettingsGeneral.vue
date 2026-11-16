<script setup lang="ts">
// Settings → General: defaults for new downloads and for the app itself.

import { computed, useId } from "vue";
import { useI18n } from "vue-i18n";

import GeneralStorageCard from "@/components/settings/GeneralStorageCard.vue";
import SavedTick from "@/components/settings/SavedTick.vue";
import SettingCard from "@/components/settings/SettingCard.vue";
import SettingRow from "@/components/settings/SettingRow.vue";
import SettingsSection from "@/components/settings/SettingsSection.vue";
import FolderPicker from "@/components/settings/controls/FolderPicker.vue";
import NumberStepper from "@/components/settings/controls/NumberStepper.vue";
import SpeedLimitField from "@/components/settings/controls/SpeedLimitField.vue";
import ToggleSwitch from "@/components/settings/controls/ToggleSwitch.vue";
import SegmentedControl, { type SegmentOption } from "@/components/ui/SegmentedControl.vue";
import Select from "@/components/ui/Select.vue";

import { useSettingsStore } from "@/stores/settings";
import { useGeneralSettings, type DeleteAction } from "@/composables/useGeneralSettings";
import { useLocale } from "@/composables/useLocale";
import { useSavedFlash } from "@/composables/useSavedFlash";
import type { ThemeMode } from "@/composables/useTheme";
import { useToast } from "@/composables/useToast";
import type { LocaleSetting } from "@/i18n";
import type { SettingValue } from "@/types/tauri-bindings";

const { t } = useI18n();
const store = useSettingsStore();
const settings = useGeneralSettings();
const locale = useLocale();
const toast = useToast();
const saved = useSavedFlash();

const folderId = useId();
const languageId = useId();

const SEGMENTS_MAX = 32;
const CONCURRENT_MAX = 16;

/** Store a value and, once it is stored, say so beside the control. */
async function save(rowId: string, key: string, value: SettingValue): Promise<void> {
  try {
    await store.set(key, value);
    saved.flash(rowId);
  } catch (e: unknown) {
    const message = (e as { message?: string })?.message ?? String(e);
    toast.push(t("settings.saveFailed", { error: message }), "error");
  }
}

const removeOptions = computed<SegmentOption<DeleteAction>[]>(() => [
  { value: "ask", label: t("settings.removeActionAsk") },
  { value: "row_only", label: t("settings.removeActionRowOnly") },
  { value: "row_and_data", label: t("settings.removeActionRowAndData") },
]);

// The swatches show the two page backgrounds, whatever theme is on now.
const LIGHT_PAGE = "#fbfaf6";
const DARK_PAGE = "#0a0a0a";
const themeOptions = computed<SegmentOption<ThemeMode>[]>(() => [
  {
    value: "system",
    label: t("settings.themeSystem"),
    swatch: `linear-gradient(135deg, ${LIGHT_PAGE} 0 50%, ${DARK_PAGE} 50% 100%)`,
  },
  { value: "light", label: t("settings.themeLight"), swatch: LIGHT_PAGE },
  { value: "dark", label: t("settings.themeDark"), swatch: DARK_PAGE },
]);

const languageOptions = computed<{ value: LocaleSetting; label: string }[]>(() => [
  { value: "system", label: t("settings.languageSystem") },
  { value: "en", label: t("settings.languageEnglish") },
  { value: "id", label: t("settings.languageIndonesian") },
]);

const showRestartHint = computed(() => locale.resolved.value !== locale.bootLocale);
</script>

<template>
  <SettingsSection
    :title="t('settings.sectionGeneral')"
    :description="t('settings.sectionGeneralDesc')"
  >
    <SettingCard :title="t('settings.cardFiles')">
      <SettingRow
        id="general/default-output-path"
        :label="t('settings.fileDefaultFolder')"
        :description="t('settings.fileDefaultFolderDesc')"
        :label-for="folderId"
        stacked
      >
        <FolderPicker
          :id="folderId"
          :model-value="settings.defaultOutputPath.value"
          full
          @update:model-value="save('general/default-output-path', 'default_output_path', $event)"
        />
      </SettingRow>
      <SettingRow
        id="general/always-ask-filename"
        v-slot="{ labelId }"
        :label="t('settings.fileAlwaysAskFilename')"
        :description="t('settings.fileAlwaysAskFilenameDesc')"
      >
        <ToggleSwitch v-model="settings.alwaysAskFilename.value" :labelledby="labelId" />
      </SettingRow>
      <SettingRow
        id="general/delete-default-action"
        v-slot="{ labelId }"
        :label="t('settings.removeAction')"
        :description="t('settings.removeActionDesc')"
      >
        <SegmentedControl
          v-model="settings.deleteDefaultAction.value"
          :options="removeOptions"
          :labelledby="labelId"
        />
      </SettingRow>
    </SettingCard>

    <SettingCard :title="t('settings.cardBandwidth')">
      <SettingRow
        id="general/global-speed-limit"
        v-slot="{ labelId }"
        :label="t('settings.bandwidthLimit')"
        :description="t('settings.bandwidthLimitDesc')"
      >
        <SavedTick :show="saved.isSaved('general/global-speed-limit')" />
        <SpeedLimitField
          :model-value="settings.globalSpeedBps.value"
          :labelledby="labelId"
          @update:model-value="save('general/global-speed-limit', 'global_speed_limit_bps', $event)"
        />
      </SettingRow>
      <SettingRow
        id="general/default-segments"
        v-slot="{ labelId }"
        :label="t('settings.fileDefaultSegments')"
        :description="t('settings.fileDefaultSegmentsDesc')"
      >
        <SavedTick :show="saved.isSaved('general/default-segments')" />
        <NumberStepper
          :model-value="settings.defaultSegments.value"
          :min="1"
          :max="SEGMENTS_MAX"
          :labelledby="labelId"
          :decrease-label="t('addUrl.fewerConnections')"
          :increase-label="t('addUrl.moreConnections')"
          @update:model-value="save('general/default-segments', 'default_segments', $event)"
        />
      </SettingRow>
      <SettingRow
        id="general/max-concurrent"
        v-slot="{ labelId }"
        :label="t('settings.fileMaxConcurrent')"
        :description="t('settings.fileMaxConcurrentDesc')"
      >
        <SavedTick :show="saved.isSaved('general/max-concurrent')" />
        <NumberStepper
          :model-value="settings.maxConcurrent.value"
          :min="1"
          :max="CONCURRENT_MAX"
          :labelledby="labelId"
          :decrease-label="t('settings.fewer')"
          :increase-label="t('settings.more')"
          @update:model-value="save('general/max-concurrent', 'max_concurrent_downloads', $event)"
        />
      </SettingRow>
    </SettingCard>

    <SettingCard :title="t('settings.cardAppearance')">
      <SettingRow
        id="general/theme"
        v-slot="{ labelId }"
        :label="t('settings.themeLabel')"
        :description="t('settings.themeDesc')"
      >
        <SegmentedControl
          v-model="settings.themeMode.value"
          :options="themeOptions"
          :labelledby="labelId"
        />
      </SettingRow>
      <SettingRow
        id="general/language"
        :label="t('settings.languageLabel')"
        :description="showRestartHint ? t('settings.languageRestartHint') : t('settings.languageDesc')"
        :label-for="languageId"
      >
        <div class="w-[220px]">
          <Select
            :id="languageId"
            :model-value="locale.setting.value"
            :options="languageOptions"
            @update:model-value="locale.setLocale($event)"
          />
        </div>
      </SettingRow>
    </SettingCard>

    <GeneralStorageCard />
  </SettingsSection>
</template>
