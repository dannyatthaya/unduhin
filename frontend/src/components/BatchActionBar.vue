<script setup lang="ts">
// Floating selection toolbar. Renders only when there's an active
// multi-select. The bar pins to the bottom of the main content area —
// styling matches the dark pill in the multi-select screenshot in both
// themes (background uses card-foreground with luminance flipped).
//
// All actions iterate over the selected ids in parallel and emit a
// single toast per batch with a counts summary (success vs. failed).

import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import {
  Folder,
  FolderTree,
  Pause,
  Play,
  RotateCw,
  Trash2,
  X,
  XCircle,
} from "lucide-vue-next";

import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPortal,
  DropdownMenuRoot,
  DropdownMenuTrigger,
} from "reka-ui";

import { useCategoriesStore } from "@/stores/categories";
import { useDownloadsStore } from "@/stores/downloads";
import { useSelectionStore } from "@/stores/selection";
import { useDeleteConfirm } from "@/composables/useDeleteConfirm";
import { useToast } from "@/composables/useToast";
import type {
  Category,
  DownloadId,
  DownloadRecord,
} from "@/types/tauri-bindings";

const { t } = useI18n();
const selection = useSelectionStore();
const downloads = useDownloadsStore();
const categories = useCategoriesStore();
const deleteConfirm = useDeleteConfirm();
const toast = useToast();

const isMovingTo = ref(false);

const selectedRecords = computed<DownloadRecord[]>(() => {
  const out: DownloadRecord[] = [];
  for (const id of selection.ids) {
    const r = downloads.records.get(id);
    if (r) out.push(r);
  }
  return out;
});

const canPause = computed(() =>
  selectedRecords.value.some(
    (r) => r.status === "active" || r.status === "queued",
  ),
);

const canResume = computed(() =>
  selectedRecords.value.some(
    (r) => r.status === "paused" || r.status === "failed",
  ),
);

const canCancel = computed(() =>
  selectedRecords.value.some((r) =>
    ["active", "queued", "paused", "muxing"].includes(r.status),
  ),
);

const canRetry = computed(() =>
  selectedRecords.value.some(
    (r) => r.status === "failed" || r.status === "cancelled",
  ),
);

/**
 * Run an action against each id that passes `filter`, in parallel.
 * Reports one combined toast with the OK / failed counts so the
 * batch-bar surface stays quiet for big multi-selects — one toast
 * per batch with a counts summary.
 */
async function runBatch(
  successKey: string,
  actionLabel: string,
  filter: (r: DownloadRecord) => boolean,
  action: (id: DownloadId) => Promise<unknown>,
  extraNamed: Record<string, unknown> = {},
): Promise<void> {
  const targets = selectedRecords.value.filter(filter);
  if (targets.length === 0) return;
  const settled = await Promise.allSettled(targets.map((r) => action(r.id)));
  const ok = settled.filter((s) => s.status === "fulfilled").length;
  const failed = settled.length - ok;
  if (failed === 0) {
    toast.push(
      t(successKey, { n: ok, ...extraNamed }, ok),
      "success",
    );
  } else if (ok === 0) {
    toast.push(
      t("downloads.batchFailed", { action: actionLabel, n: failed }),
      "error",
    );
  } else {
    toast.push(
      t("downloads.batchMixed", { action: actionLabel, ok, failed }),
      "error",
    );
  }
}

const pauseSelected = () =>
  runBatch(
    "downloads.batchPaused",
    t("downloads.batchPause"),
    (r) => r.status === "active" || r.status === "queued",
    (id) => downloads.pause(id),
  );

const resumeSelected = () =>
  runBatch(
    "downloads.batchResumed",
    t("downloads.batchResume"),
    (r) => r.status === "paused" || r.status === "failed",
    (id) => downloads.resume(id),
  );

const cancelSelected = () =>
  runBatch(
    "downloads.batchCancelled",
    t("downloads.batchCancel"),
    (r) => ["active", "queued", "paused", "muxing"].includes(r.status),
    (id) => downloads.cancel(id),
  );

const retrySelected = () =>
  runBatch(
    "downloads.batchRetried",
    t("downloads.batchRetry"),
    (r) => r.status === "failed" || r.status === "cancelled",
    (id) => downloads.retry(id),
  );

async function moveSelectedTo(category: Category) {
  isMovingTo.value = false;
  // Anything already in the target category is silently skipped — moving
  // a row to its current category is a no-op and shouldn't pollute the
  // counts summary.
  await runBatch(
    "downloads.batchMoved",
    t("notify.movedToAction", { category: category.name }),
    (r) => r.category_id !== category.id,
    (id) => downloads.setCategory(id, category.id),
    { category: category.name },
  );
}

async function removeSelected() {
  await deleteConfirm.requestDelete(selectedRecords.value.map((r) => r.id));
  selection.clear();
}
</script>

<template>
  <Transition
    enter-active-class="transition duration-150 ease-out"
    leave-active-class="transition duration-150 ease-in"
    enter-from-class="opacity-0 translate-y-2"
    leave-to-class="opacity-0 translate-y-2"
  >
    <div
      v-if="!selection.empty"
      class="pointer-events-none absolute inset-x-0 bottom-4 z-30 flex justify-center"
    >
      <div
        class="pointer-events-auto flex items-center gap-1 rounded-full border border-border bg-foreground/95 px-2 py-1.5 text-background shadow-lg backdrop-blur"
      >
        <span class="px-2 text-sm font-medium">
          {{ t("downloads.batchSelected", { n: selection.count }) }}
        </span>
        <span class="h-5 w-px bg-background/20" />
        <button
          type="button"
          class="inline-flex h-8 items-center gap-1.5 rounded-full px-3 text-sm font-medium transition-colors hover:bg-background/15 disabled:opacity-40"
          :disabled="!canPause"
          @click="pauseSelected"
        >
          <Pause class="h-3.5 w-3.5" />
          {{ t("downloads.batchPause") }}
        </button>
        <button
          type="button"
          class="inline-flex h-8 items-center gap-1.5 rounded-full px-3 text-sm font-medium transition-colors hover:bg-background/15 disabled:opacity-40"
          :disabled="!canResume"
          @click="resumeSelected"
        >
          <Play class="h-3.5 w-3.5" />
          {{ t("downloads.batchResume") }}
        </button>
        <button
          type="button"
          class="inline-flex h-8 items-center gap-1.5 rounded-full px-3 text-sm font-medium transition-colors hover:bg-background/15 disabled:opacity-40"
          :disabled="!canCancel"
          @click="cancelSelected"
        >
          <XCircle class="h-3.5 w-3.5" />
          {{ t("downloads.batchCancel") }}
        </button>
        <button
          type="button"
          class="inline-flex h-8 items-center gap-1.5 rounded-full px-3 text-sm font-medium transition-colors hover:bg-background/15 disabled:opacity-40"
          :disabled="!canRetry"
          @click="retrySelected"
        >
          <RotateCw class="h-3.5 w-3.5" />
          {{ t("downloads.batchRetry") }}
        </button>
        <DropdownMenuRoot v-model:open="isMovingTo">
          <DropdownMenuTrigger as-child>
            <button
              type="button"
              class="inline-flex h-8 items-center gap-1.5 rounded-full px-3 text-sm font-medium transition-colors hover:bg-background/15"
            >
              <FolderTree class="h-3.5 w-3.5" />
              {{ t("downloads.batchMoveTo") }}
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuPortal>
            <DropdownMenuContent
              align="center"
              :side-offset="6"
              class="z-50 min-w-[12rem] overflow-hidden rounded-md border border-border bg-card text-card-foreground shadow-lg"
            >
              <DropdownMenuItem
                v-for="c in categories.list"
                :key="c.id"
                class="flex w-full cursor-pointer items-center gap-2 px-3 py-1.5 text-sm outline-none transition-colors data-[highlighted]:bg-accent"
                @select="moveSelectedTo(c)"
              >
                <Folder class="h-3.5 w-3.5" />
                <span>{{ c.name }}</span>
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenuPortal>
        </DropdownMenuRoot>
        <button
          type="button"
          class="inline-flex h-8 items-center gap-1.5 rounded-full px-3 text-sm font-medium text-danger transition-colors hover:bg-danger/15"
          @click="removeSelected"
        >
          <Trash2 class="h-3.5 w-3.5" />
          {{ t("downloads.batchDelete") }}
        </button>
        <button
          type="button"
          class="inline-flex h-8 w-8 items-center justify-center rounded-full transition-colors hover:bg-background/15"
          :title="t('downloads.clearSelection')"
          @click="selection.clear()"
        >
          <X class="h-3.5 w-3.5" />
        </button>
      </div>
    </div>
  </Transition>
</template>
