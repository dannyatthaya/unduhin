<script setup lang="ts">
import { useI18n } from "vue-i18n";

import SettingsSection from "@/components/settings/SettingsSection.vue";
import SettingCard from "@/components/settings/SettingCard.vue";
import SettingRow from "@/components/settings/SettingRow.vue";
import NumberStepper from "@/components/settings/controls/NumberStepper.vue";
import ToggleSwitch from "@/components/settings/controls/ToggleSwitch.vue";
import FolderPicker from "@/components/settings/controls/FolderPicker.vue";
import SliderField from "@/components/settings/controls/SliderField.vue";

import { useTorrentSettings } from "@/composables/useTorrentSettings";
import { useSettingsFilter } from "@/composables/useSettingsFilter";

const { t } = useI18n();
const s = useTorrentSettings();
const filter = useSettingsFilter();
const isHidden = (id: string) => filter.isHidden(id);

/** Render `torrent_seed_ratio_milli` (thousandths) as a friendly ratio:
 *  0 = "off" (stop at 100%, no seeding), otherwise e.g. 1500 → "1.5x". */
function formatSeedRatio(milli: number): string {
  if (milli <= 0) return t("settings.torrentSeedRatioOff");
  return `${(milli / 1000).toFixed(1)}x`;
}

/** 0 = OS-assigned random port; surface that rather than a bare "0". */
function formatPort(port: number): string {
  return port <= 0 ? t("settings.torrentListenPortAuto") : String(port);
}
</script>

<template>
  <SettingsSection
    :eyebrow="t('downloads.settings')"
    :title="t('settings.sectionTorrent')"
    :description="t('settings.sectionTorrentDesc')"
  >
    <SettingCard
      :title="t('settings.cardTorrentNetwork')"
      :description="t('settings.cardTorrentNetworkDesc')"
    >
      <SettingRow
        id="torrent/listen-port"
        :label="t('settings.torrentListenPort')"
        :description="t('settings.torrentListenPortDesc', { auto: formatPort(s.listenPort.value) })"
        :hidden="isHidden('torrent/listen-port')"
      >
        <NumberStepper
          v-model="s.listenPort.value"
          :min="0"
          :max="65535"
        />
      </SettingRow>
      <SettingRow
        id="torrent/enable-dht"
        :label="t('settings.torrentEnableDht')"
        :description="t('settings.torrentEnableDhtDesc')"
        :hidden="isHidden('torrent/enable-dht')"
      >
        <ToggleSwitch
          v-model="s.enableDht.value"
          :aria-label="t('settings.torrentEnableDht')"
        />
      </SettingRow>
      <SettingRow
        id="torrent/enable-upnp"
        :label="t('settings.torrentEnableUpnp')"
        :description="t('settings.torrentEnableUpnpDesc')"
        :hidden="isHidden('torrent/enable-upnp')"
      >
        <ToggleSwitch
          v-model="s.enableUpnp.value"
          :aria-label="t('settings.torrentEnableUpnp')"
        />
      </SettingRow>
      <SettingRow
        id="torrent/max-peers"
        :label="t('settings.torrentMaxPeers')"
        :description="t('settings.torrentMaxPeersDesc')"
        :hidden="isHidden('torrent/max-peers')"
      >
        <NumberStepper
          v-model="s.maxPeers.value"
          :min="1"
          :max="2000"
          :step="10"
        />
      </SettingRow>
    </SettingCard>

    <SettingCard
      :title="t('settings.cardTorrentStorage')"
      :description="t('settings.cardTorrentStorageDesc')"
    >
      <SettingRow
        id="torrent/download-dir"
        :label="t('settings.torrentDownloadDir')"
        :description="t('settings.torrentDownloadDirDesc')"
        :hidden="isHidden('torrent/download-dir')"
      >
        <FolderPicker
          v-model="s.downloadDir.value"
          :placeholder="t('settings.torrentDownloadDirPlaceholder')"
        />
      </SettingRow>
    </SettingCard>

    <SettingCard
      :title="t('settings.cardTorrentSeeding')"
      :description="t('settings.cardTorrentSeedingDesc')"
    >
      <SettingRow
        id="torrent/seed-ratio"
        :label="t('settings.torrentSeedRatio')"
        :description="t('settings.torrentSeedRatioDesc')"
        :hidden="isHidden('torrent/seed-ratio')"
      >
        <SliderField
          v-model="s.seedRatioMilli.value"
          :min="0"
          :max="5000"
          :step="250"
          :format="formatSeedRatio"
        />
      </SettingRow>
    </SettingCard>

    <p class="px-1 pt-2 text-xs text-muted-foreground">
      {{ t("common.saveHint") }}
    </p>
  </SettingsSection>
</template>
