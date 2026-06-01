<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { Minus, Plus } from "lucide-vue-next";
import { useDebounceFn } from "@vueuse/core";

import SegmentBars from "./SegmentBars.vue";

import { useDownloadsStore } from "@/stores/downloads";
import { useToast } from "@/composables/useToast";
import { formatBytes, formatSpeed } from "@/lib/format";
import { api, type DownloadRecord, type SegmentState } from "@/types/tauri-bindings";

const { t } = useI18n();
const props = defineProps<{ download: DownloadRecord }>();

const store = useDownloadsStore();
const stats = computed(() => store.statsFor(props.download.id));
const live = computed(() => store.liveSegmentsFor(props.download.id));

/** yt-dlp-driven downloads are single-stream from our perspective —
 *  the segments grid + tuning controls don't apply. We render a
 *  dedicated panel instead. */
const isMedia = computed(() => props.download.media_info != null);

/** Hide the Tuning section on rows that can never accept the change
 *  anyway. Mid-flight re-segmentation isn't wired yet; for
 *  now we just gate visibility on terminal status. */
const isTerminal = computed(
  () =>
    props.download.status === "completed" ||
    props.download.status === "failed" ||
    props.download.status === "cancelled"
);

interface SegmentRow {
  index: number;
  range: string;
  got: string;
  speedBps: number | null;
  pct: number;
  isSlow: boolean;
  isDone: boolean;
}

function formatRange(start: number, end: number): string {
  const a = formatRangeUnit(start);
  const b = formatRangeUnit(end);
  return `${a}–${b}`;
}

function formatRangeUnit(n: number): string {
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)}KB`;
  if (n < 1024 * 1024 * 1024) return `${Math.round(n / (1024 * 1024))}MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(1)}G`;
}

/** Per-segment speed and Slow/Stalled state come from the engine via
 *  the `segment_progress` event; we do NOT synthesize either from the
 *  aggregate. When live data is missing (no tick yet, or after reload)
 *  we fall back to the persisted byte ranges for shape only. */
const rows = computed<SegmentRow[]>(() => {
  const segs = props.download.segments_meta ?? [];
  const liveMap = live.value;
  if (segs.length === 0) {
    return Array.from({ length: props.download.segments }, (_, i) => {
      const l = liveMap?.get(i);
      const pct = l ? Math.min(100, Math.round((l.bytes / Math.max(1, l.total)) * 100)) : 0;
      const isDone = l?.state === "done";
      const isSlow = l?.state === "slow" || l?.state === "stalled";
      return {
        index: i,
        range: "—",
        got: "—",
        speedBps: isDone ? null : l?.speed_bps ?? null,
        pct,
        isSlow,
        isDone,
      };
    });
  }
  return segs.map((s: SegmentState) => {
    const total = Math.max(1, s.segment.end - s.segment.start);
    const pct = Math.min(100, Math.round((s.bytes_downloaded / total) * 100));
    const l = liveMap?.get(s.segment.index);
    const isDone = l ? l.state === "done" : pct >= 100;
    const isSlow = l?.state === "slow" || l?.state === "stalled";
    return {
      index: s.segment.index,
      range: formatRange(s.segment.start, s.segment.end),
      got: `${formatBytes(s.bytes_downloaded, 0)}/${formatBytes(total, 0)}`,
      speedBps: isDone ? null : l?.speed_bps ?? null,
      pct,
      isSlow,
      isDone,
    };
  });
});

const counts = computed(() => {
  const total = rows.value.length;
  const done = rows.value.filter((r) => r.isDone).length;
  const slow = rows.value.filter((r) => r.isSlow).length;
  return { total, done, slow, active: total - done };
});

const slowestIndex = computed(() => {
  const slow = rows.value.find((r) => r.isSlow);
  return slow ? slow.index : null;
});

const slowMessage = computed(() => {
  if (slowestIndex.value == null) return null;
  return t("detail.segmentsSlowWarning", { index: slowestIndex.value + 1 });
});

// Segment-count tuning ± control. Click → optimistic local update,
// debounced 400 ms call to `set_segments` so a fast burst of clicks
// only fires one backend request. The engine applies the change live
// (split / graceful join) on mid-flight transfers; for queued / paused
// downloads it just updates the persisted intent.
const { push: pushToast } = useToast();
const segmentCount = ref(props.download.segments);
watch(
  () => props.download.segments,
  (n) => {
    segmentCount.value = n;
  }
);
const pushSegments = useDebounceFn(async (n: number) => {
  try {
    await api.setSegments(props.download.id, n);
  } catch (e: unknown) {
    const message =
      (e as { message?: string })?.message ?? t("errors.generic", { error: "" });
    pushToast(t("errors.setSegments", { error: message }), "error");
    // Roll back to the server's truth on failure.
    segmentCount.value = props.download.segments;
  }
}, 400);
function bumpSegments(delta: number) {
  const next = Math.max(1, Math.min(32, segmentCount.value + delta));
  if (next === segmentCount.value) return;
  segmentCount.value = next;
  void pushSegments(next);
}

function pctClass(r: SegmentRow): string {
  if (r.isDone) return "text-success";
  if (r.isSlow) return "text-warning";
  return "text-primary";
}
</script>

<template>
  <!-- Media (yt-dlp) downloads: single stream, no per-segment view. -->
  <div
    v-if="isMedia"
    class="space-y-3"
  >
    <section class="rounded-lg border border-border bg-card px-4 py-3">
      <h3 class="text-sm font-semibold text-foreground">{{ t("detail.singleStreamTitle") }}</h3>
      <p class="mt-1 text-xs text-muted-foreground">
        {{ t("detail.singleStreamBody") }}
      </p>
    </section>
    <section
      v-if="download.media_info"
      class="rounded-lg border border-border bg-card px-4 py-3 text-xs text-muted-foreground"
    >
      <dl class="grid grid-cols-[88px_1fr] gap-y-1.5">
        <dt class="font-semibold uppercase tracking-wider">{{ t("detail.mediaSource") }}</dt>
        <dd class="font-mono">{{ download.media_info.extractor }}</dd>
        <dt class="font-semibold uppercase tracking-wider">{{ t("detail.mediaFormat") }}</dt>
        <dd class="font-mono">{{ download.media_info.format_selector }}</dd>
        <dt class="font-semibold uppercase tracking-wider">{{ t("detail.mediaMuxed") }}</dt>
        <dd>{{ download.media_info.needs_ffmpeg ? t("detail.mediaMuxedYes") : t("common.no") }}</dd>
      </dl>
    </section>
  </div>

  <!-- Engine downloads: full multi-segment view. -->
  <div
    v-else
    class="space-y-5"
  >
    <!-- Stats strip -->
    <section class="grid grid-cols-4 gap-2 rounded-lg border border-border bg-card px-3 py-2.5">
      <div>
        <p class="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          {{ t("detail.overviewActiveStats") }}
        </p>
        <p class="mt-0.5 text-base font-semibold text-foreground">
          {{ counts.active }}<span class="text-sm font-normal text-muted-foreground">/{{ counts.total }}</span>
        </p>
      </div>
      <div>
        <p class="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          {{ t("detail.overviewDoneStats") }}
        </p>
        <p class="mt-0.5 text-base font-semibold text-success">{{ counts.done }}</p>
      </div>
      <div>
        <p class="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          {{ t("detail.overviewSlowStats") }}
        </p>
        <p
          class="mt-0.5 text-base font-semibold"
          :class="counts.slow > 0 ? 'text-warning' : 'text-muted-foreground'"
        >
          {{ counts.slow }}
        </p>
      </div>
      <div>
        <p class="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          {{ t("detail.overviewCombinedStats") }}
        </p>
        <p class="mt-0.5 flex items-baseline gap-1 text-base font-semibold text-foreground">
          {{ stats ? formatSpeed(stats.speed_bps) : "—" }}
          <span
            aria-hidden
            class="text-xs text-muted-foreground"
          >↓</span>
        </p>
      </div>
    </section>

    <section>
      <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
        {{ t("detail.segmentHealth") }}
      </h3>
      <SegmentBars
        :segments="download.segments_meta ?? []"
        :bars="download.segments"
        :slow-segment="slowestIndex"
        :height="44"
      />
    </section>

    <section>
      <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
        {{ t("detail.perSegment") }}
      </h3>
      <div class="overflow-hidden rounded-lg border border-border">
        <table class="w-full table-fixed text-xs">
          <thead>
            <tr class="bg-muted/40 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
              <th class="w-9 px-2 py-2 text-left">{{ t("detail.segmentTableIndex") }}</th>
              <th class="px-2 py-2 text-left">{{ t("detail.segmentRange") }}</th>
              <th class="px-2 py-2 text-left">{{ t("detail.segmentGot") }}</th>
              <th class="w-17 px-2 py-2 text-left">{{ t("detail.segmentTableSpeed") }}</th>
              <th class="w-11 px-2 py-2 text-right">{{ t("detail.segmentTableProgress") }}</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="r in rows"
              :key="r.index"
              class="border-t border-border"
              :class="r.isSlow ? 'bg-warning/10' : ''"
            >
              <td class="px-2 py-2 font-mono text-foreground">
                {{ String(r.index + 1).padStart(2, "0") }}
              </td>
              <td class="px-2 py-2 font-mono text-foreground">{{ r.range }}</td>
              <td class="px-2 py-2 font-mono text-foreground">{{ r.got }}</td>
              <td class="px-2 py-2 font-mono text-muted-foreground">
                {{ r.isDone ? "—" : r.speedBps && r.speedBps > 0 ? formatSpeed(r.speedBps) : "—" }}
              </td>
              <td
                class="px-2 py-2 text-right font-mono font-semibold"
                :class="pctClass(r)"
              >
                {{ r.pct }}%
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <p
        v-if="slowMessage"
        class="mt-3 flex items-start gap-1.5 text-xs text-warning"
      >
        <span aria-hidden>⚠</span>
        <span>{{ slowMessage }}</span>
      </p>
    </section>

    <section v-if="!isTerminal">
      <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
        {{ t("detail.segmentTuning") }}
      </h3>
      <div class="flex items-center justify-between rounded-lg border border-border bg-card px-3 py-2.5">
        <div>
          <p class="text-sm font-medium text-foreground">{{ t("detail.segmentCount") }}</p>
          <p class="text-xs text-muted-foreground">{{ t("detail.segmentCountHint") }}</p>
        </div>
        <div class="flex items-center gap-1.5">
          <button
            type="button"
            class="inline-flex h-7 w-7 items-center justify-center rounded-md border border-border text-foreground transition-colors hover:bg-accent disabled:opacity-50"
            :disabled="segmentCount <= 1"
            @click="bumpSegments(-1)"
            :aria-label="t('detail.decreaseSegments')"
          >
            <Minus class="h-3.5 w-3.5" />
          </button>
          <span class="w-6 text-center font-mono text-sm font-semibold">{{ segmentCount }}</span>
          <button
            type="button"
            class="inline-flex h-7 w-7 items-center justify-center rounded-md border border-border text-foreground transition-colors hover:bg-accent disabled:opacity-50"
            :disabled="segmentCount >= 32"
            @click="bumpSegments(1)"
            :aria-label="t('detail.increaseSegments')"
          >
            <Plus class="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
    </section>
  </div>
</template>
