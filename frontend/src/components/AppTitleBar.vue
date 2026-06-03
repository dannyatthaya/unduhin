<script setup lang="ts">
// Custom Windows-style title bar. The native `decorations` are disabled
// in `tauri.conf.json`, so this is the only place the app title and
// window controls live. Dragging is handled by Tauri via the
// `data-tauri-drag-region` attribute; double-clicking the drag region
// also toggles maximize for free.

import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useRoute } from "vue-router";
import { useI18n } from "vue-i18n";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, Copy, X } from "lucide-vue-next";

import { useDetailStore } from "@/stores/detail";
import { useDownloadsStore } from "@/stores/downloads";
import { truncateFilename } from "@/lib/format";

const { t } = useI18n();
const downloads = useDownloadsStore();
const detail = useDetailStore();
const route = useRoute();

const subtitle = computed(() => {
  if (route.path.startsWith("/settings")) return t("downloads.titlebarSettings");
  if (downloads.loading) return t("downloads.titlebarLoading");
  if (detail.openId != null) {
    const r = downloads.records.get(detail.openId);
    if (r) return truncateFilename(r.filename);
  }
  const active = downloads.all.filter(
    (d) => d.status === "active" || d.status === "muxing",
  );
  if (active.length === 1) return truncateFilename(active[0].filename);
  const inflight = downloads.all.filter(
    (d) =>
      d.status === "active" ||
      d.status === "muxing" ||
      d.status === "queued" ||
      d.status === "paused" ||
      d.status === "failed"
  ).length;
  if (downloads.all.length === 0) return t("downloads.titlebarWelcome");
  if (inflight === 0) return t("downloads.titlebarIdle");
  return t("downloads.titlebarActiveCount", { n: inflight });
});

const isMaximized = ref(false);
const win = getCurrentWindow();

let unlisten: (() => void) | undefined;

onMounted(async () => {
  isMaximized.value = await win.isMaximized();
  unlisten = await win.onResized(async () => {
    isMaximized.value = await win.isMaximized();
  });
});

onBeforeUnmount(() => {
  unlisten?.();
});

function minimize() {
  void win.minimize();
}
function toggleMaximize() {
  void win.toggleMaximize();
}
function close() {
  void win.close();
}
</script>

<template>
  <div
    data-tauri-drag-region
    class="flex h-9 shrink-0 select-none items-center justify-between border-b border-border bg-background text-foreground"
  >
    <div
      data-tauri-drag-region
      class="flex h-full min-w-0 flex-1 items-center gap-2 px-3"
    >
      <svg
        class="h-3.5 w-3.5 shrink-0"
        viewBox="0 0 128 128"
        xmlns="http://www.w3.org/2000/svg"
        aria-hidden="true"
      >
        <rect width="128" height="128" rx="24" fill="hsl(var(--primary))" />
        <g fill="hsl(var(--primary-foreground))">
          <rect x="26" y="28" width="14" height="56" rx="3" />
          <rect x="88" y="28" width="14" height="56" rx="3" />
          <rect x="26" y="78" width="76" height="9" rx="3" />
          <rect x="34" y="89" width="60" height="7" rx="3" />
          <rect x="44" y="98" width="40" height="5" rx="2.5" />
        </g>
      </svg>
      <span class="shrink-0 text-xs font-semibold">Unduhin</span>
      <span class="truncate text-xs text-muted-foreground">— {{ subtitle }}</span>
    </div>

    <div class="flex h-full">
      <button
        type="button"
        class="flex h-full w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :title="t('downloads.titlebarMinimize')"
        @click="minimize"
      >
        <Minus class="h-3.5 w-3.5" />
      </button>
      <button
        type="button"
        class="flex h-full w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :title="isMaximized ? t('downloads.titlebarRestore') : t('downloads.titlebarMaximize')"
        @click="toggleMaximize"
      >
        <Copy v-if="isMaximized" class="h-3 w-3 -scale-x-100" />
        <Square v-else class="h-3 w-3" />
      </button>
      <button
        type="button"
        class="flex h-full w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-danger hover:text-white"
        :title="t('downloads.titlebarClose')"
        @click="close"
      >
        <X class="h-4 w-4" />
      </button>
    </div>
  </div>
</template>
