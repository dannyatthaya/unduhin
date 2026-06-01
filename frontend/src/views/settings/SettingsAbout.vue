<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import {
  BookOpen,
  Bug,
  CheckCircle2,
  ClipboardCopy,
  Copy,
  Download,
  ExternalLink,
  FileText,
  Github,
  Loader2,
  MessagesSquare,
  RefreshCw,
} from "lucide-vue-next";
import { openUrl } from "@tauri-apps/plugin-opener";

import SettingsSection from "@/components/settings/SettingsSection.vue";
import SettingCard from "@/components/settings/SettingCard.vue";
import SettingRow from "@/components/settings/SettingRow.vue";
import Button from "@/components/ui/Button.vue";
import Switch from "@/components/ui/Switch.vue";
import Select from "@/components/ui/Select.vue";

import { useSystemStore } from "@/stores/system";
import { useAboutPage } from "@/composables/useAboutPage";
import { useToast } from "@/composables/useToast";
import licenceManifestRaw from "@/generated/licences.json";

import icon from "@/assets/icon.png"

interface LicenceEntry {
  name: string;
  version: string;
  license: string;
  repository: string | null;
}

interface LicenceManifest {
  generated_at: string;
  rust: LicenceEntry[];
  node: LicenceEntry[];
}

const licenceManifest = licenceManifestRaw as LicenceManifest;

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const system = useSystemStore();
const about = useAboutPage();
const toast = useToast();

onMounted(() => {
  // Arrived from the tray's "Check for updates…" item — run the check and
  // strip the flag so a manual reload doesn't re-trigger it.
  if (route.query.check === "1") {
    void about.checkForUpdates();
    void router.replace({ name: "settings-about", query: {} });
  }
});

const appName = computed(() => system.appInfo?.name ?? "Unduhin");
const version = computed(() => system.appInfo?.version ?? "—");
const buildTimestamp = computed(() => system.appInfo?.build_timestamp ?? "—");
const commit = computed(() => system.appInfo?.git_sha ?? "—");
const channel = computed(() => about.channel.value);
const os = computed(() => system.appInfo?.os ?? "—");

const channelOptions = computed(() => [
  { value: "stable", label: t("settings.aboutChannelStable") },
  { value: "beta", label: t("settings.aboutChannelBeta") },
]);

const buildLabel = computed(() => formatBuildLabel(buildTimestamp.value));
const lastCheckedLabel = computed(() => relativeTime(about.lastCheckedAt.value));

const statusIconBgClass = computed(() => {
  if (about.phase.value === "error" || about.lastResult.value === "error")
    return "bg-danger/10 text-danger";
  if (about.available.value || about.lastResult.value === "update_available")
    return "bg-primary/10 text-primary";
  return "bg-success/10 text-success";
});

const statusHeadline = computed(() => {
  if (about.phase.value === "checking") return t("settings.aboutUpdatesChecking");
  if (about.phase.value === "downloading")
    return t("settings.aboutUpdatesDownloading", { bytes: formatBytes(about.downloadedBytes.value) });
  if (about.phase.value === "ready") return t("settings.aboutUpdatesInstalled");
  if (about.phase.value === "error") return t("settings.aboutUpdatesError");
  if (about.available.value)
    return t("settings.aboutUpdatesAvailable", { version: about.available.value.version });
  return t("settings.aboutUpdatesLatest");
});

const statusSubline = computed(() => {
  const status =
    about.lastResult.value === "error"
      ? t("settings.aboutSublineError")
      : t("settings.aboutSublineUpToDate");
  const time = lastCheckedLabel.value || t("settings.aboutSublineNeverChecked");
  const auto = about.checkOnStartup.value
    ? t("settings.aboutAutoOn")
    : t("settings.aboutAutoOff");
  return t("settings.aboutUpdatesSubline", { status, time, auto });
});

async function onCheckForUpdates() {
  await about.checkForUpdates();
  if (about.phase.value === "error" && about.phaseError.value) {
    toast.push(t("notify.updateCheckFailed", { error: about.phaseError.value }), "error");
  } else if (!about.available.value) {
    toast.push(t("notify.updateLatest"), "success");
  }
}

async function onInstall() {
  await about.installAvailable();
  if (about.phase.value === "error" && about.phaseError.value) {
    toast.push(t("notify.installFailed", { error: about.phaseError.value }), "error");
  }
}

async function copyText(text: string, label: string) {
  await navigator.clipboard.writeText(text);
  toast.push(t("notify.copySuccess", { label }), "success");
}

async function copyDiagnostic() {
  await copyText(about.copyDiagnostic(), t("settings.aboutCopyDiagnostic"));
}

async function open(href: string) {
  try {
    await openUrl(href);
  } catch {
    /* swallow — opener occasionally fails on dev-bin builds */
  }
}

// ---- Licence table ---------------------------------------------------------

const showAllLicences = ref(false);
const allLicences = computed<LicenceEntry[]>(() => [
  ...(licenceManifest.node ?? []),
  ...(licenceManifest.rust ?? []),
]);
const visibleLicences = computed(() =>
  showAllLicences.value ? allLicences.value : allLicences.value.slice(0, 6),
);

async function exportNotice() {
  const lines = [
    `${appName.value} ${version.value} — open-source notices`,
    `Generated ${new Date().toISOString()}`,
    "",
    "Frontend / JS dependencies:",
    ...(licenceManifest.node ?? []).map(
      (e) => `  ${e.name} ${e.version} — ${e.license}`,
    ),
    "",
    "Backend / Rust dependencies:",
    ...(licenceManifest.rust ?? []).map(
      (e) => `  ${e.name} ${e.version} — ${e.license}`,
    ),
    "",
  ];
  await copyText(lines.join("\n"), "NOTICE.txt");
}

// ---- Helpers ---------------------------------------------------------------

function formatBuildLabel(ts: string): string {
  // "2026-05-14 10:42 UTC" -> "2026.05.14·1042"
  const m = ts.match(/^(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2})/);
  if (!m) return ts;
  return `${m[1]}.${m[2]}.${m[3]}·${m[4]}${m[5]}`;
}

function relativeTime(iso: string): string {
  if (!iso) return "";
  const ts = Date.parse(iso);
  if (Number.isNaN(ts)) return "";
  const diffMs = Date.now() - ts;
  if (diffMs < 60_000) return t("settings.aboutJustNow");
  const mins = Math.floor(diffMs / 60_000);
  if (mins < 60) return t("settings.aboutMinsAgo", { n: mins });
  const hours = Math.floor(mins / 60);
  if (hours < 24) return t("settings.aboutHoursAgo", { n: hours });
  const days = Math.floor(hours / 24);
  return t("settings.aboutDaysAgo", { n: days });
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}
</script>

<template>
  <SettingsSection
    :eyebrow="t('downloads.settings')"
    :title="t('settings.sectionAbout')"
    :description="t('settings.sectionAboutDesc')"
  >
    <!-- Hero card -->
    <article
      class="relative overflow-hidden rounded-lg border border-border bg-linear-to-br from-card to-primary/5 px-6 py-6"
    >
      <div class="flex items-start gap-6">
        <div class="relative">
          <div
            class="flex h-20 w-20 items-center justify-center rounded-2xl bg-primary text-primary-foreground shadow-sm"
            aria-hidden="true"
          >
            <img :src="icon" />
          </div>

          <span
            v-if="version.includes('beta') || channel === 'beta'"
            class="absolute -bottom-1 -right-1 rounded bg-foreground px-1.5 py-0.5 font-mono text-[9px] font-semibold uppercase tracking-wider text-background"
          >
            {{ t("settings.aboutBeta") }}
          </span>
        </div>

        <div class="flex flex-1 flex-col gap-3 min-w-0">
          <div class="flex items-start justify-between gap-3">
            <div class="flex flex-col gap-1">
              <h2 class="text-2xl font-bold leading-tight">{{ appName }}</h2>
              <p class="max-w-md text-sm text-muted-foreground">
                {{ t("settings.aboutDescription") }}
              </p>
            </div>

            <div class="relative">
              <Button
                variant="secondary"
                size="sm"
                @click="copyDiagnostic"
              >
                <ClipboardCopy class="h-3.5 w-3.5" />
                {{ t("settings.aboutCopyDiagnostic") }}
              </Button>

              <div
                class="absolute -left-60 top-1/2 -translate-y-1/2 font-mono text-[11px] text-muted-foreground"
                :aria-label="t('settings.aboutLabelHostOs')"
              >
                {{ os }}
              </div>
            </div>
          </div>

          <dl
            class="grid grid-cols-[auto_auto_auto_auto] items-start gap-x-8 gap-y-1 pt-2 text-xs"
            id="about/hero"
          >
            <div class="flex flex-col gap-0.5">
              <dt class="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
                {{ t("settings.aboutLabelVersion") }}
              </dt>
              <dd class="font-mono text-sm">{{ version }}</dd>
            </div>
            <div class="flex flex-col gap-0.5">
              <dt class="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
                {{ t("settings.aboutLabelBuild") }}
              </dt>
              <dd class="font-mono text-sm">{{ buildLabel }}</dd>
            </div>
            <div class="flex flex-col gap-0.5">
              <dt class="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
                {{ t("settings.aboutLabelCommit") }}
              </dt>
              <dd class="flex items-center gap-1.5 font-mono text-sm">
                {{ commit }}
                <button
                  type="button"
                  class="text-muted-foreground transition-colors hover:text-foreground"
                  :title="t('settings.aboutCopyCommitHash')"
                  @click="copyText(commit, t('settings.aboutLabelCommit'))"
                >
                  <Copy class="h-3 w-3" />
                </button>
              </dd>
            </div>
            <div class="flex flex-col gap-0.5">
              <dt class="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
                {{ t("settings.aboutLabelChannel") }}
              </dt>
              <dd>
                <span
                  class="inline-flex items-center gap-1.5 rounded border border-success/40 bg-success/10 px-2 py-0.5 font-mono text-xs"
                >
                  <span class="h-1.5 w-1.5 rounded-full bg-success"></span>
                  {{ channel }}
                </span>
              </dd>
            </div>
          </dl>
        </div>
      </div>
    </article>

    <!-- Update status card -->
    <article class="overflow-hidden rounded-lg border border-border bg-card">
      <div class="flex items-center gap-5 px-5 py-5">
        <div :class="[
          'flex h-11 w-11 shrink-0 items-center justify-center rounded-full',
          statusIconBgClass,
        ]">
          <Loader2
            v-if="about.phase.value === 'checking' || about.phase.value === 'downloading'"
            class="h-5 w-5 animate-spin"
          />
          <CheckCircle2
            v-else-if="!about.available.value"
            class="h-5 w-5"
          />
          <Download
            v-else
            class="h-5 w-5"
          />
        </div>
        <div class="flex flex-1 flex-col gap-0.5 min-w-0">
          <p class="text-sm font-semibold">{{ statusHeadline }}</p>
          <p class="truncate font-mono text-xs text-muted-foreground">
            <span class="text-success">●</span>
            {{ statusSubline }}
          </p>
        </div>
        <div class="flex shrink-0 items-center gap-2">
          <Button
            v-if="about.available.value"
            variant="primary"
            size="sm"
            :disabled="about.phase.value === 'downloading' || about.phase.value === 'ready'"
            @click="onInstall"
          >
            <Download class="h-3.5 w-3.5" />
            {{ t("settings.aboutInstall") }}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            :disabled="about.phase.value === 'checking'"
            @click="onCheckForUpdates"
          >
            <RefreshCw class="h-3.5 w-3.5" />
            {{ t("settings.aboutCheckForUpdates") }}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            @click="open('https://github.com/dannyatthaya/unduhin/releases')"
          >
            <FileText class="h-3.5 w-3.5" />
            {{ t("settings.aboutReleaseNotes") }}
          </Button>
        </div>
      </div>
    </article>

    <!-- Updates & privacy -->
    <SettingCard
      :title="t('settings.aboutCardUpdates')"
      :description="t('settings.aboutCardUpdatesDesc')"
    >
      <SettingRow
        id="about/update-channel"
        :label="t('settings.aboutUpdateChannel')"
        :description="t('settings.aboutUpdateChannelDesc')"
      >
        <div class="w-40">
          <Select
            :model-value="channel"
            :options="channelOptions"
            @update:model-value="about.setChannel($event as 'stable' | 'beta')"
          />
        </div>
      </SettingRow>
      <p
        v-if="channel === 'beta'"
        class="px-5 pb-4 pt-0 text-xs text-muted-foreground"
      >
        {{ t("settings.aboutBetaHintPrefix") }}
        <button
          type="button"
          class="text-primary underline-offset-2 hover:underline"
          @click="open('https://github.com/dannyatthaya/unduhin/releases')"
        >
          {{ t("settings.aboutBetaLinkLabel") }}
        </button>
        {{ t("settings.aboutBetaHintSuffix") }}
      </p>
      <SettingRow
        id="about/update-check-on-startup"
        :label="t('settings.aboutCheckOnStartup')"
        :description="t('settings.aboutCheckOnStartupDesc')"
      >
        <Switch
          :model-value="about.checkOnStartup.value"
          @update:model-value="about.setBoolean('update_check_on_startup', $event)"
        />
      </SettingRow>
      <SettingRow
        id="about/send-crash-reports"
        :label="t('settings.aboutCrashReports')"
        :description="t('settings.aboutCrashReportsDesc')"
      >
        <Switch
          :model-value="about.crashReports.value"
          @update:model-value="about.setBoolean('send_crash_reports', $event)"
        />
      </SettingRow>
      <SettingRow
        id="about/send-usage-stats"
        :label="t('settings.aboutUsageStats')"
        :description="t('settings.aboutUsageStatsDesc')"
      >
        <Switch
          :model-value="about.usageStats.value"
          @update:model-value="about.setBoolean('send_usage_stats', $event)"
        />
      </SettingRow>
    </SettingCard>

    <!-- Links & resources -->
    <SettingCard
      :title="t('settings.aboutCardLinks')"
      :description="t('settings.aboutCardLinksDesc')"
    >
      <div class="grid grid-cols-2 gap-3 p-5">
        <button
          type="button"
          class="flex items-center justify-between gap-3 rounded-md border border-border bg-background px-4 py-3 text-left transition-colors hover:bg-accent"
          @click="open('https://github.com/dannyatthaya/unduhin#readme')"
        >
          <div class="flex items-center gap-3">
            <BookOpen class="h-4 w-4 text-muted-foreground" />
            <div class="flex flex-col">
              <span class="text-sm font-medium">{{ t("settings.aboutLinkDocs") }}</span>
              <span class="font-mono text-[11px] text-muted-foreground">github.com/.../unduhin</span>
            </div>
          </div>
          <ExternalLink class="h-3.5 w-3.5 text-muted-foreground" />
        </button>
        <button
          type="button"
          class="flex items-center justify-between gap-3 rounded-md border border-border bg-background px-4 py-3 text-left transition-colors hover:bg-accent"
          @click="open('https://github.com/dannyatthaya/unduhin')"
        >
          <div class="flex items-center gap-3">
            <Github class="h-4 w-4 text-muted-foreground" />
            <div class="flex flex-col">
              <span class="text-sm font-medium">{{ t("settings.aboutLinkGithub") }}</span>
              <span class="font-mono text-[11px] text-muted-foreground">
                github.com/dannyatthaya/unduhin
              </span>
            </div>
          </div>
          <ExternalLink class="h-3.5 w-3.5 text-muted-foreground" />
        </button>
        <button
          type="button"
          class="flex items-center justify-between gap-3 rounded-md border border-border bg-background px-4 py-3 text-left transition-colors hover:bg-accent"
          @click="open('https://github.com/dannyatthaya/unduhin/issues/new')"
        >
          <div class="flex items-center gap-3">
            <Bug class="h-4 w-4 text-muted-foreground" />
            <div class="flex flex-col">
              <span class="text-sm font-medium">{{ t("settings.aboutLinkBug") }}</span>
              <span class="font-mono text-[11px] text-muted-foreground">github.com/.../issues/new</span>
            </div>
          </div>
          <ExternalLink class="h-3.5 w-3.5 text-muted-foreground" />
        </button>
        <button
          type="button"
          class="flex items-center justify-between gap-3 rounded-md border border-border bg-background px-4 py-3 text-left transition-colors hover:bg-accent"
          @click="open('https://github.com/dannyatthaya/unduhin/discussions')"
        >
          <div class="flex items-center gap-3">
            <MessagesSquare class="h-4 w-4 text-muted-foreground" />
            <div class="flex flex-col">
              <span class="text-sm font-medium">{{ t("settings.aboutLinkForum") }}</span>
              <span class="font-mono text-[11px] text-muted-foreground">github.com/.../discussions</span>
            </div>
          </div>
          <ExternalLink class="h-3.5 w-3.5 text-muted-foreground" />
        </button>
      </div>
    </SettingCard>

    <!-- Open-source licences -->
    <SettingCard
      :title="t('settings.aboutCardLicences')"
      :description="t('settings.aboutLicencesDesc', { n: visibleLicences.length, m: allLicences.length })"
    >
      <template #actions>
        <Button
          size="sm"
          variant="secondary"
          @click="exportNotice"
        >
          <Download class="h-3.5 w-3.5" />
          {{ t("settings.aboutLicencesExport") }}
        </Button>
      </template>
      <div class="divide-y divide-border">
        <div
          v-for="entry in visibleLicences"
          :key="`${entry.name}@${entry.version}`"
          class="flex items-center justify-between gap-3 px-5 py-2.5"
        >
          <div class="flex items-baseline gap-2 min-w-0">
            <span class="font-mono text-sm font-medium">{{ entry.name }}</span>
            <span class="font-mono text-[11px] text-muted-foreground">{{ entry.version }}</span>
          </div>
          <div class="flex items-center gap-3 shrink-0">
            <span class="rounded bg-muted px-1.5 py-0.5 font-mono text-[10px]">
              {{ entry.license }}
            </span>
            <button
              v-if="entry.repository"
              type="button"
              class="text-muted-foreground transition-colors hover:text-foreground"
              :title="entry.repository ?? undefined"
              @click="open(entry.repository!)"
            >
              <ExternalLink class="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
        <button
          v-if="allLicences.length > 6"
          type="button"
          class="w-full px-5 py-2.5 text-center text-xs text-primary transition-colors hover:bg-accent"
          @click="showAllLicences = !showAllLicences"
        >
          {{ showAllLicences ? t("settings.aboutLicencesShowLess") : t("settings.aboutLicencesViewAll", { n: allLicences.length }) }}
        </button>
      </div>
    </SettingCard>

    <footer class="flex flex-col items-center gap-1 pb-2 pt-4 text-xs text-muted-foreground">
      <p class="flex items-center gap-1.5">
        {{ t("settings.aboutFooterCopyright") }}
      </p>
      <p class="font-mono text-[10px]">
        {{ t("settings.aboutFooterEtymology") }}
      </p>
    </footer>
  </SettingsSection>
</template>
