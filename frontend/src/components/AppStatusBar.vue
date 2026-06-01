<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Gauge, Loader2 } from "lucide-vue-next";

import { formatBytes, formatSpeed } from "@/lib/format";
import { useDownloadsStore } from "@/stores/downloads";
import { useSelectionStore } from "@/stores/selection";
import { useSettingsStore } from "@/stores/settings";

defineProps<{ loading?: boolean }>();

const { t } = useI18n();
const downloads = useDownloadsStore();
const selection = useSelectionStore();
const settings = useSettingsStore();

const totals = computed(() => downloads.totals);
const isEmpty = computed(() => downloads.all.length === 0);

const selectedSize = computed(() => {
  let bytes = 0;
  for (const id of selection.ids) {
    const r = downloads.records.get(id);
    if (!r) continue;
    bytes += r.total_bytes ?? r.downloaded_bytes;
  }
  return bytes;
});

const speedLimit = computed(() => {
  const v = settings.values["global_speed_limit_bps"];
  if (typeof v !== "number" || v <= 0) return t("common.off");
  return formatSpeed(v);
});
</script>

<template>
  <footer
    class="flex items-center gap-6 border-t border-border bg-transparent px-5 py-2 text-xs text-muted-foreground"
  >
    <template v-if="loading">
      <div class="flex items-center gap-1.5">
        <Loader2 class="h-3.5 w-3.5 animate-spin text-primary" />
        <span>{{ t("common.loadingDownloads") }}</span>
      </div>
      <div class="ml-auto flex items-center gap-1.5">
        <span class="h-1.5 w-1.5 rounded-full bg-muted-foreground" />
        <span>{{ t("downloads.statusbarConnecting") }}</span>
      </div>
    </template>

    <template v-else-if="!selection.empty">
      <span>
        {{ t("downloads.statusbarSelection", { n: selection.count, size: formatBytes(selectedSize) }) }}
      </span>
      <span>
        {{ t("downloads.statusbarKeyboardHints") }}
      </span>
      <div class="ml-auto flex items-center gap-1.5">
        <span class="h-1.5 w-1.5 rounded-full bg-success" />
        <span>{{ t("downloads.statusbarConnected") }}</span>
      </div>
    </template>

    <template v-else>
      <template v-if="isEmpty">
        <span>{{ t("downloads.statusbarNoDownloads") }}</span>
      </template>
      <template v-else>
        <div class="flex items-center gap-3">
          <span>{{ t("downloads.statusbarCountActive", { n: totals.active }) }}</span>
          <span v-if="totals.queued > 0">
            · {{ t("downloads.statusbarCountQueued", { n: totals.queued }) }}
          </span>
          <span v-if="totals.paused > 0">
            · {{ t("downloads.statusbarCountPaused", { n: totals.paused }) }}
          </span>
        </div>

        <div class="flex items-center gap-1.5">
          <span>{{ t("downloads.statusbarAggregate", { speed: formatSpeed(downloads.aggregateSpeedBps) }) }}</span>
        </div>

        <div class="flex items-center gap-1.5">
          <Gauge class="h-3.5 w-3.5" />
          <span>{{ t("downloads.statusbarSpeedLimit", { limit: speedLimit }) }}</span>
        </div>
      </template>

      <div class="ml-auto flex items-center gap-1.5">
        <span class="h-1.5 w-1.5 rounded-full bg-success" />
        <span>{{ isEmpty ? t("downloads.statusbarConnectedReady") : t("downloads.statusbarConnected") }}</span>
      </div>
    </template>
  </footer>
</template>
