<script setup lang="ts">
// The facts list at the bottom of Overview: where the file came from,
// where it is going, its size, category and checksum. Links are plain text
// with a copy button; each fact shows once.

import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { open as openFolderDialog } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Copy } from "lucide-vue-next";

import Button from "@/components/ui/Button.vue";

import { useCategoriesStore } from "@/stores/categories";
import { useDownloadsStore } from "@/stores/downloads";
import { useTimelineText } from "@/composables/useTimelineText";
import { useToast } from "@/composables/useToast";
import { ownerOfExtension } from "@/lib/categoryMatch";
import { formatAdded, formatBytes, hostOf } from "@/lib/format";
import { extensionOf } from "@/lib/linkInput";
import { api, type DownloadRecord } from "@/types/tauri-bindings";

const props = defineProps<{ download: DownloadRecord }>();

const { t, locale } = useI18n();
const router = useRouter();
const categories = useCategoriesStore();
const downloads = useDownloadsStore();
const toast = useToast();
const { sourceLabel } = useTimelineText();

const isTorrent = computed(() => props.download.kind === "torrent");
const isCompleted = computed(() => props.download.status === "completed");
const isRunning = computed(
  () => props.download.status === "active" || props.download.status === "muxing",
);

/** The folder a file lives in; a torrent's path already is its folder. */
const folder = computed(() => {
  const path = props.download.output_path;
  if (isTorrent.value) return path;
  const cut = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return cut > 0 ? path.slice(0, cut) : path;
});

const folderLabel = computed(() => {
  if (isCompleted.value) return t("detail.savedTo");
  return props.download.status === "queued" ? t("detail.willSaveTo") : t("detail.savingTo");
});

/** A finished download shows the file itself; others show the folder. */
const pathShown = computed(() =>
  isCompleted.value ? props.download.output_path : folder.value,
);

const exactBytes = computed(() => {
  const bytes = props.download.total_bytes;
  return bytes == null
    ? ""
    : t("detail.bytesSuffix", { bytes: Number(bytes).toLocaleString(locale.value) });
});

const categoryName = computed(() => categories.nameOf(props.download.category_id));
const extension = computed(() => extensionOf(props.download.filename));
const extensionOwner = computed(() =>
  extension.value ? ownerOfExtension(extension.value, categories.list) : null,
);
/** "Owns .mp4" when this download's category is the one that files it. */
const categoryNote = computed(() => {
  if (!extension.value || isTorrent.value) return "";
  if (extensionOwner.value?.id === props.download.category_id) {
    return t("detail.categoryOwns", { extension: `.${extension.value}` });
  }
  return extensionOwner.value
    ? ""
    : t("detail.categoryNoOwner", { extension: `.${extension.value}` });
});

const addedLine = computed(() => {
  const ms = Date.parse(props.download.created_at);
  if (Number.isNaN(ms)) return t("common.dash");
  const day = formatAdded(props.download.created_at, t("downloads.addedYesterday"), locale.value);
  const clock = new Date(ms).toLocaleTimeString(locale.value, {
    hour: "2-digit",
    minute: "2-digit",
  });
  // Today's "day" is already a clock time.
  return day.includes(":") ? t("detail.addedToday", { time: day }) : `${day}, ${clock}`;
});

async function copy(text: string, doneKey: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast.push(t(doneKey), "success");
  } catch {
    toast.push(t("detail.copyFailed"), "error");
  }
}

async function changeFolder() {
  const picked = await openFolderDialog({
    directory: true,
    multiple: false,
    defaultPath: folder.value || undefined,
  });
  if (typeof picked !== "string") return;
  try {
    downloads.putRecord(await api.moveDownload(props.download.id, picked));
  } catch (e: unknown) {
    const message = (e as { message?: string })?.message ?? String(e);
    toast.push(t("errors.moveDownload", { error: message }), "error");
  }
}

async function showFile() {
  try {
    await revealItemInDir(props.download.output_path);
  } catch {
    toast.push(t("errors.revealFile"), "error");
  }
}

const hashing = ref(false);

async function calculateChecksum() {
  hashing.value = true;
  try {
    const sha256 = await api.computeChecksum(props.download.id);
    downloads.putRecord({ ...props.download, sha256 });
  } catch (e: unknown) {
    const message = (e as { message?: string })?.message ?? String(e);
    toast.push(t("detail.checksumFailed", { error: message }), "error");
  } finally {
    hashing.value = false;
  }
}

const ROW = "flex min-w-0 flex-col gap-1 pt-2.5";
const TERM = "text-xs text-muted-foreground";
const SUB = "text-xs leading-[1.45] text-muted-foreground";
const ICON_BUTTON =
  "flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
</script>

<template>
  <dl class="-mx-5 flex flex-col gap-3.5 border-t border-dl-line px-5 pt-1">
    <div :class="ROW">
      <dt :class="TERM">{{ t("detail.sourceLink") }}</dt>
      <dd class="flex min-w-0 items-center gap-2">
        <span class="min-w-0 flex-1 truncate font-mono text-xs" :title="download.url">
          {{ download.url }}
        </span>
        <button
          type="button"
          :class="ICON_BUTTON"
          :aria-label="t('detail.copyLink')"
          :title="t('detail.copyLink')"
          @click="copy(download.url, 'detail.linkCopied')"
        >
          <Copy class="h-3.5 w-3.5" aria-hidden="true" />
        </button>
      </dd>
    </div>

    <div :class="ROW">
      <dt :class="TERM">{{ folderLabel }}</dt>
      <dd class="flex min-w-0 items-center gap-2">
        <span class="min-w-0 flex-1 truncate font-mono text-xs" :title="pathShown">
          {{ pathShown }}
        </span>
        <Button v-if="isCompleted" size="xs" variant="secondary" class="bg-transparent" @click="showFile">
          {{ t("detail.show") }}
        </Button>
        <Button
          v-else-if="!isTorrent"
          size="xs"
          variant="secondary"
          class="bg-transparent"
          :disabled="isRunning"
          :title="isRunning ? t('detail.changeFolderRunning') : undefined"
          @click="changeFolder"
        >
          {{ t("detail.change") }}
        </Button>
      </dd>
    </div>

    <div :class="ROW">
      <dt :class="TERM">{{ t("detail.metricSize") }}</dt>
      <dd class="flex min-w-0 flex-col gap-0.5">
        <span>{{ download.total_bytes == null ? t("detail.sizeUnknown") : formatBytes(download.total_bytes) }}</span>
        <span v-if="exactBytes" :class="SUB">{{ exactBytes }}</span>
      </dd>
    </div>

    <div :class="ROW">
      <dt :class="TERM">{{ t("detail.metricCategory") }}</dt>
      <dd class="flex min-w-0 flex-col gap-0.5">
        <span>{{ categoryName }}</span>
        <span v-if="categoryNote" :class="SUB">
          {{ categoryNote }}
          <button
            v-if="!extensionOwner"
            type="button"
            class="rounded font-medium text-primary underline-offset-2 hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
            @click="router.push({ name: 'settings-categories' })"
          >
            {{ t("detail.categoryAddIt") }}
          </button>
        </span>
      </dd>
    </div>

    <div v-if="isCompleted && !isTorrent" :class="ROW">
      <dt :class="TERM">{{ t("detail.checksum") }}</dt>
      <dd class="flex min-w-0 items-center gap-2">
        <template v-if="download.sha256">
          <span class="min-w-0 flex-1 break-all font-mono text-xs">{{ download.sha256 }}</span>
          <button
            type="button"
            :class="ICON_BUTTON"
            :aria-label="t('detail.copyChecksum')"
            :title="t('detail.copyChecksum')"
            @click="copy(download.sha256, 'detail.checksumCopied')"
          >
            <Copy class="h-3.5 w-3.5" aria-hidden="true" />
          </button>
        </template>
        <template v-else>
          <span class="min-w-0 flex-1" :class="SUB">
            {{ hashing ? t("detail.checksumWorking") : t("detail.checksumNone") }}
          </span>
          <Button
            size="xs"
            variant="secondary"
            class="bg-transparent"
            :disabled="hashing"
            @click="calculateChecksum"
          >
            {{ t("detail.checksumCalculate") }}
          </Button>
        </template>
      </dd>
    </div>

    <div :class="ROW">
      <dt :class="TERM">{{ t("detail.added") }}</dt>
      <dd class="flex min-w-0 flex-col gap-0.5">
        <span>{{ addedLine }}</span>
        <span :class="SUB">{{ sourceLabel(download.source) }}</span>
      </dd>
    </div>

    <div v-if="!isTorrent && hostOf(download.url)" :class="ROW">
      <dt :class="TERM">{{ t("detail.server") }}</dt>
      <dd>{{ hostOf(download.url) }}</dd>
    </div>
  </dl>
</template>
