<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { LayoutGrid, LogOut, HelpCircle, Moon } from "lucide-vue-next";
import { enable as enableAutostart, disable as disableAutostart } from "@tauri-apps/plugin-autostart";

import SettingsSection from "@/components/settings/SettingsSection.vue";
import SettingCard from "@/components/settings/SettingCard.vue";
import SettingRow from "@/components/settings/SettingRow.vue";
import ToggleSwitch from "@/components/settings/controls/ToggleSwitch.vue";
import TileChoice, {
  type TileOption,
} from "@/components/settings/controls/TileChoice.vue";
import Button from "@/components/ui/Button.vue";
import ScheduleDialog from "@/components/ScheduleDialog.vue";

import {
  useBehaviourSettings,
  type CloseBehavior,
} from "@/composables/useBehaviourSettings";
import {
  previewNotification,
  type NotificationKind,
} from "@/composables/useNotificationPreview";
import { useSettingsFilter } from "@/composables/useSettingsFilter";
import { useSchedulesStore } from "@/stores/schedules";

const { t } = useI18n();
const s = useBehaviourSettings();
const filter = useSettingsFilter();
const isHidden = (id: string) => filter.isHidden(id);

const schedules = useSchedulesStore();
const scheduleDialogOpen = ref(false);

onMounted(() => {
  // The settings page may be deep-linked; make sure the schedules cache
  // is populated when the user opens this card directly.
  if (schedules.list.length === 0) void schedules.refresh();
});

const dayNames = computed(() => [
  t("common.dayMon"),
  t("common.dayTue"),
  t("common.dayWed"),
  t("common.dayThu"),
  t("common.dayFri"),
  t("common.daySat"),
  t("common.daySun"),
]);

const quietSummary = computed(() => {
  const q = schedules.globalQuietHours;
  if (!q || !q.active) return t("common.off");
  const startTime = q.start_iso ?? "--:--";
  const endTime = q.end_iso ?? "--:--";
  const mask = q.days_mask;
  let days = "";
  if (mask === 127) days = t("common.everyDay");
  else if (mask === (1 << 5) + (1 << 6)) days = t("common.weekends");
  else if (mask === 0b0011111) days = t("common.weekdays");
  else {
    const enabled = dayNames.value.filter((_, i) => (mask & (1 << i)) !== 0);
    days = enabled.join(", ");
  }
  return t("downloads.scheduleSummaryRange", { start: startTime, end: endTime, days });
});

const closeOptions = computed<TileOption<CloseBehavior>[]>(() => [
  { value: "minimize", label: t("settings.closeMinimize"), hint: t("settings.closeMinimizeHint"), icon: LayoutGrid },
  { value: "exit", label: t("settings.closeExit"), hint: t("settings.closeExitHint"), icon: LogOut },
  { value: "ask", label: t("settings.closeAsk"), hint: t("settings.closeAskHint"), icon: HelpCircle },
]);

// Wrap the autostart toggle so flipping it also (en|dis)ables the OS-level
// state owned by the plugin. The startup reconcile in src-tauri/src/lib.rs
// keeps the two in sync on launch; this keeps them in sync at runtime.
const autostart = computed({
  get: () => s.autostart.value,
  set: async (v: boolean) => {
    s.autostart.value = v;
    try {
      if (v) await enableAutostart();
      else await disableAutostart();
    } catch (e) {
      console.warn("autostart toggle failed", e);
    }
  },
});

async function preview(kind: NotificationKind) {
  await previewNotification(kind);
}
</script>

<template>
  <SettingsSection
    :eyebrow="t('downloads.settings')"
    :title="t('settings.sectionBehaviour')"
    :description="t('settings.sectionBehaviourDesc')"
  >
    <SettingCard
      :title="t('settings.cardStartup')"
      :description="t('settings.cardStartupDesc')"
    >
      <SettingRow
        id="behaviour/autostart"
        :label="t('settings.startupAutostart')"
        :description="t('settings.startupAutostartDesc')"
        :hidden="isHidden('behaviour/autostart')"
      >
        <ToggleSwitch v-model="autostart" />
      </SettingRow>
      <SettingRow
        id="behaviour/start-minimized"
        :label="t('settings.startupMinimized')"
        :description="t('settings.startupMinimizedDesc')"
        :hidden="isHidden('behaviour/start-minimized')"
      >
        <ToggleSwitch
          v-model="s.startMinimized.value"
          :disabled="!autostart"
        />
      </SettingRow>
      <SettingRow
        id="behaviour/close-behavior"
        :label="t('settings.closeBehaviour')"
        :description="t('settings.closeBehaviourDesc')"
        :hidden="isHidden('behaviour/close-behavior')"
      >
        <TileChoice
          v-model="s.closeBehavior.value"
          :options="closeOptions"
        />
      </SettingRow>
      <SettingRow
        id="behaviour/confirm-on-quit"
        :label="t('settings.confirmOnQuit')"
        :description="t('settings.confirmOnQuitDesc')"
        :hidden="isHidden('behaviour/confirm-on-quit')"
      >
        <ToggleSwitch v-model="s.confirmOnQuit.value" />
      </SettingRow>
    </SettingCard>

    <SettingCard
      :title="t('settings.cardNotifications')"
      :description="t('settings.cardNotificationsDesc')"
    >
      <SettingRow
        id="behaviour/notify-complete"
        :label="t('settings.notifyComplete')"
        :description="t('settings.notifyCompleteDesc')"
        :hidden="isHidden('behaviour/notify-complete')"
      >
        <Button size="sm" variant="secondary" @click="preview('complete')">
          {{ t("settings.notifyPreview") }}
        </Button>
        <ToggleSwitch v-model="s.notifyComplete.value" />
      </SettingRow>
      <SettingRow
        id="behaviour/notify-fail"
        :label="t('settings.notifyFail')"
        :description="t('settings.notifyFailDesc')"
        :hidden="isHidden('behaviour/notify-fail')"
      >
        <Button size="sm" variant="secondary" @click="preview('fail')">
          {{ t("settings.notifyPreview") }}
        </Button>
        <ToggleSwitch v-model="s.notifyFail.value" />
      </SettingRow>
      <SettingRow
        id="behaviour/notify-queue-empty"
        :label="t('settings.notifyQueueEmpty')"
        :description="t('settings.notifyQueueEmptyDesc')"
        :hidden="isHidden('behaviour/notify-queue-empty')"
      >
        <Button size="sm" variant="secondary" @click="preview('queue-empty')">
          {{ t("settings.notifyPreview") }}
        </Button>
        <ToggleSwitch v-model="s.notifyQueueEmpty.value" />
      </SettingRow>
    </SettingCard>

    <SettingCard
      :title="t('settings.cardQuietHours')"
      :description="t('settings.cardQuietHoursDesc')"
    >
      <div
        :data-setting-id="'behaviour/quiet-hours'"
        class="flex items-center justify-between gap-4 px-5 py-4"
        :hidden="isHidden('behaviour/quiet-hours')"
      >
        <div class="flex items-center gap-3">
          <span class="flex h-9 w-9 items-center justify-center rounded-md bg-muted">
            <Moon class="h-4 w-4 text-muted-foreground" />
          </span>
          <div class="flex flex-col">
            <span class="text-sm font-medium">{{ t("settings.cardQuietHours") }}</span>
            <span class="text-xs text-muted-foreground">
              {{ quietSummary }}
            </span>
          </div>
        </div>
        <Button variant="secondary" size="sm" @click="scheduleDialogOpen = true">
          {{ t("settings.quietHoursEdit") }}
        </Button>
      </div>
    </SettingCard>

    <ScheduleDialog
      :open="scheduleDialogOpen"
      :scope="{ kind: 'global' }"
      @close="scheduleDialogOpen = false"
    />

    <p class="px-1 pt-2 text-xs text-muted-foreground">
      {{ t("common.saveHint") }}
    </p>
  </SettingsSection>
</template>
