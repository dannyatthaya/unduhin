<script setup lang="ts">
// Details panel for one download. It opens on top of the list at the right
// edge (420px), so the list underneath never moves or loses columns. Only
// the name, the progress and the tabs stay pinned; the rest scrolls.
// Esc or ✕ closes it (Esc is routed by DownloadsView, after "clear
// selection").

import { computed, nextTick, ref } from "vue";
import { useI18n } from "vue-i18n";
import { X } from "lucide-vue-next";

import DetailFooter from "./DetailFooter.vue";
import DetailHistory from "./DetailHistory.vue";
import DetailOverview from "./DetailOverview.vue";
import DetailProgress from "./DetailProgress.vue";
import DetailSegments from "./DetailSegments.vue";
import DetailTorrent from "./DetailTorrent.vue";
import KindBadge from "@/components/downloads/KindBadge.vue";
import ScheduleDialog from "@/components/ScheduleDialog.vue";

import { useDetailStore, type DetailTab } from "@/stores/detail";
import { useDownloadsStore } from "@/stores/downloads";
import { hostOf } from "@/lib/format";
import type { DownloadRecord } from "@/types/tauri-bindings";

const { t } = useI18n();

const detail = useDetailStore();
const downloads = useDownloadsStore();

const record = computed<DownloadRecord | null>(() => {
  const id = detail.openId;
  return id == null ? null : downloads.records.get(id) ?? null;
});

/** Torrent rows run the librqbit backend: they have no HTTP parts, so the
 *  Parts tab is replaced by a Torrent tab (peers, seeds, files). */
const isTorrent = computed(() => record.value?.kind === "torrent");
const middleTab = computed<DetailTab>(() => (isTorrent.value ? "torrent" : "segments"));

const tabs = computed<{ id: DetailTab; label: string }[]>(() => [
  { id: "overview", label: t("detail.tabOverview") },
  {
    id: middleTab.value,
    label: isTorrent.value ? t("detail.tabTorrent") : t("detail.tabSegments"),
  },
  { id: "history", label: t("detail.tabHistory") },
]);

/** A stale tab that does not apply to this kind (Parts on a torrent row)
 *  falls back to the kind's own middle tab rather than a blank panel. */
const activeTab = computed<DetailTab>(() =>
  tabs.value.some((tab) => tab.id === detail.tab) ? detail.tab : middleTab.value,
);

function onTabKeydown(e: KeyboardEvent) {
  if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
  const ids = tabs.value.map((tab) => tab.id);
  const step = e.key === "ArrowRight" ? 1 : -1;
  const next = ids[(ids.indexOf(activeTab.value) + step + ids.length) % ids.length];
  e.preventDefault();
  detail.setTab(next);
  void nextTick(() => {
    const list = e.currentTarget as HTMLElement | null;
    list?.querySelector<HTMLElement>('[aria-selected="true"]')?.focus();
  });
}

const scheduleOpen = ref(false);

const overviewRef = ref<InstanceType<typeof DetailOverview> | null>(null);
const footerRef = ref<InstanceType<typeof DetailFooter> | null>(null);

/** "Paste new link" in the footer: the box lives at the top of Overview. */
async function focusFreshLink() {
  detail.setTab("overview");
  await nextTick();
  overviewRef.value?.focusFreshLink();
}

function onPanelKeydown(e: KeyboardEvent) {
  if (footerRef.value?.handleKey(e)) {
    e.preventDefault();
    e.stopPropagation();
  }
}
</script>

<template>
  <aside
    v-if="record"
    :aria-label="t('detail.panelAria', { filename: record.filename })"
    class="absolute inset-y-0 right-0 z-20 flex min-h-0 w-[420px] max-w-full flex-col border-l border-border bg-dl-chrome shadow-dl-panel"
    @keydown="onPanelKeydown"
  >
    <header class="flex shrink-0 items-start gap-3 pb-3 pl-5 pr-3 pt-[18px]">
      <KindBadge :filename="record.filename" :download-kind="record.kind" size="lg" />
      <div class="flex min-w-0 flex-1 flex-col gap-[3px]">
        <h2
          class="line-clamp-2 text-[15px] font-semibold leading-[1.3] [overflow-wrap:anywhere]"
          :title="record.filename"
        >
          {{ record.filename }}
        </h2>
        <span class="truncate text-xs text-muted-foreground">
          {{ hostOf(record.url) || t("downloads.kindTorrent") }}
        </span>
      </div>
      <button
        type="button"
        class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
        :aria-label="t('detail.closeAria')"
        :title="t('detail.closeHint')"
        @click="detail.close()"
      >
        <X class="h-[15px] w-[15px]" aria-hidden="true" />
      </button>
    </header>

    <DetailProgress :download="record" />

    <div
      role="tablist"
      :aria-label="t('detail.tabsAria')"
      class="flex shrink-0 gap-0.5 border-b border-dl-line px-5"
      @keydown="onTabKeydown"
    >
      <button
        v-for="tab in tabs"
        :id="`detail-tab-${tab.id}`"
        :key="tab.id"
        type="button"
        role="tab"
        aria-controls="detail-tabpanel"
        :aria-selected="activeTab === tab.id"
        :tabindex="activeTab === tab.id ? 0 : -1"
        class="-mb-px flex h-[38px] items-center px-2.5 font-medium transition-colors focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring"
        :class="
          activeTab === tab.id
            ? 'text-foreground shadow-[inset_0_-2px_0_hsl(var(--primary))]'
            : 'text-muted-foreground hover:text-foreground'
        "
        @click="detail.setTab(tab.id)"
      >
        {{ tab.label }}
      </button>
    </div>

    <!-- Body: vertical scroll only, never horizontal. -->
    <div
      id="detail-tabpanel"
      role="tabpanel"
      :aria-labelledby="`detail-tab-${activeTab}`"
      class="min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-5 py-4"
    >
      <DetailOverview
        v-if="activeTab === 'overview'"
        ref="overviewRef"
        :download="record"
        @edit-schedule="scheduleOpen = true"
      />
      <DetailTorrent v-else-if="activeTab === 'torrent'" :download="record" />
      <DetailSegments v-else-if="activeTab === 'segments'" :download="record" />
      <DetailHistory v-else :download="record" />
    </div>

    <DetailFooter
      ref="footerRef"
      :download="record"
      @paste-link="focusFreshLink"
      @schedule="scheduleOpen = true"
    />
  </aside>
  <ScheduleDialog
    v-if="record"
    :open="scheduleOpen"
    :download-id="record.id"
    @close="scheduleOpen = false"
  />
</template>
