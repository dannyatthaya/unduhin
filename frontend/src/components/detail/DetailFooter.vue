<script setup lang="ts">
// The pinned footer of the details panel: the one action that fits the
// download's state, Show in folder, and the ••• menu with everything else.
// The menu's shortcuts work while focus is anywhere in the panel
// (DetailPane forwards its keydown to `handleKey`).

import { computed, type Component } from "vue";
import { useI18n } from "vue-i18n";
import {
  AppWindow,
  CalendarClock,
  Copy,
  ExternalLink,
  Folder,
  FolderInput,
  Link,
  MoreHorizontal,
  Pause,
  Pencil,
  Play,
  RotateCw,
  Trash2,
  X,
} from "lucide-vue-next";
import { open as openFolderDialog } from "@tauri-apps/plugin-dialog";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";

import Button from "@/components/ui/Button.vue";
import RowMenu, { type RowMenuItem } from "@/components/RowMenu.vue";

import { useDownloadsStore } from "@/stores/downloads";
import { useSchedulesStore } from "@/stores/schedules";
import { useDeleteConfirm } from "@/composables/useDeleteConfirm";
import { useRenameDialog } from "@/composables/useRenameDialog";
import { useToast } from "@/composables/useToast";
import { api, type DownloadRecord } from "@/types/tauri-bindings";

const props = defineProps<{ download: DownloadRecord }>();
const emit = defineEmits<{ "paste-link": []; schedule: [] }>();

const { t } = useI18n();
const toast = useToast();
const downloads = useDownloadsStore();
const schedules = useSchedulesStore();
const deleteConfirm = useDeleteConfirm();
const rename = useRenameDialog();

const IS_WINDOWS = navigator.userAgent.includes("Windows");

const status = computed(() => props.download.status);
const isTorrent = computed(() => props.download.kind === "torrent");
const isRunning = computed(() => status.value === "active" || status.value === "muxing");
const isCompleted = computed(() => status.value === "completed");
const isExpiredLink = computed(
  () =>
    status.value === "failed" &&
    props.download.error_kind === "expired_auth" &&
    props.download.kind === "http",
);
const ownSchedules = computed(() => schedules.byDownload.get(props.download.id) ?? []);
/** Renaming or moving a file something is writing to would break it. */
const canChangeFile = computed(() => !isRunning.value && !isTorrent.value);

function messageOf(e: unknown): string {
  return (e as { message?: string })?.message ?? String(e);
}

/** Run a store action; a failure becomes a toast instead of a silent no-op. */
async function attempt(action: () => Promise<unknown>, errorKey?: string): Promise<void> {
  try {
    await action();
  } catch (e: unknown) {
    const message = messageOf(e);
    toast.push(errorKey ? t(errorKey, { error: message }) : message || t("downloads.actionFailed"), "error");
  }
}

const retry = () => attempt(() => downloads.retry(props.download.id), "errors.restartDownload");

async function startNow(): Promise<void> {
  await attempt(async () => {
    for (const row of ownSchedules.value) await schedules.remove(row.id);
  });
}

async function openFile(): Promise<void> {
  try {
    await openPath(props.download.output_path);
  } catch {
    toast.push(t("errors.openFile", { filename: props.download.filename }), "error");
  }
}

async function showInFolder(): Promise<void> {
  try {
    await revealItemInDir(props.download.output_path);
  } catch {
    toast.push(t("errors.revealFile"), "error");
  }
}

async function copy(text: string, doneKey: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    toast.push(t(doneKey), "success");
  } catch {
    toast.push(t("detail.copyFailed"), "error");
  }
}

const copyPath = () => copy(props.download.output_path, "detail.pathCopied");

async function moveTo(): Promise<void> {
  const picked = await openFolderDialog({ directory: true, multiple: false });
  if (typeof picked !== "string") return;
  await attempt(
    async () => downloads.putRecord(await api.moveDownload(props.download.id, picked)),
    "errors.moveDownload",
  );
}

const removeFromList = () => deleteConfirm.removeFromList([props.download.id]);
const deleteFile = () => deleteConfirm.requestDelete([props.download.id], { alwaysAsk: true });

interface FooterAction {
  label: string;
  icon: Component;
  run: () => void;
}

const primary = computed<FooterAction | null>(() => {
  const id = props.download.id;
  switch (status.value) {
    case "active":
    case "muxing":
      return { label: t("detail.actionPause"), icon: Pause, run: () => void attempt(() => downloads.pause(id)) };
    case "queued":
      // A download held back by a start time starts once the time is gone.
      return ownSchedules.value.length > 0
        ? { label: t("detail.startNow"), icon: Play, run: () => void startNow() }
        : { label: t("detail.actionPause"), icon: Pause, run: () => void attempt(() => downloads.pause(id)) };
    case "paused":
      return { label: t("detail.actionResume"), icon: Play, run: () => void attempt(() => downloads.resume(id)) };
    case "failed":
      return isExpiredLink.value
        ? { label: t("detail.pasteNewLink"), icon: Link, run: () => emit("paste-link") }
        : { label: t("downloads.rowRetry"), icon: RotateCw, run: () => void retry() };
    case "cancelled":
      return { label: t("downloads.rowRetry"), icon: RotateCw, run: () => void retry() };
    case "completed":
      return { label: t("downloads.rowOpen"), icon: ExternalLink, run: () => void openFile() };
    default:
      return null;
  }
});

/** Nothing is on disk to show until a queued download has started. */
const canShowInFolder = computed(() => status.value !== "queued" && status.value !== "cancelled");

const menu = computed<RowMenuItem[]>(() => {
  const r = props.download;
  const items: RowMenuItem[] = [];
  if (isCompleted.value && IS_WINDOWS && !isTorrent.value) {
    items.push({
      label: t("detail.menuOpenWith"),
      icon: AppWindow,
      onSelect: () => void attempt(() => api.openWith(r.id), "errors.openWith"),
    });
  }
  items.push({
    label: t("detail.copyLink"),
    icon: Link,
    onSelect: () => void copy(r.url, "detail.linkCopied"),
  });
  items.push({
    label: t("detail.menuCopyPath"),
    icon: Copy,
    shortcut: "Ctrl Shift C",
    onSelect: () => void copyPath(),
  });
  if (canChangeFile.value) {
    items.push({
      label: t("detail.menuRename"),
      icon: Pencil,
      shortcut: "F2",
      onSelect: () => rename.request(r.id),
    });
    items.push({ label: t("detail.menuMoveTo"), icon: FolderInput, onSelect: () => void moveTo() });
  }
  if (status.value === "queued" || status.value === "paused") {
    items.push({
      label: t("downloads.menuSchedule"),
      icon: CalendarClock,
      onSelect: () => emit("schedule"),
    });
  }
  if (isCompleted.value) {
    items.push({ label: t("detail.menuDownloadAgain"), icon: RotateCw, onSelect: () => void retry() });
  }
  items.push({
    label: t("downloads.menuRemoveFromList"),
    icon: X,
    shortcut: "Del",
    separatorBefore: true,
    onSelect: () => void removeFromList(),
  });
  items.push({
    label: t("detail.menuDeleteFile"),
    icon: Trash2,
    shortcut: "Shift Del",
    danger: true,
    onSelect: () => void deleteFile(),
  });
  return items;
});

function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

/** The menu's shortcuts. Returns true when the key was used. */
function handleKey(e: KeyboardEvent): boolean {
  if (isTyping(e.target)) return false;
  if (e.key === "F2" && canChangeFile.value) {
    rename.request(props.download.id);
    return true;
  }
  if (e.key.toLowerCase() === "c" && e.ctrlKey && e.shiftKey) {
    void copyPath();
    return true;
  }
  if (e.key === "Delete") {
    void (e.shiftKey ? deleteFile() : removeFromList());
    return true;
  }
  return false;
}

defineExpose({ handleKey });
</script>

<template>
  <footer class="flex shrink-0 items-center gap-2 border-t border-dl-line py-3 pl-5 pr-4">
    <Button v-if="primary" class="gap-1.5" @click="primary.run">
      <component :is="primary.icon" class="h-[15px] w-[15px]" aria-hidden="true" />
      {{ primary.label }}
    </Button>
    <Button v-if="canShowInFolder" variant="ghost" class="gap-1.5 px-3" @click="showInFolder">
      <Folder class="h-[15px] w-[15px]" aria-hidden="true" />
      {{ t("downloads.rowShowInFolder") }}
    </Button>
    <span class="ml-auto flex">
      <RowMenu :items="menu">
        <MoreHorizontal class="h-[18px] w-[18px]" aria-hidden="true" />
      </RowMenu>
    </span>
  </footer>
</template>
