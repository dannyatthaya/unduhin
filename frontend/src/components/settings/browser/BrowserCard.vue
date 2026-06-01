<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Chrome, Globe, Flame, Compass } from "lucide-vue-next";

import type { BrowserRow } from "@/composables/useBrowserStatus";

/** Where the Firefox install-instructions link points. The Firefox card
 *  is a placeholder (*out of scope*) — the doc page itself ships as a
 *  stub at the URL below. */
const FIREFOX_DOCS_URL = "https://docs.unduhin.app/firefox";

const props = defineProps<{
  /**
   * Either a single row (Firefox, presented as a standalone card) or
   * the cluster of Chromium browsers that share one card.
   */
  primary: BrowserRow;
  /** Sibling Chromium rows surfaced beneath the primary row. */
  siblings?: BrowserRow[];
}>();

const { t } = useI18n();

/** Resolve a per-family icon. Chrome-family browsers share the
 *  generic Chrome glyph; Firefox gets its own; Safari gets the
 *  Compass glyph that matches the mockup. */
function iconFor(id: BrowserRow["id"]) {
  switch (id) {
    case "firefox":
      return Flame;
    case "safari":
      return Compass;
    case "edge":
      return Globe;
    default:
      return Chrome;
  }
}

const allRows = computed<BrowserRow[]>(() => {
  if (props.siblings && props.siblings.length > 0) {
    return [props.primary, ...props.siblings];
  }
  return [props.primary];
});

/** Card-level health state used to pick the eyebrow text and version
 *  line. Only meaningful on the primary row. */
const cardState = computed<"installed" | "needs-install" | "not-on-platform">(
  () => {
    // Safari is a static stub on Windows — the card never reports
    // anything but the macOS-Q3 placeholder.
    if (props.primary.family === "safari") return "not-on-platform";
    // For Chromium family: any sibling counts as "installed" if its
    // host is registered.
    const anyRegistered = allRows.value.some((r) => r.host_registered);
    if (anyRegistered) return "installed";
    // Browser installed but host registration missing → "needs install".
    const anyInstalled = allRows.value.some((r) => r.installed);
    if (anyInstalled) return "needs-install";
    return "not-on-platform";
  },
);

const sublabel = computed(() => {
  if (props.primary.family === "firefox") return t("settings.browserFirefoxSublabel");
  if (props.primary.family === "safari") return t("settings.browserSafariSublabel");
  // Chromium card lists how many siblings are installed.
  const installedCount = allRows.value.filter((r) => r.installed).length;
  return t("settings.browserChromiumSublabel", { n: installedCount });
});

function dotClass(row: BrowserRow): string {
  if (row.host_registered) return "bg-success";
  if (row.installed) return "bg-warn";
  return "bg-muted-foreground/30";
}
</script>

<template>
  <article
    class="flex h-full flex-col gap-3 rounded-lg border border-border bg-card p-4 text-card-foreground"
  >
    <header class="flex items-start gap-3">
      <div
        class="grid h-9 w-9 shrink-0 place-items-center rounded-md bg-primary/10 text-primary"
      >
        <component :is="iconFor(primary.id)" class="h-4 w-4" />
      </div>
      <div class="flex flex-col">
        <span class="text-sm font-semibold">{{ primary.label }}</span>
        <span class="text-xs text-muted-foreground">{{ sublabel }}</span>
      </div>
    </header>

    <div
      v-if="primary.family !== 'safari'"
      class="flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground"
    >
      <span
        v-for="row in allRows"
        :key="row.id"
        class="inline-flex items-center gap-1.5"
        :title="
          row.host_registered
            ? t('settings.browserDotRegistered')
            : row.installed
              ? t('settings.browserDotInstalled')
              : t('settings.browserDotMissing')
        "
      >
        <span class="h-1.5 w-1.5 rounded-full" :class="dotClass(row)" />
        <span>{{ row.label }}</span>
      </span>
    </div>

    <p
      class="text-xs"
      :class="
        cardState === 'installed'
          ? 'text-success'
          : cardState === 'needs-install'
            ? 'text-warn'
            : 'text-muted-foreground'
      "
    >
      <template v-if="cardState === 'installed'">
        {{ t("settings.browserStateInstalled") }}
      </template>
      <template v-else-if="cardState === 'needs-install'">
        {{ t("settings.browserStateNeedsInstall") }}
      </template>
      <template v-else-if="primary.family === 'firefox' && primary.installed">
        {{ t("settings.browserStateFirefoxInstalledNoExt") }}
      </template>
      <template v-else-if="primary.family === 'firefox'">
        {{ t("settings.browserStateFirefoxSoon") }}
      </template>
      <template v-else-if="primary.family === 'safari'">
        {{ t("settings.browserStateSafariSoon") }}
      </template>
      <template v-else>
        {{ t("settings.browserStateNoChromium") }}
      </template>
    </p>

    <a
      v-if="primary.family === 'firefox'"
      :href="FIREFOX_DOCS_URL"
      target="_blank"
      rel="noopener noreferrer"
      class="mt-auto text-xs font-medium text-primary underline-offset-2 hover:underline"
    >
      {{ t("settings.browserFirefoxInstallLink") }}
    </a>
    <span v-else class="mt-auto" />
  </article>
</template>
