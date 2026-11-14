<script setup lang="ts">
// Replaces the toolbar while rows are selected — same height, so the list
// underneath doesn't jump. Every action runs on the whole selection and
// only on the rows it makes sense for.

import { computed, type Component } from "vue";
import { useI18n } from "vue-i18n";
import {
  ExternalLink,
  Folder,
  FolderInput,
  Link,
  Trash2,
  X,
} from "lucide-vue-next";
import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPortal,
  DropdownMenuRoot,
  DropdownMenuTrigger,
} from "reka-ui";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";

import { useCategoriesStore } from "@/stores/categories";
import { useDownloadsStore } from "@/stores/downloads";
import { useSelectionStore } from "@/stores/selection";
import { useDeleteConfirm } from "@/composables/useDeleteConfirm";
import { useToast } from "@/composables/useToast";
import type { Category, DownloadId, DownloadRecord } from "@/types/tauri-bindings";

export interface SelectScope {
  /** Section name, or null when the scope is simply every listed row. */
  label: string | null;
  ids: DownloadId[];
}

defineProps<{
  /** "Select all N in …" target; null when everything is already selected. */
  scope: SelectScope | null;
}>();

const emit = defineEmits<{ "select-scope": [] }>();

/** Opening launches one external app per file — keep that bounded. */
const MAX_BULK_OPEN = 10;

const { t } = useI18n();
const downloads = useDownloadsStore();
const selection = useSelectionStore();
const categories = useCategoriesStore();
const deleteConfirm = useDeleteConfirm();
const toast = useToast();

const selectedIds = computed<DownloadId[]>(() => Array.from(selection.ids));
const selected = computed<DownloadRecord[]>(() =>
  selectedIds.value
    .map((id) => downloads.records.get(id))
    .filter((r): r is DownloadRecord => r != null),
);
const finished = computed(() => selected.value.filter((r) => r.status === "completed"));
const canOpen = computed(
  () => finished.value.length > 0 && finished.value.length <= MAX_BULK_OPEN,
);
const openHint = computed(() =>
  finished.value.length > MAX_BULK_OPEN
    ? t("downloads.bulkOpenTooMany", { n: MAX_BULK_OPEN })
    : undefined,
);

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

async function openFiles() {
  if (!canOpen.value) return;
  const results = await Promise.allSettled(
    finished.value.map((r) => openPath(r.output_path)),
  );
  const failed = results.filter((r) => r.status === "rejected").length;
  if (failed > 0) {
    toast.push(
      t("downloads.batchFailed", { action: t("downloads.bulkOpen"), n: failed }),
      "error",
    );
  }
}

async function showInFolder() {
  const paths = selected.value.map((r) => r.output_path).filter(Boolean);
  if (paths.length === 0) return;
  try {
    await revealItemInDir(paths);
  } catch (e: unknown) {
    toast.push(`${t("errors.revealFile")} ${errorText(e)}`, "error");
  }
}

async function moveTo(category: Category) {
  const targets = selected.value.filter((r) => r.category_id !== category.id);
  if (targets.length === 0) return;
  const settled = await Promise.allSettled(
    targets.map((r) => downloads.setCategory(r.id, category.id)),
  );
  const ok = settled.filter((s) => s.status === "fulfilled").length;
  const failed = settled.length - ok;
  if (failed === 0) {
    toast.push(
      t("downloads.batchMoved", { n: ok, category: category.name }, ok),
      "success",
    );
    return;
  }
  const action = t("notify.movedToAction", { category: category.name });
  toast.push(
    ok === 0
      ? t("downloads.batchFailed", { action, n: failed })
      : t("downloads.batchMixed", { action, ok, failed }),
    "error",
  );
}

async function copyLinks() {
  const links = selected.value.map((r) => r.url);
  try {
    await navigator.clipboard.writeText(links.join("\n"));
    toast.push(t("downloads.linksCopied", { n: links.length }, links.length), "success");
  } catch {
    toast.push(t("detail.copyFailed"), "error");
  }
}

async function removeFromList() {
  await deleteConfirm.removeFromList(selectedIds.value);
}

async function deleteFiles() {
  await deleteConfirm.requestDelete(selectedIds.value, { alwaysAsk: true });
}

interface BulkAction {
  key: string;
  label: string;
  icon: Component;
  run: () => Promise<void>;
  disabled?: boolean;
  title?: string;
  danger?: boolean;
}

const leadingActions = computed<BulkAction[]>(() => [
  {
    key: "open",
    label: t("downloads.bulkOpen"),
    icon: ExternalLink,
    run: openFiles,
    disabled: !canOpen.value,
    title: openHint.value,
  },
  {
    key: "folder",
    label: t("downloads.rowShowInFolder"),
    icon: Folder,
    run: showInFolder,
  },
]);

const trailingActions = computed<BulkAction[]>(() => [
  { key: "copy", label: t("downloads.bulkCopyLinks"), icon: Link, run: copyLinks },
  {
    key: "remove",
    label: t("downloads.menuRemoveFromList"),
    icon: X,
    run: removeFromList,
  },
  {
    key: "delete",
    label: t("downloads.bulkDeleteFiles"),
    icon: Trash2,
    run: deleteFiles,
    danger: true,
  },
]);

const BUTTON =
  "flex h-8 items-center gap-1.5 whitespace-nowrap rounded-lg border px-2.5 text-xs font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:cursor-default disabled:opacity-45";
const NEUTRAL = "border-border bg-card text-foreground enabled:hover:bg-accent";
const DANGER = "border-danger bg-transparent text-danger hover:bg-dl-fail-soft";
</script>

<template>
  <div
    role="toolbar"
    :aria-label="t('downloads.bulkToolbarAria')"
    class="flex h-14 shrink-0 items-center gap-2 border-b border-dl-accent-ring bg-dl-bulk px-5"
  >
    <button
      type="button"
      class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-foreground transition-colors hover:bg-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
      :aria-label="t('downloads.clearSelection')"
      :title="t('downloads.clearSelectionHint')"
      @click="selection.clear()"
    >
      <X class="h-[15px] w-[15px]" aria-hidden="true" />
    </button>
    <span aria-live="polite" class="whitespace-nowrap font-semibold">
      {{ t("downloads.batchSelected", { n: selection.count }) }}
    </span>
    <button
      v-if="scope"
      type="button"
      class="h-8 shrink truncate rounded-lg px-2 font-medium text-primary underline underline-offset-[3px] transition-colors hover:bg-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
      @click="emit('select-scope')"
    >
      {{
        scope.label
          ? t("downloads.selectAllIn", { n: scope.ids.length, group: scope.label })
          : t("downloads.selectAllN", { n: scope.ids.length })
      }}
    </button>

    <div class="ml-auto flex shrink-0 items-center justify-end gap-1.5">
      <button
        v-for="action in leadingActions"
        :key="action.key"
        type="button"
        :class="[BUTTON, NEUTRAL]"
        :disabled="action.disabled"
        :title="action.title"
        @click="action.run()"
      >
        <component :is="action.icon" class="h-[15px] w-[15px]" aria-hidden="true" />
        {{ action.label }}
      </button>

      <DropdownMenuRoot>
        <DropdownMenuTrigger as-child>
          <button type="button" :class="[BUTTON, NEUTRAL]">
            <FolderInput class="h-[15px] w-[15px]" aria-hidden="true" />
            {{ t("downloads.batchMoveTo") }}
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuPortal>
          <DropdownMenuContent
            align="end"
            :side-offset="6"
            class="dl-surface z-50 min-w-[12rem] overflow-hidden rounded-lg border border-border bg-card p-1 text-card-foreground shadow-md data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0"
          >
            <DropdownMenuItem
              v-for="category in categories.list"
              :key="category.id"
              class="flex cursor-default select-none items-center rounded-md px-2.5 py-1.5 outline-none data-[highlighted]:bg-accent"
              @select="moveTo(category)"
            >
              {{ category.name }}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenuPortal>
      </DropdownMenuRoot>

      <button
        v-for="action in trailingActions"
        :key="action.key"
        type="button"
        :class="[BUTTON, action.danger ? DANGER : NEUTRAL]"
        @click="action.run()"
      >
        <component :is="action.icon" class="h-[15px] w-[15px]" aria-hidden="true" />
        {{ action.label }}
      </button>
    </div>
  </div>
</template>
