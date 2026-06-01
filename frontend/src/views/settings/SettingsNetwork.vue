<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { RotateCcw } from "lucide-vue-next";

import SettingsSection from "@/components/settings/SettingsSection.vue";
import SettingCard from "@/components/settings/SettingCard.vue";
import SettingRow from "@/components/settings/SettingRow.vue";
import NumberStepper from "@/components/settings/controls/NumberStepper.vue";
import SliderField from "@/components/settings/controls/SliderField.vue";
import Button from "@/components/ui/Button.vue";

import { useNetworkSettings } from "@/composables/useNetworkSettings";
import { useSettingsFilter } from "@/composables/useSettingsFilter";

const { t } = useI18n();
const s = useNetworkSettings();
const filter = useSettingsFilter();
const isHidden = (id: string) => filter.isHidden(id);

interface UAPreset {
  label: string;
  value: string;
}

const PRESETS: UAPreset[] = [
  { label: "Unduhin/0.3", value: "" },
  {
    label: "Chrome · Windows",
    value:
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
  },
  {
    label: "Firefox · Windows",
    value:
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:124.0) Gecko/20100101 Firefox/124.0",
  },
  {
    label: "Edge · Windows",
    value:
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36 Edg/124.0.0.0",
  },
  { label: "curl/8.6", value: "curl/8.6.0" },
  { label: "wget/1.21", value: "Wget/1.21.4" },
];

function applyPreset(preset: UAPreset) {
  s.userAgent.value = preset.value;
}

function resetUA() {
  s.userAgent.value = "";
}

function formatBackoff(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`;
  return `${ms}ms`;
}
</script>

<template>
  <SettingsSection
    :eyebrow="t('downloads.settings')"
    :title="t('settings.sectionNetwork')"
    :description="t('settings.sectionNetworkDesc')"
  >
    <SettingCard
      :title="t('settings.cardTimeouts')"
      :description="t('settings.cardTimeoutsDesc')"
    >
      <SettingRow
        id="network/connect-timeout"
        :label="t('settings.connectTimeout')"
        :description="t('settings.connectTimeoutDesc')"
        :hidden="isHidden('network/connect-timeout')"
      >
        <NumberStepper
          v-model="s.connectTimeoutSecs.value"
          :min="1"
          :max="600"
          :suffix="t('settings.suffixSeconds')"
        />
      </SettingRow>
      <SettingRow
        id="network/read-timeout"
        :label="t('settings.readTimeout')"
        :description="t('settings.readTimeoutDesc')"
        :hidden="isHidden('network/read-timeout')"
      >
        <NumberStepper
          v-model="s.readTimeoutSecs.value"
          :min="1"
          :max="600"
          :suffix="t('settings.suffixSeconds')"
        />
      </SettingRow>
      <SettingRow
        id="network/max-retries"
        :label="t('settings.maxRetries')"
        :description="t('settings.maxRetriesDesc')"
        :hidden="isHidden('network/max-retries')"
      >
        <NumberStepper v-model="s.maxRetries.value" :min="1" :max="20" />
      </SettingRow>
      <SettingRow
        id="network/retry-backoff"
        :label="t('settings.retryBackoff')"
        :description="t('settings.retryBackoffDesc')"
        :hidden="isHidden('network/retry-backoff')"
      >
        <SliderField
          v-model="s.retryBackoffBaseMs.value"
          :min="100"
          :max="10000"
          :step="100"
          :format="formatBackoff"
        />
      </SettingRow>
    </SettingCard>

    <SettingCard
      :title="t('settings.cardUserAgent')"
      :description="t('settings.cardUserAgentDesc')"
    >
      <div
        :data-setting-id="'network/user-agent'"
        v-show="!isHidden('network/user-agent')"
        class="flex flex-col gap-3 px-5 py-4"
      >
        <textarea
          v-model="s.userAgent.value"
          rows="2"
          :placeholder="t('settings.userAgentPlaceholder')"
          class="w-full resize-none rounded-md border border-input bg-background px-3 py-2 font-mono text-xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        />
        <div class="flex flex-wrap items-center gap-2">
          <button
            v-for="preset in PRESETS"
            :key="preset.label"
            type="button"
            class="rounded border border-border px-2 py-1 text-xs transition-colors hover:bg-accent"
            :class="
              s.userAgent.value === preset.value
                ? 'border-primary bg-primary/5 text-primary'
                : 'text-foreground'
            "
            @click="applyPreset(preset)"
          >
            {{ preset.label }}
          </button>
          <Button
            v-if="s.userAgent.value !== ''"
            size="sm"
            variant="ghost"
            class="ml-auto"
            @click="resetUA"
          >
            <RotateCcw class="h-3.5 w-3.5" />
            {{ t("settings.resetToDefault") }}
          </Button>
        </div>
      </div>
    </SettingCard>

    <p class="px-1 pt-2 text-xs text-muted-foreground">
      {{ t("common.saveHint") }}
    </p>
  </SettingsSection>
</template>
