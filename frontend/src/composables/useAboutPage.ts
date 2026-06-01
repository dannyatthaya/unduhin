// Drives the Settings → About page.
//
//  - reads version + git sha + build timestamp from the system store
//    (populated via the `app_info` Tauri command),
//  - exposes the persisted channel/telemetry toggles via the settings store,
//  - runs `check()` through the Tauri updater plugin and persists the
//    last-checked timestamp + result so a reload still shows context.
//
// The actual HTTPS fetch is done by the plugin — we only translate the
// boolean "available?" into our typed UpdateCheckStatus and persist it.

import { computed, ref } from "vue";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

import { api, type UpdateCheckStatus } from "@/types/tauri-bindings";
import { useSystemStore } from "@/stores/system";
import { useSettingsStore } from "@/stores/settings";

export type CheckPhase = "idle" | "checking" | "downloading" | "ready" | "error";

export function useAboutPage() {
  const system = useSystemStore();
  const settings = useSettingsStore();

  const phase = ref<CheckPhase>("idle");
  const phaseError = ref<string | null>(null);
  const available = ref<Update | null>(null);
  const downloadedBytes = ref(0);
  const downloadTotal = ref<number | null>(null);

  const channel = computed<string>(() =>
    typeof settings.values.update_channel === "string"
      ? settings.values.update_channel
      : "stable",
  );

  function settingBool(key: string, fallback: boolean): boolean {
    const v = settings.values[key];
    return typeof v === "boolean" ? v : fallback;
  }

  const lastCheckedAt = computed<string>(() =>
    typeof settings.values.last_update_check_at === "string"
      ? settings.values.last_update_check_at
      : "",
  );

  const lastResult = computed<UpdateCheckStatus | "">(() => {
    const v = settings.values.last_update_check_result;
    if (v === "up_to_date" || v === "update_available" || v === "error") return v;
    return "";
  });

  async function checkForUpdates() {
    phase.value = "checking";
    phaseError.value = null;
    available.value = null;
    try {
      // Tauri v2's `check()` uses the endpoint configured in
      // tauri.conf.json (`plugins.updater.endpoints`). The app ships a
      // single endpoint (latest-stable.json); beta-channel auto-checks
      // are deferred for now — until then, picking "beta" persists
      // the preference and the UI links to GitHub Releases for manual
      // installs (see SettingsAbout.vue for the inline note).
      const update = await check();
      if (update) {
        available.value = update;
        phase.value = "idle";
        await api.recordUpdateCheck(
          "update_available",
          update.version ?? null,
          update.body ?? null,
        );
        await settings.refresh();
      } else {
        phase.value = "idle";
        await api.recordUpdateCheck("up_to_date");
        await settings.refresh();
      }
    } catch (e) {
      phase.value = "error";
      phaseError.value = e instanceof Error ? e.message : String(e);
      await api.recordUpdateCheck("error");
      await settings.refresh();
    }
  }

  async function installAvailable() {
    const update = available.value;
    if (!update) return;
    phase.value = "downloading";
    phaseError.value = null;
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          downloadTotal.value = event.data.contentLength ?? null;
          downloadedBytes.value = 0;
        } else if (event.event === "Progress") {
          downloadedBytes.value += event.data.chunkLength;
        } else if (event.event === "Finished") {
          phase.value = "ready";
        }
      });
      await relaunch();
    } catch (e) {
      phase.value = "error";
      phaseError.value = e instanceof Error ? e.message : String(e);
    }
  }

  async function setChannel(next: "stable" | "beta") {
    await settings.set("update_channel", next);
    // The exposed `channel` already follows the settings store, but
    // app_info's channel is sampled at command time — refresh so the
    // top-of-page chip flips immediately too.
    await system.refresh();
  }

  async function setBoolean(key: string, value: boolean) {
    await settings.set(key, value);
  }

  function copyDiagnostic(): string {
    const info = system.appInfo;
    const lines = [
      `Unduhin ${info?.version ?? "—"}`,
      `Channel: ${info?.channel ?? channel.value}`,
      `Build:   ${info?.build_timestamp ?? "—"}`,
      `Commit:  ${info?.git_sha ?? "—"}`,
      `OS:      ${info?.os ?? "—"}`,
      `Last update check: ${lastCheckedAt.value || "never"} (${lastResult.value || "n/a"})`,
    ];
    return lines.join("\n");
  }

  return {
    // state
    phase,
    phaseError,
    available,
    downloadedBytes,
    downloadTotal,
    // derived
    channel,
    lastCheckedAt,
    lastResult,
    crashReports: computed(() => settingBool("send_crash_reports", false)),
    usageStats: computed(() => settingBool("send_usage_stats", false)),
    checkOnStartup: computed(() => settingBool("update_check_on_startup", true)),
    // actions
    checkForUpdates,
    installAvailable,
    setChannel,
    setBoolean,
    copyDiagnostic,
  };
}
