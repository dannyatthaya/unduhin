<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { Check, Copy, FolderOpen, RefreshCw } from "lucide-vue-next";

import { invoke } from "@tauri-apps/api/core";
import { openPath } from "@tauri-apps/plugin-opener";

import Button from "@/components/ui/Button.vue";
import BrowserCard from "@/components/settings/browser/BrowserCard.vue";
import type { BrowserRow } from "@/composables/useBrowserStatus";

const props = defineProps<{
  browsers: BrowserRow[];
  loading: boolean;
}>();

const emit = defineEmits<{
  (e: "rescan"): void;
}>();

const { t } = useI18n();

/** The Chromium browsers the installer registers. All share a single card
 *  with per-browser dots. */
const CHROMIUM_PRIMARY: BrowserRow["id"] = "chrome";

const chromiumPrimary = computed<BrowserRow | null>(() =>
  props.browsers.find((b) => b.id === CHROMIUM_PRIMARY) ?? null,
);
const chromiumSiblings = computed<BrowserRow[]>(() =>
  props.browsers.filter(
    (b) => b.family === "chromium" && b.id !== CHROMIUM_PRIMARY,
  ),
);
const firefox = computed<BrowserRow | null>(() =>
  props.browsers.find((b) => b.id === "firefox") ?? null,
);
const safari = computed<BrowserRow | null>(() =>
  props.browsers.find((b) => b.id === "safari") ?? null,
);

/** App-managed canonical unpacked-extension folder. Users Load-unpacked
 *  it once; the app refreshes its contents on every launch and the
 *  extension reloads itself. `null` hides the strip (command failed —
 *  non-Windows dev shell, most likely). */
const extensionDir = ref<string | null>(null);
const copied = ref(false);

onMounted(async () => {
  try {
    extensionDir.value = await invoke<string>("extension_folder_path");
  } catch {
    extensionDir.value = null;
  }
});

function openFolder(): void {
  if (!extensionDir.value) return;
  void openPath(extensionDir.value).catch((e) =>
    console.error("openPath(extension folder) failed", e),
  );
}

function copyPath(): void {
  if (!extensionDir.value) return;
  void navigator.clipboard.writeText(extensionDir.value).then(() => {
    copied.value = true;
    setTimeout(() => {
      copied.value = false;
    }, 1_500);
  });
}
</script>

<template>
  <article
    class="overflow-hidden rounded-lg border border-border bg-card text-card-foreground"
  >
    <header class="flex items-start justify-between gap-4 px-5 py-4">
      <div class="flex flex-col gap-1">
        <h2 class="text-sm font-semibold">
          {{ t("settings.cardBrowserExtensions") }}
        </h2>
        <p class="text-xs text-muted-foreground">
          {{ t("settings.cardBrowserExtensionsDesc") }}
        </p>
      </div>
      <Button
        variant="secondary"
        size="sm"
        :disabled="loading"
        @click="emit('rescan')"
      >
        <RefreshCw
          class="h-3.5 w-3.5"
          :class="loading ? 'animate-spin' : ''"
        />
        {{ t("settings.browserRescan") }}
      </Button>
    </header>
    <div class="border-t border-border px-5 py-4">
      <div
        v-if="loading && browsers.length === 0"
        class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3"
        aria-hidden="true"
      >
        <div
          v-for="i in 3"
          :key="i"
          class="h-28 animate-pulse rounded-lg border border-border bg-muted/40"
        />
      </div>
      <div v-else class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        <BrowserCard
          v-if="chromiumPrimary"
          :primary="chromiumPrimary"
          :siblings="chromiumSiblings"
        />
        <BrowserCard v-if="firefox" :primary="firefox" />
        <BrowserCard v-if="safari" :primary="safari" />
      </div>
    </div>
    <div
      v-if="extensionDir"
      class="flex flex-col gap-2 border-t border-border px-5 py-4"
    >
      <p class="text-xs text-muted-foreground">
        {{ t("settings.browserExtensionInstallHint") }}
      </p>
      <div class="flex items-center gap-2">
        <code
          class="min-w-0 flex-1 truncate rounded border border-border bg-muted/40 px-2 py-1.5 text-xs"
          :title="extensionDir"
        >{{ extensionDir }}</code>
        <Button variant="secondary" size="sm" @click="copyPath">
          <Check v-if="copied" class="h-3.5 w-3.5" />
          <Copy v-else class="h-3.5 w-3.5" />
          {{
            copied
              ? t("settings.browserExtensionPathCopied")
              : t("settings.browserExtensionCopyPath")
          }}
        </Button>
        <Button variant="secondary" size="sm" @click="openFolder">
          <FolderOpen class="h-3.5 w-3.5" />
          {{ t("settings.browserExtensionOpenFolder") }}
        </Button>
      </div>
    </div>
  </article>
</template>
