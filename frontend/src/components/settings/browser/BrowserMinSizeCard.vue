<script setup lang="ts">
import { useI18n } from "vue-i18n";

import SettingCard from "@/components/settings/SettingCard.vue";
import SettingRow from "@/components/settings/SettingRow.vue";
import NumberStepper from "@/components/settings/controls/NumberStepper.vue";
import type { useBrowserSettings } from "@/composables/useBrowserSettings";

const props = defineProps<{
  settings: ReturnType<typeof useBrowserSettings>;
}>();

const { t } = useI18n();
</script>

<template>
  <SettingCard
    :title="t('settings.cardBrowserMinSize')"
    :description="t('settings.cardBrowserMinSizeDesc')"
  >
    <SettingRow
      id="browser/min-size"
      :label="t('settings.browserMinSize')"
      :description="t('settings.browserMinSizeDesc')"
    >
      <NumberStepper
        v-model="props.settings.bindings.minSizeMb.value"
        :min="0"
        :max="1024"
        :step="1"
        :suffix="t('settings.browserMbSuffix')"
      />
    </SettingRow>
  </SettingCard>
</template>
