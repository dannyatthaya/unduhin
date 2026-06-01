<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Inbox, HelpCircle, Filter, ArrowRightFromLine } from "lucide-vue-next";

import SettingCard from "@/components/settings/SettingCard.vue";
import SettingRow from "@/components/settings/SettingRow.vue";
import ToggleSwitch from "@/components/settings/controls/ToggleSwitch.vue";
import TileChoice, {
  type TileOption,
} from "@/components/settings/controls/TileChoice.vue";
import type { useBrowserSettings } from "@/composables/useBrowserSettings";
import type { HandoffMode } from "@/types/wire";

const props = defineProps<{
  settings: ReturnType<typeof useBrowserSettings>;
}>();

const { t } = useI18n();

const modeOptions = computed<TileOption<HandoffMode>[]>(() => [
  {
    value: "catch-all",
    label: t("settings.browserModeCatchAll"),
    hint: t("settings.browserModeCatchAllHint"),
    icon: Inbox,
  },
  {
    value: "ask-first",
    label: t("settings.browserModeAskFirst"),
    hint: t("settings.browserModeAskFirstHint"),
    icon: HelpCircle,
  },
  {
    value: "rules-only",
    label: t("settings.browserModeRulesOnly"),
    hint: t("settings.browserModeRulesOnlyHint"),
    icon: Filter,
  },
  {
    value: "passthrough",
    label: t("settings.browserModePassthrough"),
    hint: t("settings.browserModePassthroughHint"),
    icon: ArrowRightFromLine,
  },
]);
</script>

<template>
  <SettingCard
    :title="t('settings.cardBrowserBehaviour')"
    :description="t('settings.cardBrowserBehaviourDesc')"
  >
    <div class="px-5 py-4">
      <TileChoice
        v-model="props.settings.bindings.mode.value"
        :options="modeOptions"
      />
    </div>
    <SettingRow
      id="browser/install-context-menu"
      :label="t('settings.browserContextMenu')"
      :description="t('settings.browserContextMenuDesc')"
    >
      <ToggleSwitch v-model="props.settings.bindings.installContextMenu.value" />
    </SettingRow>
    <SettingRow
      id="browser/hide-shelf"
      :label="t('settings.browserHideShelf')"
      :description="t('settings.browserHideShelfDesc')"
    >
      <ToggleSwitch v-model="props.settings.bindings.hideShelf.value" />
    </SettingRow>
    <SettingRow
      id="browser/forward-cookies"
      :label="t('settings.browserForwardCookies')"
      :description="t('settings.browserForwardCookiesDesc')"
    >
      <ToggleSwitch v-model="props.settings.bindings.forwardCookies.value" />
    </SettingRow>
  </SettingCard>
</template>
