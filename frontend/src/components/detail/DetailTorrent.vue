<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { ArrowDown, ArrowUp, Magnet, FileDown, Hash } from "lucide-vue-next";

import ProgressBar from "@/components/ProgressBar.vue";

import { useDownloadsStore } from "@/stores/downloads";
import { formatBytes, formatSpeed } from "@/lib/format";
import {
  fileProgressRows,
  formatRatio,
  torrentSourceLabel,
} from "@/lib/torrentFormat";
import type { DownloadRecord } from "@/types/tauri-bindings";

const { t } = useI18n();
const props = defineProps<{ download: DownloadRecord }>();

const store = useDownloadsStore();
const liveFiles = computed(() => store.liveTorrentFilesFor(props.download.id));

/** The persisted torrent blob — present on every `kind === "torrent"` row,
 *  but defensively guarded for the brief window before metadata resolves. */
const meta = computed(() => props.download.torrent);
const swarm = computed(() => meta.value?.swarm ?? null);

/** librqbit only reports a swarm snapshot once the session attaches; show a
 *  waiting state until the first `swarm_progress` tick lands (and survives a
 *  relaunch via the persisted blob). */
const hasSwarm = computed(() => swarm.value != null);

const ratio = computed(() => formatRatio(swarm.value?.ratio_milli));

const sourceIcon = computed(() => {
  switch (meta.value?.source.kind) {
    case "file":
      return FileDown;
    case "info_hash":
      return Hash;
    default:
      return Magnet;
  }
});

const sourceLabel = computed(() =>
  meta.value ? torrentSourceLabel(meta.value.source) : "—",
);

const rows = computed(() => fileProgressRows(meta.value, liveFiles.value));

const selectedCount = computed(
  () => rows.value.filter((r) => r.selected).length,
);

function rowTone(done: boolean): "active" | "done" {
  return done ? "done" : "active";
}
</script>

<template>
  <div class="space-y-5">
    <!-- Swarm stats strip -->
    <section>
      <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
        {{ t("detail.torrentSwarm") }}
      </h3>
      <div
        v-if="hasSwarm && swarm"
        class="grid grid-cols-3 gap-2 rounded-lg border border-border bg-card px-3 py-2.5"
      >
        <div>
          <p class="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            {{ t("detail.torrentPeers") }}
          </p>
          <p class="mt-0.5 text-base font-semibold text-foreground">{{ swarm.peers }}</p>
        </div>
        <div>
          <p class="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            {{ t("detail.torrentSeeds") }}
          </p>
          <p class="mt-0.5 text-base font-semibold text-foreground">{{ swarm.seeds }}</p>
        </div>
        <div>
          <p class="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            {{ t("detail.torrentRatio") }}
          </p>
          <p class="mt-0.5 text-base font-semibold text-foreground tabular-nums">{{ ratio }}</p>
        </div>
        <div class="col-span-3 flex items-center gap-4 border-t border-border/60 pt-2 text-xs">
          <span class="flex items-center gap-1.5 text-foreground">
            <ArrowDown
              aria-hidden
              class="h-3.5 w-3.5 text-success"
            />
            <span class="font-medium">{{ formatSpeed(swarm.down_bps) }}</span>
          </span>
          <span class="flex items-center gap-1.5 text-foreground">
            <ArrowUp
              aria-hidden
              class="h-3.5 w-3.5 text-info"
            />
            <span class="font-medium">{{ formatSpeed(swarm.up_bps) }}</span>
          </span>
        </div>
      </div>
      <p
        v-else
        class="rounded-lg border border-dashed border-border bg-card px-4 py-3 text-xs text-muted-foreground"
      >
        {{ t("detail.torrentSwarmWaiting") }}
      </p>
    </section>

    <!-- Source -->
    <section>
      <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
        {{ t("detail.torrentSource") }}
      </h3>
      <div class="rounded-lg border border-border bg-card px-4 py-3 text-xs">
        <p class="flex items-start gap-2">
          <component
            :is="sourceIcon"
            aria-hidden
            class="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground"
          />
          <span class="break-all font-mono text-foreground">{{ sourceLabel }}</span>
        </p>
        <p
          v-if="meta"
          class="mt-2 grid grid-cols-[88px_1fr] items-baseline gap-2 text-muted-foreground"
        >
          <span class="font-semibold uppercase tracking-wider">{{ t("detail.torrentInfoHash") }}</span>
          <span class="break-all font-mono text-foreground">{{ meta.info_hash }}</span>
        </p>
      </div>
    </section>

    <!-- Per-file progress -->
    <section>
      <h3 class="mb-2 flex items-center justify-between text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
        <span>{{ t("detail.torrentFiles") }}</span>
        <span
          v-if="rows.length > 0"
          class="font-normal normal-case tracking-normal"
        >
          {{ t("detail.torrentFilesSelected", { selected: selectedCount, total: rows.length }) }}
        </span>
      </h3>
      <div
        v-if="rows.length > 0"
        class="overflow-hidden rounded-lg border border-border"
      >
        <ul class="divide-y divide-border">
          <li
            v-for="r in rows"
            :key="r.index"
            class="px-3 py-2.5"
            :class="r.selected ? '' : 'opacity-50'"
          >
            <div class="flex items-center justify-between gap-3 text-xs">
              <span
                class="min-w-0 flex-1 truncate font-mono text-foreground"
                :title="r.path"
              >{{ r.path }}</span>
              <span class="shrink-0 font-mono text-muted-foreground">
                {{ formatBytes(r.downloaded, 0) }}/{{ formatBytes(r.length, 0) }}
              </span>
            </div>
            <ProgressBar
              :value="r.pct"
              :tone="rowTone(r.done)"
              class="mt-1.5"
            />
            <p
              v-if="!r.selected"
              class="mt-1 text-[10px] uppercase tracking-wider text-muted-foreground"
            >
              {{ t("detail.torrentFileSkipped") }}
            </p>
          </li>
        </ul>
      </div>
      <p
        v-else
        class="rounded-lg border border-dashed border-border bg-card px-4 py-3 text-xs text-muted-foreground"
      >
        {{ t("detail.torrentFilesWaiting") }}
      </p>
    </section>
  </div>
</template>
