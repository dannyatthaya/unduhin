<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { Clock, MoreHorizontal, Pause, Play, RotateCw, Trash2, X } from "lucide-vue-next";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";

import DetailOverview from "./DetailOverview.vue";
import DetailSegments from "./DetailSegments.vue";
import DetailHistory from "./DetailHistory.vue";
import RowMenu from "@/components/RowMenu.vue";
import ScheduleDialog from "@/components/ScheduleDialog.vue";
import Drawer from "@/components/ui/Drawer.vue";

import { useCategoriesStore } from "@/stores/categories";
import { useDetailStore, type DetailTab } from "@/stores/detail";
import { useDownloadsStore } from "@/stores/downloads";
import { useToast } from "@/composables/useToast";
import { useDeleteConfirm } from "@/composables/useDeleteConfirm";
import { relativeTime } from "@/lib/format";
import type { DownloadRecord } from "@/types/tauri-bindings";

const { t } = useI18n();
const toast = useToast();

const detail = useDetailStore();
const downloads = useDownloadsStore();
const categories = useCategoriesStore();
const deleteConfirm = useDeleteConfirm();

const record = computed<DownloadRecord | null>(() => {
  const id = detail.openId;
  return id == null ? null : downloads.records.get(id) ?? null;
});

const scheduleOpen = ref(false);

// The drawer is shown whenever a download is pinned. Selection and
// detail are now independent surfaces — the floating batch bar pins
// to the bottom and the drawer pins to the right, so they don't fight
// for screen real estate.
const drawerOpen = computed(() => record.value != null);

const segmentsCount = computed(() => record.value?.segments ?? 0);
const historyCount = computed(() =>
  record.value ? downloads.timelineFor(record.value.id).length : 0
);

const tabs = computed<{ id: DetailTab; label: string; count?: () => number }[]>(() => [
  { id: "overview", label: t("detail.tabOverview") },
  { id: "segments", label: t("detail.tabSegments"), count: () => segmentsCount.value },
  { id: "history", label: t("detail.tabHistory"), count: () => historyCount.value },
]);

const breadcrumbs = computed(() => {
  if (!record.value) return [];
  const ext = (() => {
    const i = record.value.filename.lastIndexOf(".");
    if (i < 0) return "File";
    return record.value.filename.slice(i + 1).toUpperCase();
  })();
  const cat = categories.nameOf(record.value.category_id).toUpperCase();
  return [ext, cat];
});

const startedAgo = computed(() =>
  record.value ? relativeTime(record.value.created_at) : "—"
);

const statusTone = computed(() => {
  if (!record.value) return "bg-muted-foreground";
  switch (record.value.status) {
    case "active":
      return "bg-primary";
    case "muxing":
      return "bg-info";
    case "queued":
      return "bg-muted-foreground";
    case "paused":
      return "bg-warning";
    case "failed":
      return "bg-danger";
    case "completed":
      return "bg-success";
    default:
      return "bg-muted-foreground";
  }
});

const statusLabel = computed(() => {
  if (!record.value) return "";
  switch (record.value.status) {
    case "active":
      return t("downloads.statusActive");
    case "muxing":
      return t("downloads.statusMuxing");
    case "paused":
      return t("downloads.statusPaused");
    case "queued":
      return t("downloads.statusQueued");
    case "completed":
      return t("downloads.statusDone");
    case "failed":
      return t("downloads.statusFailed");
    case "cancelled":
      return t("downloads.statusCancelled");
    default:
      return "";
  }
});

const primaryActionLabel = computed(() => {
  if (!record.value) return "";
  switch (record.value.status) {
    case "active":
    case "queued":
      return t("detail.actionPause");
    case "paused":
      return t("detail.actionResume");
    case "failed":
    case "cancelled":
      return t("detail.actionRestart");
    case "completed":
      return t("downloads.rowOpenFile");
    default:
      return "";
  }
});

const primaryActionIcon = computed(() => {
  if (!record.value) return Pause;
  switch (record.value.status) {
    case "active":
    case "queued":
      return Pause;
    case "paused":
      return Play;
    case "failed":
    case "cancelled":
      return RotateCw;
    default:
      return Play;
  }
});

async function runPrimary() {
  if (!record.value) return;
  const id = record.value.id;
  switch (record.value.status) {
    case "active":
    case "queued":
      await downloads.pause(id);
      break;
    case "paused":
      await downloads.resume(id);
      break;
    case "failed":
    case "cancelled":
      try {
        await downloads.retry(id);
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e);
        toast.push(t("errors.restartDownload", { error: message }), "error");
      }
      break;
    case "completed":
      try {
        await openPath(record.value.output_path);
      } catch (e) {
        console.error("openPath failed", e);
        toast.push(
          t("errors.openFile", { filename: record.value.filename }),
          "error",
        );
      }
      break;
  }
}

async function openFolder() {
  if (!record.value) return;
  try {
    await revealItemInDir(record.value.output_path);
  } catch (e) {
    console.error("revealItemInDir failed", e);
    toast.push(t("errors.revealFile"), "error");
  }
}

const moreMenu = computed(() => {
  if (!record.value) return [];
  const id = record.value.id;
  const items: { label: string; danger?: boolean; onSelect: () => void }[] = [
    { label: t("downloads.rowOpenFolder"), onSelect: openFolder },
    {
      label: t("downloads.menuCopyUrl"),
      onSelect: () => {
        if (!record.value) return;
        void navigator.clipboard.writeText(record.value.url);
      },
    },
  ];
  if (record.value.status !== "completed") {
    items.push({
      label: t("downloads.batchCancel"),
      onSelect: () => downloads.cancel(id),
    });
  }
  return items;
});
</script>

<template>
  <Drawer
    :open="drawerOpen"
    @close="detail.close()"
  >
    <template v-if="record">
      <!-- Header -->
      <header class="px-5 pt-4">
        <div class="flex items-center justify-between text-[11px] font-semibold tracking-wider text-muted-foreground">
          <p class="flex items-center gap-1">
            <span
              v-for="(b, i) in breadcrumbs"
              :key="i"
              class="flex items-center gap-1"
            >
              <span>{{ b }}</span>
              <span
                v-if="i < breadcrumbs.length - 1"
                aria-hidden
              >·</span>
            </span>
          </p>
          <button
            type="button"
            class="inline-flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :title="t('common.close')"
            @click="detail.close()"
          >
            <X class="h-4 w-4" />
          </button>
        </div>
        <h2 class="mt-2 wrap-break-word text-lg font-semibold leading-snug text-foreground">
          {{ record.filename }}
        </h2>
        <div class="mt-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <span
            class="inline-flex items-center gap-1.5 rounded-full border border-border bg-card px-2 py-0.5 text-xs font-medium text-foreground"
          >
            <span
              class="h-1.5 w-1.5 rounded-full"
              :class="statusTone"
            />
            {{ statusLabel }}
          </span>
          <span>{{ t("detail.startedAgo", { time: startedAgo }) }}</span>
        </div>
      </header>

      <!-- Tabs -->
      <nav class="mt-4 border-b border-border px-5">
        <ul class="flex items-center gap-4 text-sm">
          <li
            v-for="t in tabs"
            :key="t.id"
          >
            <button
              type="button"
              class="flex items-center gap-1.5 border-b-2 py-2 font-medium transition-colors"
              :class="detail.tab === t.id
                  ? 'border-primary text-primary'
                  : 'border-transparent text-muted-foreground hover:text-foreground'
                "
              @click="detail.setTab(t.id)"
            >
              <span>{{ t.label }}</span>
              <span
                v-if="t.count"
                class="rounded-full bg-muted px-1.5 text-[10px] font-semibold text-muted-foreground"
              >
                {{ t.count() }}
              </span>
            </button>
          </li>
        </ul>
      </nav>

      <!-- Body — vertical scroll only, never horizontal. -->
      <div class="flex-1 overflow-y-auto overflow-x-hidden px-5 py-5">
        <DetailOverview
          v-if="detail.tab === 'overview'"
          :download="record"
          @view-segments="detail.setTab('segments')"
        />
        <DetailSegments
          v-else-if="detail.tab === 'segments'"
          :download="record"
        />
        <DetailHistory
          v-else
          :download="record"
        />
      </div>

      <!-- Footer actions -->
      <footer class="flex items-center gap-2 border-t border-border bg-background px-5 py-3">
        <button
          v-if="primaryActionLabel"
          type="button"
          class="inline-flex h-9 flex-1 items-center justify-center gap-1.5 rounded-md border border-border bg-card text-sm font-medium text-foreground transition-colors hover:bg-accent"
          @click="runPrimary"
        >
          <component
            :is="primaryActionIcon"
            class="h-4 w-4"
          />
          <span>{{ primaryActionLabel }}</span>
        </button>
        <button
          type="button"
          class="inline-flex h-9 w-9 items-center justify-center rounded-md border border-border bg-card text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :title="t('detail.actionSchedule')"
          @click="scheduleOpen = true"
        >
          <Clock class="h-4 w-4" />
        </button>
        <RowMenu :items="moreMenu">
          <MoreHorizontal class="h-4 w-4" />
        </RowMenu>
        <button
          type="button"
          class="inline-flex h-9 w-9 items-center justify-center rounded-md bg-danger text-white transition-colors hover:bg-danger/90"
          :title="t('common.remove')"
          @click="deleteConfirm.requestDelete([record.id])"
        >
          <Trash2 class="h-4 w-4" />
        </button>
      </footer>
    </template>
  </Drawer>
  <ScheduleDialog
    v-if="record"
    :open="scheduleOpen"
    :scope="{ kind: 'download', downloadId: record.id }"
    @close="scheduleOpen = false"
  />
</template>
