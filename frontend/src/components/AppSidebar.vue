<script setup lang="ts">
// Downloads sidebar: the New download button, the library (categories),
// and a footer with the browser bridge state, free disk space and the
// Settings link. In a narrow window (`rail`) it collapses to a 56px icon
// rail. Status filters live in the toolbar tabs, not here.

import { computed, ref, type Component } from "vue";
import { useI18n } from "vue-i18n";
import {
  Archive,
  CircleHelp,
  FileText,
  Folder,
  HardDrive,
  Inbox,
  LayoutGrid,
  Music2,
  Plus,
  Settings,
  Video,
} from "lucide-vue-next";

import { useCategoriesStore } from "@/stores/categories";
import { useDownloadsStore } from "@/stores/downloads";
import { useSystemStore } from "@/stores/system";
import { useBrowserStatus } from "@/composables/useBrowserStatus";
import { useToast } from "@/composables/useToast";
import { iconFor as iconForKey } from "@/lib/categoryIcons";
import { formatBytes } from "@/lib/format";
import { isMacOS } from "@/lib/platform";
import type { Category, CategoryId, DownloadId } from "@/types/tauri-bindings";

const { t } = useI18n();

const props = defineProps<{
  selectedCategoryId: number | null;
  loading?: boolean;
  /** Collapse to the icon rail. */
  rail?: boolean;
}>();

const emit = defineEmits<{
  "select-category": [id: number | null];
  "add-url": [];
}>();

const categories = useCategoriesStore();
const downloads = useDownloadsStore();
const system = useSystemStore();
const { status: browserStatus } = useBrowserStatus();

const newShortcut = isMacOS ? "⌘N" : "Ctrl N";

const diskLine = computed(() => {
  if (props.loading) return t("downloads.sidebarDiskChecking");
  if (!system.disk) return t("downloads.sidebarDiskChecking2");
  const free = formatBytes(system.disk.free_bytes, 0);
  const drive = system.disk.drive.replace(/\\$/, "");
  return t("downloads.sidebarDiskFree", { free, drive });
});

// The pipe the browser extension talks to. "Ready" means the app is
// listening for it — the backend has no signal for a live extension.
const bridgeReady = computed(() => browserStatus.value?.pipe.listening === true);
const bridgeLabel = computed(() =>
  bridgeReady.value ? t("downloads.bridgeReady") : t("downloads.bridgeOffline"),
);

function iconForName(name: string): Component {
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

function iconForCategory(cat: Category): Component {
  return cat.icon ? iconForKey(cat.icon).icon : iconForName(cat.name);
}

interface NavItem {
  key: CategoryId | "all";
  id: CategoryId | null;
  label: string;
  icon: Component;
  count: number;
  current: boolean;
}

const items = computed<NavItem[]>(() => {
  const counts = new Map<CategoryId, number>();
  for (const d of downloads.all) {
    if (d.category_id == null) continue;
    counts.set(d.category_id, (counts.get(d.category_id) ?? 0) + 1);
  }
  return [
    {
      key: "all",
      id: null,
      label: t("downloads.sidebarAllDownloads"),
      icon: Inbox,
      count: downloads.all.length,
      current: props.selectedCategoryId == null,
    },
    ...categories.list.map((cat) => ({
      key: cat.id,
      id: cat.id,
      label: cat.name,
      icon: iconForCategory(cat),
      count: counts.get(cat.id) ?? 0,
      current: props.selectedCategoryId === cat.id,
    })),
  ];
});

function countLabel(n: number): string {
  return props.loading ? "—" : String(n);
}

function itemClass(item: NavItem): string {
  if (dragOverKey.value === item.key) {
    return "bg-dl-accent-soft text-foreground ring-2 ring-inset ring-primary";
  }
  if (item.current) {
    return "bg-dl-accent-soft font-semibold text-foreground shadow-[inset_0_0_0_1px_var(--dl-accent-ring)]";
  }
  return `${item.count === 0 ? "text-dl-dim" : "text-dl-soft"} hover:bg-accent`;
}

// --- Drag-to-recategorize -------------------------------------------------
//
// DownloadRow ships a JSON array of download ids under the
// `application/x-unduhin-ids` MIME on drag-start. Each library item accepts
// the drop; "All downloads" clears the assignment.

const DRAG_MIME = "application/x-unduhin-ids";
const toast = useToast();
const dragOverKey = ref<CategoryId | "all" | null>(null);

function hasUnduhinPayload(e: DragEvent): boolean {
  return e.dataTransfer?.types.includes(DRAG_MIME) ?? false;
}

function readIds(e: DragEvent): DownloadId[] {
  const raw = e.dataTransfer?.getData(DRAG_MIME);
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((x): x is number => typeof x === "number");
  } catch {
    return [];
  }
}

function onDragOver(e: DragEvent, item: NavItem) {
  if (!hasUnduhinPayload(e)) return;
  e.preventDefault();
  if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
  dragOverKey.value = item.key;
}

function onDragLeave(item: NavItem) {
  if (dragOverKey.value === item.key) dragOverKey.value = null;
}

async function onDrop(e: DragEvent, item: NavItem) {
  if (!hasUnduhinPayload(e)) return;
  e.preventDefault();
  dragOverKey.value = null;

  // Skip rows already in the target.
  const candidates = readIds(e).filter((id) => {
    const rec = downloads.records.get(id);
    return rec ? rec.category_id !== item.id : false;
  });
  if (candidates.length === 0) return;

  const settled = await Promise.allSettled(
    candidates.map((id) => downloads.setCategory(id, item.id)),
  );
  const ok = settled.filter((s) => s.status === "fulfilled").length;
  const failed = settled.length - ok;
  const action =
    item.id == null
      ? t("notify.uncategorizedAction")
      : t("notify.movedToAction", { category: item.label });
  if (failed === 0) {
    toast.push(t("downloads.batchMoved", { n: ok, category: item.label }, ok), "success");
  } else if (ok === 0) {
    toast.push(t("downloads.batchFailed", { action, n: failed }), "error");
  } else {
    toast.push(t("downloads.batchMixed", { action, ok, failed }), "error");
  }
}

const FOCUS_RING =
  "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
</script>

<template>
  <aside
    :aria-label="t('downloads.sidebarLabel')"
    class="flex h-full shrink-0 flex-col border-r border-dl-line bg-dl-chrome"
    :class="rail ? 'w-14 items-center gap-3.5 px-2 py-3' : 'w-[232px] gap-5 px-3 pb-3.5 pt-4'"
  >
    <div class="flex flex-col gap-1.5">
      <button
        type="button"
        class="flex h-10 items-center gap-2 rounded-[10px] bg-dl-accent font-semibold text-dl-on-accent transition-[filter] hover:brightness-95"
        :class="[FOCUS_RING, rail ? 'w-10 justify-center' : 'px-3']"
        :aria-label="rail ? `${t('downloads.newDownload')} (${newShortcut})` : undefined"
        :title="rail ? `${t('downloads.newDownload')} (${newShortcut})` : undefined"
        @click="emit('add-url')"
      >
        <Plus :class="rail ? 'h-[18px] w-[18px]' : 'h-4 w-4'" aria-hidden="true" />
        <template v-if="!rail">
          <span class="flex-1 text-left">{{ t("downloads.newDownload") }}</span>
          <kbd class="text-[11px] opacity-75">{{ newShortcut }}</kbd>
        </template>
      </button>
      <p v-if="!rail" class="px-1 text-xs text-muted-foreground">
        {{ t("downloads.newDownloadHint") }}
      </p>
    </div>

    <nav
      :aria-label="t('downloads.sidebarLibrary')"
      class="-mx-1 -my-0.5 flex min-h-0 shrink flex-col overflow-y-auto px-1 py-0.5"
      :class="rail ? 'gap-1' : 'gap-0.5'"
    >
      <p
        v-if="!rail"
        class="mb-1.5 ml-2.5 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground"
      >
        {{ t("downloads.sidebarLibrary") }}
      </p>
      <button
        v-for="item in items"
        :key="item.key"
        type="button"
        class="flex shrink-0 items-center rounded-lg transition-colors"
        :class="[
          FOCUS_RING,
          itemClass(item),
          rail ? 'h-10 w-10 justify-center' : 'h-9 gap-2.5 px-2.5',
        ]"
        :aria-current="item.current ? 'page' : undefined"
        :aria-label="rail ? `${item.label}, ${countLabel(item.count)}` : undefined"
        :title="rail ? `${item.label} · ${countLabel(item.count)}` : undefined"
        @click="emit('select-category', item.id)"
        @dragenter="onDragOver($event, item)"
        @dragover="onDragOver($event, item)"
        @dragleave="onDragLeave(item)"
        @drop="onDrop($event, item)"
      >
        <component
          :is="item.icon"
          class="pointer-events-none shrink-0"
          :class="rail ? 'h-[18px] w-[18px]' : 'h-[17px] w-[17px]'"
          aria-hidden="true"
        />
        <template v-if="!rail">
          <span class="pointer-events-none min-w-0 flex-1 truncate text-left">
            {{ item.label }}
          </span>
          <span
            class="pointer-events-none px-[7px] py-px font-mono text-[11px]"
            :class="
              item.current
                ? 'rounded-[10px] bg-dl-accent font-medium text-dl-on-accent'
                : ''
            "
          >
            {{ countLabel(item.count) }}
          </span>
        </template>
      </button>
    </nav>

    <div
      class="mt-auto flex flex-col"
      :class="
        rail
          ? 'items-center gap-2'
          : '-mx-3 gap-1 border-t border-dl-line px-3 pt-3'
      "
    >
      <span
        v-if="rail"
        role="img"
        :aria-label="bridgeLabel"
        :title="bridgeLabel"
        class="h-2 w-2 shrink-0 rounded-full"
        :class="bridgeReady ? 'bg-success' : 'border-[1.5px] border-muted-foreground'"
      />
      <template v-else>
        <div class="flex h-8 items-center gap-2 px-2.5 text-xs text-muted-foreground">
          <span
            class="h-2 w-2 shrink-0 rounded-full"
            :class="bridgeReady ? 'bg-success' : 'border-[1.5px] border-muted-foreground'"
            aria-hidden="true"
          />
          <span class="truncate">{{ bridgeLabel }}</span>
        </div>
        <div class="flex h-8 items-center gap-2 px-2.5 text-xs text-muted-foreground">
          <HardDrive class="h-[15px] w-[15px] shrink-0" aria-hidden="true" />
          <span class="truncate">{{ diskLine }}</span>
        </div>
      </template>
      <RouterLink
        to="/settings"
        class="flex items-center rounded-lg text-foreground transition-colors hover:bg-accent"
        :class="[FOCUS_RING, rail ? 'h-10 w-10 justify-center' : 'h-9 gap-2.5 px-2.5']"
        :aria-label="rail ? t('downloads.settings') : undefined"
        :title="rail ? t('downloads.settings') : undefined"
      >
        <Settings
          :class="rail ? 'h-[18px] w-[18px]' : 'h-[17px] w-[17px]'"
          aria-hidden="true"
        />
        <template v-if="!rail">{{ t("downloads.settings") }}</template>
      </RouterLink>
    </div>
  </aside>
</template>
