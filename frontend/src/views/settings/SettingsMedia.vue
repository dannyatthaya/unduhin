<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Film, Music2, Download, CheckCircle2, XCircle, Loader2 } from "lucide-vue-next";

import SettingsSection from "@/components/settings/SettingsSection.vue";
import SettingCard from "@/components/settings/SettingCard.vue";
import SettingRow from "@/components/settings/SettingRow.vue";
import SliderField from "@/components/settings/controls/SliderField.vue";
import Button from "@/components/ui/Button.vue";
import Input from "@/components/ui/Input.vue";

import { useMediaSettings } from "@/composables/useMediaSettings";
import { useSettingsFilter } from "@/composables/useSettingsFilter";
import { useToolingStatus } from "@/composables/useToolingStatus";
import type { Tool } from "@/types/tauri-bindings";

const { t } = useI18n();
const s = useMediaSettings();
const filter = useSettingsFilter();
const isHidden = (id: string) => filter.isHidden(id);

const { ytdlp, ffmpeg, installing, install } = useToolingStatus();

function formatPercent(downloaded: number, total: number | null): string {
  if (total == null || total === 0) {
    const mb = downloaded / (1024 * 1024);
    return `${mb.toFixed(1)} MB`;
  }
  return `${Math.floor((downloaded / total) * 100)}%`;
}

async function handleInstall(tool: Tool) {
  try {
    await install(tool);
  } catch (e) {
    console.warn(`install ${tool} failed`, e);
  }
}

const ytdlpProgress = computed(() => installing.value.yt_dlp);
const ffmpegProgress = computed(() => installing.value.ffmpeg);

function formatTimeout(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`;
  return `${ms}ms`;
}
</script>

<template>
  <SettingsSection
    :eyebrow="t('downloads.settings')"
    :title="t('settings.sectionMedia')"
    :description="t('settings.sectionMediaDesc')"
  >
    <SettingCard
      :title="t('settings.cardYtdlp')"
      :description="t('settings.cardYtdlpDesc')"
    >
      <div
        :data-setting-id="'media/ytdlp-status'"
        v-show="!isHidden('media/ytdlp-status')"
        class="flex items-start gap-3 px-5 py-4"
      >
        <Film class="mt-0.5 h-8 w-8 shrink-0 text-muted-foreground" />
        <div class="flex-1">
          <div class="flex items-center gap-2 text-sm font-medium">
            <span>{{ t("settings.ytdlpStatus") }}</span>
            <span v-if="ytdlp?.installed" class="inline-flex items-center gap-1 text-success">
              <CheckCircle2 class="h-3.5 w-3.5" /> {{ t("settings.ytdlpInstalled") }}
            </span>
            <span v-else class="inline-flex items-center gap-1 text-muted-foreground">
              <XCircle class="h-3.5 w-3.5" /> {{ t("settings.ytdlpNotInstalled") }}
            </span>
          </div>
          <p class="text-xs text-muted-foreground">
            <template v-if="ytdlp?.version">
              {{ t("settings.ytdlpVersion") }}: <span class="font-mono">{{ ytdlp.version }}</span>
            </template>
            <template v-else-if="ytdlp?.latest_known">
              {{ t("settings.latestPinned") }}: <span class="font-mono">{{ ytdlp.latest_known }}</span>
            </template>
          </p>
          <p v-if="ytdlp?.path" class="mt-1 truncate text-[11px] font-mono text-muted-foreground">
            {{ ytdlp.path }}
          </p>
          <div
            v-if="ytdlpProgress.active"
            class="mt-2 flex items-center gap-2 text-xs text-muted-foreground"
          >
            <Loader2 class="h-3.5 w-3.5 animate-spin" />
            <span>{{ t("settings.installingProgress", { progress: formatPercent(ytdlpProgress.downloaded, ytdlpProgress.total) }) }}</span>
          </div>
          <p v-if="ytdlpProgress.error" class="mt-2 text-xs text-danger">
            {{ ytdlpProgress.error }}
          </p>
        </div>
        <Button
          variant="primary"
          size="sm"
          :disabled="ytdlpProgress.active"
          @click="handleInstall('yt_dlp')"
        >
          <Download class="h-3.5 w-3.5" />
          {{ ytdlp?.installed ? t("settings.update") : t("settings.install") }}
        </Button>
      </div>

      <SettingRow
        id="media/ytdlp-custom-path"
        :label="t('settings.ytdlpBinaryPath')"
        :description="t('settings.ytdlpBinaryPathDesc')"
        :hidden="isHidden('media/ytdlp-custom-path')"
      >
        <Input
          v-model="s.ytdlpBinaryPath.value"
          placeholder="C:\\path\\to\\yt-dlp.exe"
          class="w-72"
        />
      </SettingRow>

      <SettingRow
        id="media/probe-timeout"
        :label="t('settings.ytdlpProbeTimeout')"
        :description="t('settings.ytdlpProbeTimeoutDesc')"
        :hidden="isHidden('media/probe-timeout')"
      >
        <SliderField
          v-model="s.probeTimeoutMs.value"
          :min="500"
          :max="10000"
          :step="100"
          :format="formatTimeout"
        />
      </SettingRow>

      <SettingRow
        id="media/default-format"
        :label="t('settings.ytdlpDefaultFormat')"
        :description="t('settings.ytdlpDefaultFormatDesc')"
        :hidden="isHidden('media/default-format')"
      >
        <Input v-model="s.defaultFormat.value" placeholder="bv*+ba/b" class="w-48" />
      </SettingRow>
    </SettingCard>

    <SettingCard
      :title="t('settings.cardFfmpeg')"
      :description="t('settings.cardFfmpegDesc')"
    >
      <div
        :data-setting-id="'media/ffmpeg-status'"
        v-show="!isHidden('media/ffmpeg-status')"
        class="flex items-start gap-3 px-5 py-4"
      >
        <Music2 class="mt-0.5 h-8 w-8 shrink-0 text-muted-foreground" />
        <div class="flex-1">
          <div class="flex items-center gap-2 text-sm font-medium">
            <span>{{ t("settings.ffmpegStatus") }}</span>
            <span v-if="ffmpeg?.installed" class="inline-flex items-center gap-1 text-success">
              <CheckCircle2 class="h-3.5 w-3.5" /> {{ t("settings.ffmpegInstalled") }}
            </span>
            <span v-else class="inline-flex items-center gap-1 text-muted-foreground">
              <XCircle class="h-3.5 w-3.5" /> {{ t("settings.ffmpegNotInstalled") }}
            </span>
          </div>
          <p class="text-xs text-muted-foreground">
            <template v-if="ffmpeg?.version">
              {{ t("settings.ytdlpVersion") }}: <span class="font-mono">{{ ffmpeg.version }}</span>
            </template>
            <template v-else-if="ffmpeg?.latest_known">
              {{ t("settings.latestPinned") }}: <span class="font-mono">{{ ffmpeg.latest_known }}</span>
            </template>
          </p>
          <p v-if="ffmpeg?.path" class="mt-1 truncate text-[11px] font-mono text-muted-foreground">
            {{ ffmpeg.path }}
          </p>
          <div
            v-if="ffmpegProgress.active"
            class="mt-2 flex items-center gap-2 text-xs text-muted-foreground"
          >
            <Loader2 class="h-3.5 w-3.5 animate-spin" />
            <span>{{ t("settings.installingProgress", { progress: formatPercent(ffmpegProgress.downloaded, ffmpegProgress.total) }) }}</span>
          </div>
          <p v-if="ffmpegProgress.error" class="mt-2 text-xs text-danger">
            {{ ffmpegProgress.error }}
          </p>
        </div>
        <Button
          variant="primary"
          size="sm"
          :disabled="ffmpegProgress.active"
          @click="handleInstall('ffmpeg')"
        >
          <Download class="h-3.5 w-3.5" />
          {{ ffmpeg?.installed ? t("settings.update") : t("settings.install") }}
        </Button>
      </div>

      <SettingRow
        id="media/ffmpeg-custom-path"
        :label="t('settings.ffmpegBinaryPath')"
        :description="t('settings.ffmpegBinaryPathDesc')"
        :hidden="isHidden('media/ffmpeg-custom-path')"
      >
        <Input
          v-model="s.ffmpegBinaryPath.value"
          placeholder="C:\\path\\to\\ffmpeg.exe"
          class="w-72"
        />
      </SettingRow>
    </SettingCard>

    <p class="px-1 pt-2 text-xs text-muted-foreground">
      {{ t("settings.mediaPrivacyFooter") }}
    </p>
  </SettingsSection>
</template>
