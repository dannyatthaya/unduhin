import { computed, ref, watch, onScopeDispose } from "vue";

import {
  api,
  type CoreEvent,
  type Tool,
  type ToolStatus,
  onCoreEvent,
} from "@/types/tauri-bindings";

interface InstallState {
  active: boolean;
  downloaded: number;
  total: number | null;
  error: string | null;
}

/**
 * Tracks installation status for yt-dlp and ffmpeg. Polls on mount, then
 * keeps itself fresh by listening for tool_install_* events. Reuse this
 * composable from any view that needs to know whether the user can hit
 * the media-URL flow.
 */
export function useToolingStatus() {
  const ytdlp = ref<ToolStatus | null>(null);
  const ffmpeg = ref<ToolStatus | null>(null);
  const installing = ref<Record<Tool, InstallState>>({
    yt_dlp: { active: false, downloaded: 0, total: null, error: null },
    ffmpeg: { active: false, downloaded: 0, total: null, error: null },
  });

  let unlisten: (() => void) | null = null;

  async function refresh() {
    const [yt, ff] = await Promise.all([
      api.toolStatus("yt_dlp"),
      api.toolStatus("ffmpeg"),
    ]);
    ytdlp.value = yt;
    ffmpeg.value = ff;
  }

  async function install(tool: Tool) {
    installing.value[tool] = {
      active: true,
      downloaded: 0,
      total: null,
      error: null,
    };
    try {
      const result = await api.installTool(tool);
      if (tool === "yt_dlp") ytdlp.value = result;
      else ffmpeg.value = result;
    } catch (e: unknown) {
      installing.value[tool] = {
        ...installing.value[tool],
        active: false,
        error: (e as { message?: string })?.message ?? "Install failed.",
      };
      throw e;
    } finally {
      installing.value[tool] = { ...installing.value[tool], active: false };
    }
  }

  function handle(event: CoreEvent) {
    switch (event.type) {
      case "tool_install_progress":
        installing.value[event.tool] = {
          active: true,
          downloaded: event.downloaded,
          total: event.total,
          error: null,
        };
        break;
      case "tool_install_completed":
        installing.value[event.tool] = {
          active: false,
          downloaded: 0,
          total: null,
          error: null,
        };
        // Status will be re-fetched by `install()`'s resolve, but a
        // refresh here covers PATH-based detections that landed mid-install.
        void refresh();
        break;
      case "tool_install_failed":
        installing.value[event.tool] = {
          active: false,
          downloaded: 0,
          total: null,
          error: event.error,
        };
        break;
    }
  }

  void (async () => {
    unlisten = await onCoreEvent(handle);
  })();
  void refresh();

  onScopeDispose(() => {
    if (unlisten) unlisten();
  });

  return {
    ytdlp,
    ffmpeg,
    installing,
    refresh,
    install,
    ytdlpAvailable: computed(() => ytdlp.value?.installed === true),
    ffmpegAvailable: computed(() => ffmpeg.value?.installed === true),
  };
}

/** Watch a single tool's `active` flag — used by SettingsMedia for the
 * "Installing…" label. */
export function watchInstallingActive(
  installing: ReturnType<typeof useToolingStatus>["installing"],
  tool: Tool,
  fn: (active: boolean) => void,
): void {
  watch(
    () => installing.value[tool].active,
    (v) => fn(v),
    { immediate: true },
  );
}
