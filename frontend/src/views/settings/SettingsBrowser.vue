<script setup lang="ts">
import { useI18n } from "vue-i18n";

import { computed } from "vue";

import SettingsSection from "@/components/settings/SettingsSection.vue";
import BrowserStatusCard from "@/components/settings/browser/BrowserStatusCard.vue";
import BrowserExtensionsCard from "@/components/settings/browser/BrowserExtensionsCard.vue";
import BrowserBehaviourCard from "@/components/settings/browser/BrowserBehaviourCard.vue";
import BrowserFileTypesCard from "@/components/settings/browser/BrowserFileTypesCard.vue";
import BrowserMinSizeCard from "@/components/settings/browser/BrowserMinSizeCard.vue";
import BrowserDomainRulesCard from "@/components/settings/browser/BrowserDomainRulesCard.vue";
import BrowserClipboardCard from "@/components/settings/browser/BrowserClipboardCard.vue";

import { useBrowserStatus } from "@/composables/useBrowserStatus";
import { useBrowserSettings } from "@/composables/useBrowserSettings";
import { useToast } from "@/composables/useToast";

const { t } = useI18n();
const browser = useBrowserStatus();
const browserSettings = useBrowserSettings();
const { push } = useToast();

/** Pipe-down banner state — surfaces above the cards once the status
 *  call has resolved and explicitly returned `listening: false`. The
 *  first `loading` pass is suppressed so we don't flash a banner during
 *  the initial bind. */
const pipeDown = computed<boolean>(() => {
  if (browser.loading.value) return false;
  const status = browser.status.value;
  return status?.pipe ? !status.pipe.listening : false;
});

async function onTestHandoff() {
  try {
    const result = await browser.testHandoff();
    const ms = (result.round_trip_us / 1000).toFixed(2);
    push(t("settings.browserTestSuccess", { ms }), "success");
  } catch (e: unknown) {
    push(
      (e as { message?: string })?.message ??
        t("settings.browserTestFailed"),
      "error",
    );
  }
}
</script>

<template>
  <SettingsSection
    :eyebrow="t('downloads.settings')"
    :title="t('settings.sectionBrowser')"
    :description="t('settings.sectionBrowserDesc')"
  >
    <div
      v-if="pipeDown"
      role="alert"
      class="rounded-md border border-danger/40 bg-danger/10 px-4 py-2 text-xs text-danger"
    >
      {{ t("settings.browserPipeDownBanner") }}
    </div>
    <BrowserStatusCard
      :status="browser.status.value"
      :loading="browser.loading.value"
      :testing="browser.testing.value"
      @test-handoff="onTestHandoff"
    />
    <BrowserExtensionsCard
      :browsers="browser.status.value?.browsers ?? []"
      :loading="browser.loading.value"
      @rescan="() => browser.refresh()"
    />
    <BrowserBehaviourCard :settings="browserSettings" />
    <BrowserFileTypesCard :settings="browserSettings" />
    <BrowserMinSizeCard :settings="browserSettings" />
    <BrowserDomainRulesCard :settings="browserSettings" />
    <BrowserClipboardCard :settings="browserSettings" />
    <p class="px-1 pt-2 text-center text-xs text-muted-foreground">
      {{ t("settings.browserBridgeFooter") }}
    </p>
  </SettingsSection>
</template>
