<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import {
  Folder,
  FileText,
  Music2,
  Video,
  Archive,
  LayoutGrid,
  CircleHelp,
} from "lucide-vue-next";

import { useCategoriesStore } from "@/stores/categories";
import { useDownloadsStore } from "@/stores/downloads";
import { useSystemStore } from "@/stores/system";
import { useToast } from "@/composables/useToast";
import { formatBytes } from "@/lib/format";
import type { CategoryId, DownloadId, Status } from "@/types/tauri-bindings";

const { t } = useI18n();

const props = defineProps<{
  selectedCategoryId: number | null;
  selectedStatus: Status | null;
  loading?: boolean;
}>();

const emit = defineEmits<{
  "select-category": [id: number | null];
  "select-status": [status: Status | null];
}>();

const categories = useCategoriesStore();
const downloads = useDownloadsStore();
const system = useSystemStore();

const allCount = computed(() => downloads.all.length);

const versionLine = computed(() => {
  const v = system.appInfo?.version;
  const base = v ? `v${v}` : "—";
  return downloads.all.length === 0
    ? t("downloads.sidebarFirstRun", { version: v ?? "—" })
    : t("downloads.sidebarVersion", { version: v ?? "—" });
});

const diskLine = computed(() => {
  if (props.loading) return t("downloads.sidebarDiskChecking");
  if (!system.disk) return t("downloads.sidebarDiskChecking2");
  const free = formatBytes(system.disk.free_bytes, 0);
  const drive = system.disk.drive.replace(/\\$/, "");
  return t("downloads.sidebarDiskFree", { free, drive });
});

function countForCategory(id: number): number {
  return downloads.all.filter((d) => d.category_id === id).length;
}

function countLabel(n: number): string {
  return props.loading ? "—" : String(n);
}

function iconFor(name: string) {
  switch (name) {
    case "Documents":
      return FileText;
    case "Music":
      return Music2;
    case "Video":
      return Video;
    case "Compressed":
      return Archive;
    case "Programs":
      return LayoutGrid;
    case "Other":
      return CircleHelp;
    default:
      return Folder;
  }
}

const statusItems = computed<{ status: Status; label: string; tone: string }[]>(() => [
  { status: "active", label: t("downloads.filterActive"), tone: "bg-primary" },
  { status: "queued", label: t("downloads.filterQueued"), tone: "bg-muted-foreground" },
  { status: "paused", label: t("downloads.filterPaused"), tone: "bg-warning" },
  { status: "completed", label: t("downloads.filterCompleted"), tone: "bg-success" },
  { status: "failed", label: t("downloads.filterFailed"), tone: "bg-danger" },
]);

function statusCount(s: Status): number {
  return downloads.all.filter((d) => d.status === s).length;
}

// --- Drag-to-recategorize -------------------------------------------------
//
// DownloadRow ships a JSON array of download ids under the
// `application/x-unduhin-ids` MIME on drag-start. We accept the drop on
// each category button and the All-downloads button (the latter clears
// the assignment). The dragOverId ref drives the highlight ring.

const toast = useToast();
const dragOverId = ref<CategoryId | "all" | null>(null);

function hasUnduhinPayload(e: DragEvent): boolean {
  if (!e.dataTransfer) return false;
  return e.dataTransfer.types.includes("application/x-unduhin-ids");
}

function readIds(e: DragEvent): DownloadId[] {
  const raw = e.dataTransfer?.getData("application/x-unduhin-ids");
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((x): x is number => typeof x === "number");
  } catch {
    return [];
  }
}

function onDragEnter(e: DragEvent, target: CategoryId | "all") {
  if (!hasUnduhinPayload(e)) return;
  e.preventDefault();
  dragOverId.value = target;
}

function onDragOver(e: DragEvent, target: CategoryId | "all") {
  if (!hasUnduhinPayload(e)) return;
  e.preventDefault();
  if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
  dragOverId.value = target;
}

function onDragLeave(target: CategoryId | "all") {
  if (dragOverId.value === target) dragOverId.value = null;
}

async function onDrop(e: DragEvent, target: CategoryId | "all", label: string) {
  if (!hasUnduhinPayload(e)) return;
  e.preventDefault();
  dragOverId.value = null;
  const ids = readIds(e);
  if (ids.length === 0) return;

  const desired = target === "all" ? null : target;
  // Skip rows already in the target.
  const candidates = ids.filter((id) => {
    const rec = downloads.records.get(id);
    return rec ? rec.category_id !== desired : false;
  });
  if (candidates.length === 0) return;

  const settled = await Promise.allSettled(
    candidates.map((id) => downloads.setCategory(id, desired)),
  );
  const ok = settled.filter((s) => s.status === "fulfilled").length;
  const failed = settled.length - ok;
  const action =
    target === "all"
      ? t("notify.uncategorizedAction")
      : t("notify.movedToAction", { category: label });
  if (failed === 0) {
    toast.push(
      t("downloads.batchMoved", { n: ok, category: label }, ok),
      "success",
    );
  } else if (ok === 0) {
    toast.push(
      t("downloads.batchFailed", { action, n: failed }),
      "error",
    );
  } else {
    toast.push(
      t("downloads.batchMixed", { action, ok, failed }),
      "error",
    );
  }
}
</script>

<template>
  <aside
    class="flex h-full w-60 shrink-0 flex-col border-r border-border bg-sidebar text-sidebar-foreground"
  >
    <header class="px-5 pt-6 pb-4">
      <h1 class="font-serif text-2xl font-black tracking-tight">Unduhin</h1>
      <p class="mt-0.5 font-mono text-[11px] text-muted-foreground">
        {{ versionLine }}
      </p>
    </header>

    <nav class="flex-1 overflow-y-auto px-3 pb-3">
      <h2 class="px-2 pb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
        {{ t("downloads.sidebarCategories") }}
      </h2>
      <ul class="space-y-0.5">
        <li>
          <button
            class="group flex w-full items-center gap-2.5 rounded-md px-2 py-2 text-sm font-medium transition-colors"
            :class="[
              props.selectedCategoryId == null && props.selectedStatus == null
                ? 'bg-primary/10 text-primary'
                : 'text-foreground hover:bg-accent',
              dragOverId === 'all'
                ? 'ring-2 ring-primary ring-inset bg-primary/15'
                : '',
            ]"
            @click="
              emit('select-category', null);
              emit('select-status', null);
            "
            @dragenter="onDragEnter($event, 'all')"
            @dragover="onDragOver($event, 'all')"
            @dragleave="onDragLeave('all')"
            @drop="onDrop($event, 'all', t('downloads.sidebarAllDownloads'))"
          >
            <Folder class="h-4 w-4" />
            <span class="flex-1 text-left">{{ t("downloads.sidebarAllDownloads") }}</span>
            <span
              class="rounded-md px-1.5 py-0.5 text-xs font-semibold"
              :class="
                props.selectedCategoryId == null && props.selectedStatus == null
                  ? 'bg-primary text-primary-foreground'
                  : 'bg-muted text-muted-foreground'
              "
            >
              {{ countLabel(allCount) }}
            </span>
          </button>
        </li>
        <li v-for="cat in categories.list" :key="cat.id">
          <button
            class="group flex w-full items-center gap-2.5 rounded-md px-2 py-2 text-sm transition-colors"
            :class="[
              props.selectedCategoryId === cat.id
                ? 'bg-primary/10 text-primary'
                : 'text-foreground hover:bg-accent',
              dragOverId === cat.id
                ? 'ring-2 ring-primary ring-inset bg-primary/15'
                : '',
            ]"
            @click="
              emit('select-category', cat.id);
              emit('select-status', null);
            "
            @dragenter="onDragEnter($event, cat.id)"
            @dragover="onDragOver($event, cat.id)"
            @dragleave="onDragLeave(cat.id)"
            @drop="onDrop($event, cat.id, cat.name)"
          >
            <component :is="iconFor(cat.name)" class="h-4 w-4" />
            <span class="flex-1 text-left">{{ cat.name }}</span>
            <span class="rounded-md bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
              {{ countLabel(countForCategory(cat.id)) }}
            </span>
          </button>
        </li>
      </ul>

      <h2 class="mt-6 px-2 pb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
        {{ t("downloads.sidebarStatusHeader") }}
      </h2>
      <ul class="space-y-0.5">
        <li v-for="item in statusItems" :key="item.status">
          <button
            class="group flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-sm transition-colors"
            :class="
              props.selectedStatus === item.status
                ? 'bg-primary/10 text-primary'
                : 'text-foreground hover:bg-accent'
            "
            @click="emit('select-status', props.selectedStatus === item.status ? null : item.status)"
          >
            <span class="h-1.5 w-1.5 rounded-full" :class="item.tone" />
            <span class="flex-1 text-left">{{ item.label }}</span>
            <span class="text-xs text-muted-foreground">
              {{ countLabel(statusCount(item.status)) }}
            </span>
          </button>
        </li>
      </ul>
    </nav>

    <footer class="border-t border-border px-5 py-3">
      <p class="text-xs text-muted-foreground">{{ diskLine }}</p>
      <div
        v-if="props.loading"
        class="mt-1.5 h-1 w-full overflow-hidden rounded-full bg-muted"
      >
        <div class="h-full w-1/3 animate-pulse rounded-full bg-primary/60" />
      </div>
    </footer>
  </aside>
</template>
