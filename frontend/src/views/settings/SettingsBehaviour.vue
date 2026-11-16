<script setup lang="ts">
// Settings → Behaviour: what the app does at sign-in, when the window is
// closed, and when it may interrupt you.

import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Bell } from "lucide-vue-next";
import { enable as enableAutostart, disable as disableAutostart } from "@tauri-apps/plugin-autostart";

import QuietHoursCard from "@/components/settings/QuietHoursCard.vue";
import SettingCard from "@/components/settings/SettingCard.vue";
import SettingRow from "@/components/settings/SettingRow.vue";
import SettingsSection from "@/components/settings/SettingsSection.vue";
import ToggleSwitch from "@/components/settings/controls/ToggleSwitch.vue";
import Button from "@/components/ui/Button.vue";
import SegmentedControl, { type SegmentOption } from "@/components/ui/SegmentedControl.vue";

import {
  useBehaviourSettings,
  type CloseBehavior,
} from "@/composables/useBehaviourSettings";
import { previewNotification } from "@/composables/useNotificationPreview";
import { useToast } from "@/composables/useToast";

const { t } = useI18n();
const s = useBehaviourSettings();
const toast = useToast();

const closeOptions = computed<SegmentOption<CloseBehavior>[]>(() => [
  { value: "minimize", label: t("settings.closeMinimize") },
  { value: "exit", label: t("settings.closeExit") },
  { value: "ask", label: t("settings.closeAsk") },
]);

// Flipping the setting also (en|dis)ables the OS-level entry owned by the
// plugin. The startup reconcile in src-tauri/src/lib.rs keeps the two in
// sync on launch; this keeps them in sync at runtime.
async function setAutostart(on: boolean): Promise<void> {
  s.autostart.value = on;
  try {
    if (on) await enableAutostart();
    else await disableAutostart();
  } catch (e: unknown) {
    const message = (e as { message?: string })?.message ?? String(e);
    toast.push(t("settings.autostartFailed", { error: message }), "error");
  }
}

async function sendTest(): Promise<void> {
  const allowed = await previewNotification("complete");
  if (!allowed) toast.push(t("settings.notifyBlocked"), "error");
}

const notifications = computed(() => [
  {
    id: "behaviour/notify-complete",
    label: t("settings.notifyComplete"),
    description: t("settings.notifyCompleteDesc"),
    model: s.notifyComplete,
  },
  {
    id: "behaviour/notify-fail",
    label: t("settings.notifyFail"),
    description: t("settings.notifyFailDesc"),
    model: s.notifyFail,
  },
  {
    id: "behaviour/notify-queue-empty",
    label: t("settings.notifyQueueEmpty"),
    description: t("settings.notifyQueueEmptyDesc"),
    model: s.notifyQueueEmpty,
  },
]);
</script>

<template>
  <SettingsSection
    :title="t('settings.sectionBehaviour')"
    :description="t('settings.sectionBehaviourDesc')"
  >
    <SettingCard :title="t('settings.cardStartup')">
      <SettingRow
        id="behaviour/autostart"
        v-slot="{ labelId }"
        :label="t('settings.startupAutostart')"
        :description="t('settings.startupAutostartDesc')"
      >
        <ToggleSwitch
          :model-value="s.autostart.value"
          :labelledby="labelId"
          @update:model-value="setAutostart"
        />
      </SettingRow>
      <SettingRow
        id="behaviour/start-minimized"
        v-slot="{ labelId }"
        :label="t('settings.startupMinimized')"
        :description="t('settings.startupMinimizedDesc')"
      >
        <ToggleSwitch
          v-model="s.startMinimized.value"
          :labelledby="labelId"
          :disabled="!s.autostart.value"
        />
      </SettingRow>
      <SettingRow
        id="behaviour/close-behavior"
        v-slot="{ labelId }"
        :label="t('settings.closeBehaviour')"
        :description="t('settings.closeBehaviourDesc')"
      >
        <SegmentedControl
          v-model="s.closeBehavior.value"
          :options="closeOptions"
          :labelledby="labelId"
        />
      </SettingRow>
      <SettingRow
        id="behaviour/confirm-on-quit"
        v-slot="{ labelId }"
        :label="t('settings.confirmOnQuit')"
        :description="t('settings.confirmOnQuitDesc')"
      >
        <ToggleSwitch v-model="s.confirmOnQuit.value" :labelledby="labelId" />
      </SettingRow>
    </SettingCard>

    <SettingCard :title="t('settings.cardNotifications')" :note="t('settings.notifyPermissionNote')">
      <template #actions>
        <Button variant="ghost" size="sm" class="h-[30px] gap-1.5 px-2.5 text-dl-soft" @click="sendTest">
          <Bell class="h-3.5 w-3.5" aria-hidden="true" />
          {{ t("settings.notifyTest") }}
        </Button>
      </template>
      <SettingRow
        v-for="item in notifications"
        :id="item.id"
        :key="item.id"
        v-slot="{ labelId }"
        :label="item.label"
        :description="item.description"
      >
        <ToggleSwitch v-model="item.model.value" :labelledby="labelId" />
      </SettingRow>
    </SettingCard>

    <QuietHoursCard />
  </SettingsSection>
</template>
