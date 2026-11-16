<script setup lang="ts">
// New download. One column: the link and what the server says about it,
// then category, connections, folder and when to start. Enter starts the
// download from any text field.
//
// A magnet link or a `.torrent` path switches to the two-step torrent
// flow: the first confirm fetches the file list, the second adds the
// download with the files that were picked.

import { computed, ref, toRef, useId, watch } from "vue";
import { useI18n } from "vue-i18n";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  AlertTriangle,
  Clock,
  Download,
  Folder,
  FolderOpen,
  Link,
  Pencil,
} from "lucide-vue-next";

import Dialog from "./ui/Dialog.vue";
import Button from "./ui/Button.vue";
import Field from "./ui/Field.vue";
import Select, { type SelectOption } from "./ui/Select.vue";
import NumberStepper from "./settings/controls/NumberStepper.vue";
import StartPicker from "./StartPicker.vue";
import LinkProbeCard from "./add/LinkProbeCard.vue";
import TorrentFilePicker from "./add/TorrentFilePicker.vue";
import MediaInstallBanner from "./media/MediaInstallBanner.vue";
import MediaFormatsDialog from "./media/MediaFormatsDialog.vue";

import { useCategoriesStore } from "@/stores/categories";
import { useDownloadsStore } from "@/stores/downloads";
import { useToolingStatus } from "@/composables/useToolingStatus";
import { useGeneralSettings } from "@/composables/useGeneralSettings";
import { useLinkProbe } from "@/composables/useLinkProbe";
import { useStartPlan } from "@/composables/useStartPlan";
import { useToast } from "@/composables/useToast";
import {
  api,
  type DownloadRecord,
  type ProbeResult,
  type TorrentMetadataResult,
  type TorrentMeta,
} from "@/types/tauri-bindings";
import {
  detectTorrentSource,
  displayNameFromMagnet,
  selectedFileIndices,
} from "@/lib/torrentInput";
import { categoryForFileName } from "@/lib/categoryMatch";
import { splitFileName } from "@/lib/linkInput";
import { toTimeInput } from "@/lib/startPlan";
import { parentFolderLabel, relativeTime } from "@/lib/format";

/** Bounds of the connections stepper; the engine accepts 1..=32. */
const MIN_CONNECTIONS = 1;
const MAX_CONNECTIONS = 32;
const AUTO_CATEGORY = "auto";

const { t } = useI18n();
const props = defineProps<{
  open: boolean;
  /** Pre-fills the link field when the dialog opens (a link pasted on the page). */
  initialUrl?: string;
}>();
const emit = defineEmits<{ close: []; submitted: [id: number] }>();

const downloads = useDownloadsStore();
const categories = useCategoriesStore();
const { ytdlpAvailable, ffmpegAvailable } = useToolingStatus();
const generalSettings = useGeneralSettings();
const toast = useToast();

const linkId = useId();
const linkMessageId = useId();
const categoryLabelId = useId();
const connectionsId = useId();
const folderId = useId();
const nameId = useId();

const url = ref("");
const categoryChoice = ref<string>(AUTO_CATEGORY);
const segments = ref<number | null>(null);
const outputPath = ref("");
const submitting = ref(false);
const errorMessage = ref<string | null>(null);

const plan = useStartPlan();
const probe = useLinkProbe(url, toRef(props, "open"));

const showRename = ref(false);
/** The typed name without its extension; empty means "use the detected one". */
const renameStem = ref("");

const showInstallBanner = ref(false);
const probeResult = ref<ProbeResult | null>(null);
const showFormatsDialog = ref(false);

// ── Torrent path ─────────────────────────────────────────────────────────
const torrentMeta = ref<TorrentMetadataResult | null>(null);
const torrentSelected = ref<Set<number>>(new Set());
const probingTorrent = ref(false);

const isTorrentInput = computed(() => probe.kind.value === "torrent");

function resetTorrentState() {
  torrentMeta.value = null;
  torrentSelected.value = new Set();
  probingTorrent.value = false;
}

watch(
  () => props.open,
  (open) => {
    if (!open) return;
    url.value = props.initialUrl ?? "";
    categoryChoice.value = AUTO_CATEGORY;
    segments.value = null;
    outputPath.value = "";
    errorMessage.value = null;
    showInstallBanner.value = false;
    probeResult.value = null;
    showFormatsDialog.value = false;
    showRename.value = generalSettings.alwaysAskFilename.value;
    renameStem.value = "";
    plan.reset();
    resetTorrentState();
  },
);

// Editing the link invalidates a fetched torrent file list.
watch(url, () => {
  resetTorrentState();
  errorMessage.value = null;
});

// ── Name ────────────────────────────────────────────────────────────────

const detectedParts = computed(() => splitFileName(probe.detectedName.value));
const isRenamed = computed(() => showRename.value && renameStem.value.trim() !== "");
/** What the file will be called, as far as the dialog can tell. */
const effectiveName = computed(() =>
  isRenamed.value
    ? `${renameStem.value.trim()}${detectedParts.value.extension}`
    : probe.detectedName.value,
);

function openRename() {
  renameStem.value = detectedParts.value.stem;
  showRename.value = true;
}

function useDetectedName() {
  renameStem.value = "";
  showRename.value = false;
}

// The name arrives after the check; fill the field once if it is still empty.
watch(
  () => probe.detectedName.value,
  () => {
    if (showRename.value && renameStem.value === "") {
      renameStem.value = detectedParts.value.stem;
    }
  },
);

// ── Category and folder ─────────────────────────────────────────────────

const autoCategory = computed(() =>
  categoryForFileName(effectiveName.value, categories.list),
);
const isAutoCategory = computed(() => categoryChoice.value === AUTO_CATEGORY);
const chosenCategory = computed(() =>
  isAutoCategory.value
    ? autoCategory.value
    : (categories.byId.get(Number(categoryChoice.value)) ?? null),
);

const categoryOptions = computed<SelectOption<string>[]>(() => [
  {
    value: AUTO_CATEGORY,
    label: t("addUrl.categoryAuto"),
    triggerLabel:
      probe.state.value === "checking"
        ? t("addUrl.categoryAfterCheck")
        : (autoCategory.value?.name ?? t("addUrl.categoryFromLink")),
  },
  ...categories.list.map((c) => ({ value: String(c.id), label: c.name })),
]);

/** Where the file goes when the user has not picked a folder. */
const autoFolder = computed(
  () =>
    chosenCategory.value?.default_output_path ||
    generalSettings.defaultOutputPath.value,
);
const isAutoFolder = computed(() => outputPath.value.trim() === "");
const saveTo = computed({
  get: () => outputPath.value || autoFolder.value,
  set: (value: string) => {
    outputPath.value = value === autoFolder.value ? "" : value;
  },
});

async function pickFolder() {
  const picked = await openFileDialog({
    directory: true,
    multiple: false,
    defaultPath: saveTo.value || undefined,
  });
  if (typeof picked === "string") outputPath.value = picked;
}

/** Browse for a local `.torrent` file; its path goes in the link field. */
async function pickTorrentFile() {
  const picked = await openFileDialog({
    directory: false,
    multiple: false,
    filters: [{ name: "Torrent", extensions: ["torrent"] }],
  });
  if (typeof picked === "string") url.value = picked;
}

// ── Connections ─────────────────────────────────────────────────────────

const connections = computed({
  get: () => segments.value ?? generalSettings.defaultSegments.value,
  set: (value: number) => {
    segments.value = value === generalSettings.defaultSegments.value ? null : value;
  },
});

// ── Already in the list ─────────────────────────────────────────────────

const duplicate = computed<DownloadRecord | null>(() => {
  const target = url.value.trim();
  if (probe.kind.value !== "http") return null;
  const matches = downloads.all.filter((r) => r.url === target);
  if (matches.length === 0) return null;
  return matches.reduce((latest, r) =>
    Date.parse(r.created_at) > Date.parse(latest.created_at) ? r : latest,
  );
});

const duplicateLabel = computed(() => {
  if (!duplicate.value) return "";
  return duplicate.value.status === "completed"
    ? t("addUrl.alreadyDownloaded")
    : t("addUrl.alreadyInList");
});

const duplicateMessage = computed(() => {
  const row = duplicate.value;
  if (!row) return "";
  if (row.status === "completed") {
    return t("addUrl.duplicateDone", {
      when: relativeTime(row.completed_at ?? row.created_at),
      folder: parentFolderLabel(row.output_path),
    });
  }
  return t("addUrl.duplicatePending");
});

async function showDuplicateFile() {
  if (!duplicate.value) return;
  try {
    await revealItemInDir(duplicate.value.output_path);
  } catch {
    toast.push(t("errors.revealFile"), "error");
  }
}

// ── Torrent flow ────────────────────────────────────────────────────────

const torrentDisplayName = computed(() => {
  if (torrentMeta.value) return torrentMeta.value.name;
  return displayNameFromMagnet(url.value.trim()) ?? "";
});

function messageOf(e: unknown, fallback: string): string {
  return (e as { message?: string })?.message || fallback;
}

/** Step one: fetch the file list (no download) and reveal the picker. */
async function probeTorrent() {
  const source = detectTorrentSource(url.value.trim());
  if (!source) {
    errorMessage.value = t("addUrl.torrentInvalid");
    return;
  }
  probingTorrent.value = true;
  errorMessage.value = null;
  try {
    const meta = await api.fetchTorrentMetadata(source);
    torrentMeta.value = meta;
    torrentSelected.value = new Set(meta.files.map((f) => f.index));
  } catch (e: unknown) {
    errorMessage.value = messageOf(e, t("addUrl.torrentProbeFailed"));
  } finally {
    probingTorrent.value = false;
  }
}

/** Step two: add the torrent with the user's file selection. */
async function confirmTorrent() {
  const meta = torrentMeta.value;
  const trimmed = url.value.trim();
  const source = detectTorrentSource(trimmed);
  if (!meta || !source) {
    errorMessage.value = t("addUrl.torrentInvalid");
    return;
  }
  if (torrentSelected.value.size === 0) {
    errorMessage.value = t("addUrl.torrentNoFiles");
    return;
  }

  const torrent: TorrentMeta = {
    info_hash: meta.info_hash,
    source,
    selected_files: selectedFileIndices(torrentSelected.value, meta.files.length),
    files: meta.files.map((f) => ({
      index: f.index,
      path: f.path,
      length: f.length,
      selected: torrentSelected.value.has(f.index),
    })),
    swarm: null,
  };

  await addAndClose({
    url: trimmed,
    filename: null,
    output_dir: outputPath.value.trim() || null,
    category_id: isAutoCategory.value ? null : Number(categoryChoice.value),
    segments: null,
    kind: "torrent",
    torrent,
  });
}

// ── Submit ──────────────────────────────────────────────────────────────

async function addAndClose(input: Parameters<typeof downloads.add>[0]) {
  submitting.value = true;
  try {
    const id = await downloads.add(input);
    try {
      await plan.apply(id);
    } catch {
      // The download itself was added; only its start time was not saved.
      toast.push(t("addUrl.scheduleNotSaved"), "error");
    }
    emit("submitted", id);
    emit("close");
  } catch (e: unknown) {
    errorMessage.value = messageOf(e, t("errors.addDownload", { error: "" }));
  } finally {
    submitting.value = false;
  }
}

async function submit() {
  if (!canSubmit.value) return;
  errorMessage.value = null;
  const trimmedUrl = url.value.trim();

  if (isTorrentInput.value) {
    if (!torrentMeta.value) await probeTorrent();
    else await confirmTorrent();
    return;
  }

  // Probe with yt-dlp first when it is installed. Null means "not a media
  // page": fall through to the engine. A throw means yt-dlp could not run.
  if (ytdlpAvailable.value) {
    submitting.value = true;
    try {
      const media = await api.probeMediaUrl(trimmedUrl);
      if (media) {
        probeResult.value = media;
        showFormatsDialog.value = true;
        return;
      }
    } catch (e: unknown) {
      const message = messageOf(e, "");
      if (message.toLowerCase().includes("not installed")) {
        showInstallBanner.value = true;
      } else {
        errorMessage.value = message || t("addUrl.errorProbeFailed");
      }
      return;
    } finally {
      submitting.value = false;
    }
  }

  await addAndClose({
    url: trimmedUrl,
    filename: isRenamed.value ? effectiveName.value : null,
    output_dir: outputPath.value.trim() || null,
    category_id: isAutoCategory.value ? null : Number(categoryChoice.value),
    segments: segments.value,
  });
}

const canSubmit = computed(
  () =>
    !submitting.value &&
    !probingTorrent.value &&
    (probe.kind.value === "http" || probe.kind.value === "torrent") &&
    plan.isValid.value,
);

const isScheduled = computed(() => plan.mode.value === "at" && plan.target.value != null);

const confirmLabel = computed(() => {
  if (submitting.value) return t("addUrl.submitting");
  if (isTorrentInput.value) {
    if (probingTorrent.value) return t("addUrl.torrentProbing");
    return torrentMeta.value ? t("addUrl.torrentAdd") : t("addUrl.torrentFetch");
  }
  if (isScheduled.value && plan.target.value) {
    return t("addUrl.scheduleFor", { time: toTimeInput(plan.target.value) });
  }
  return duplicate.value ? t("addUrl.downloadAgain") : t("addUrl.submit");
});

const footerHint = computed(() =>
  probe.state.value === "checking" ? t("addUrl.checkingFooter") : "",
);

/** Enter in any text field starts the download. */
function onEnter(e: KeyboardEvent) {
  const target = e.target as HTMLElement | null;
  if (e.defaultPrevented || target?.tagName !== "INPUT") return;
  if ((target as HTMLInputElement).type === "checkbox") return;
  e.preventDefault();
  void submit();
}

function onMediaSubmitted(id: number) {
  emit("submitted", id);
  emit("close");
}

const LINK_BUTTON =
  "flex h-6 items-center gap-1.5 rounded-md px-1.5 text-xs font-medium transition-colors hover:bg-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
const LABEL = "text-xs font-medium text-dl-soft";
const TAG =
  "flex h-[22px] shrink-0 items-center rounded-md bg-muted px-2 text-xs font-medium text-dl-soft";
</script>

<template>
  <Dialog
    v-if="!showFormatsDialog"
    :open="open"
    :title="t('addUrl.title')"
    @close="emit('close')"
  >
    <div class="contents" @keydown.enter="onEnter">
      <div class="flex min-w-0 flex-col gap-1.5">
        <div class="flex min-h-5 items-center justify-between gap-2">
          <label :for="linkId" :class="LABEL">{{ t("addUrl.urlLabel") }}</label>
          <button
            type="button"
            :class="[LINK_BUTTON, '-mr-1.5 text-primary']"
            @click="pickTorrentFile"
          >
            <FolderOpen class="h-[13px] w-[13px]" aria-hidden="true" />
            {{ t("addUrl.torrentBrowse") }}
          </button>
        </div>
        <Field
          :id="linkId"
          v-model="url"
          mono
          type="text"
          inputmode="url"
          spellcheck="false"
          autocomplete="off"
          :placeholder="t('addUrl.urlPlaceholder')"
          :invalid="probe.state.value === 'invalid'"
          :aria-describedby="probe.state.value === 'empty' ? undefined : linkMessageId"
        >
          <template #leading>
            <Link
              class="h-[15px] w-[15px] shrink-0"
              :class="probe.state.value === 'invalid' ? 'text-danger' : 'text-muted-foreground'"
              aria-hidden="true"
            />
          </template>
          <template v-if="initialUrl && url === initialUrl" #trailing>
            <span :class="TAG">{{ t("addUrl.pastedTag") }}</span>
          </template>
        </Field>
        <LinkProbeCard
          :state="probe.state.value"
          :result="probe.result.value"
          :host="probe.host.value"
          :name="probe.detectedName.value"
          :torrent-name="torrentDisplayName"
          :duplicate-label="duplicateLabel"
          :message-id="linkMessageId"
          @retry="probe.retry()"
        />
        <div
          v-if="duplicate"
          role="note"
          class="flex items-start gap-2.5 rounded-[10px] bg-dl-warn-soft px-3 py-[9px] text-xs leading-[1.45] shadow-[inset_0_0_0_1px_hsl(var(--warning))]"
        >
          <AlertTriangle class="mt-px h-[15px] w-[15px] shrink-0 text-warning" aria-hidden="true" />
          <span class="flex-1">{{ duplicateMessage }}</span>
          <button
            v-if="duplicate.status === 'completed'"
            type="button"
            class="-my-1 -mr-1 h-[26px] shrink-0 rounded-md px-2 text-xs font-semibold underline underline-offset-[3px] hover:bg-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
            @click="showDuplicateFile"
          >
            {{ t("addUrl.showFile") }}
          </button>
        </div>
      </div>

      <MediaInstallBanner
        v-if="showInstallBanner"
        tool="yt-dlp"
        @close="emit('close')"
      />

      <TorrentFilePicker
        v-if="torrentMeta"
        v-model:selected="torrentSelected"
        :files="torrentMeta.files"
      />

      <div v-if="showRename && !isTorrentInput" class="flex min-w-0 flex-col gap-1.5">
        <div class="flex min-h-5 items-center justify-between gap-2">
          <label :for="nameId" :class="LABEL">{{ t("addUrl.filenameLabel") }}</label>
          <button
            type="button"
            :class="[LINK_BUTTON, '-mr-1.5 text-dl-soft underline decoration-[hsl(var(--input))] underline-offset-[3px]']"
            @click="useDetectedName"
          >
            {{ t("addUrl.useDetectedName") }}
          </button>
        </div>
        <Field
          :id="nameId"
          v-model="renameStem"
          type="text"
          spellcheck="false"
          :placeholder="detectedParts.stem || t('addUrl.filenameHint')"
        >
          <template v-if="detectedParts.extension" #trailing>
            <span class="shrink-0 font-mono text-xs text-muted-foreground">
              {{ detectedParts.extension }}
            </span>
          </template>
        </Field>
        <span class="text-xs leading-[1.45] text-muted-foreground">
          {{ t("addUrl.extensionStays") }}
        </span>
      </div>

      <div class="grid grid-cols-2 gap-3">
        <div class="flex min-w-0 flex-col gap-1.5">
          <span :id="categoryLabelId" :class="LABEL">{{ t("addUrl.categoryLabel") }}</span>
          <Select
            v-model="categoryChoice"
            :options="categoryOptions"
            :aria-label="t('addUrl.categoryLabel')"
          >
            <template v-if="isAutoCategory" #tag>
              <span :class="TAG">{{ t("addUrl.autoTag") }}</span>
            </template>
          </Select>
        </div>
        <div v-if="!isTorrentInput" class="flex min-w-0 flex-col gap-1.5">
          <span :id="connectionsId" :class="LABEL">{{ t("addUrl.connectionsLabel") }}</span>
          <NumberStepper
            v-model="connections"
            :min="MIN_CONNECTIONS"
            :max="MAX_CONNECTIONS"
            :labelledby="connectionsId"
            :decrease-label="t('addUrl.fewerConnections')"
            :increase-label="t('addUrl.moreConnections')"
            :suffix="segments === null ? t('addUrl.connectionsDefault') : undefined"
          />
        </div>
      </div>

      <div class="flex min-w-0 flex-col gap-1.5">
        <label :for="folderId" :class="LABEL">{{ t("addUrl.outputLabel") }}</label>
        <div class="flex gap-2">
          <Field
            :id="folderId"
            v-model="saveTo"
            mono
            type="text"
            spellcheck="false"
            class="flex-1"
            :placeholder="t('addUrl.outputPlaceholder')"
          >
            <template #leading>
              <Folder class="h-[15px] w-[15px] shrink-0 text-muted-foreground" aria-hidden="true" />
            </template>
            <template v-if="isAutoFolder && saveTo" #trailing>
              <span
                :class="TAG"
                :title="chosenCategory ? t('addUrl.autoFolderHint', { category: chosenCategory.name }) : undefined"
              >
                {{ t("addUrl.autoTag") }}
              </span>
            </template>
          </Field>
          <Button @click="pickFolder">{{ t("common.browse") }}</Button>
        </div>
        <span v-if="duplicate" class="text-xs leading-[1.45] text-muted-foreground">
          {{ t("addUrl.duplicateKeepsBoth") }}
        </span>
      </div>

      <StartPicker :plan="plan" :hint="t('addUrl.startAtHint')" />

      <div v-if="!showRename && !isTorrentInput" class="-ml-2 -mt-1 flex">
        <button
          type="button"
          aria-expanded="false"
          class="flex h-[30px] items-center gap-1.5 rounded-lg pl-2 pr-2.5 text-[13px] font-medium text-primary transition-colors hover:bg-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          @click="openRename"
        >
          <Pencil class="h-[13px] w-[13px]" aria-hidden="true" />
          {{ t("addUrl.filenameToggle") }}
        </button>
      </div>

      <p v-if="errorMessage" role="alert" class="text-xs leading-[1.45] text-danger">
        {{ errorMessage }}
      </p>
    </div>

    <template #footer>
      <span v-if="footerHint" class="mr-auto text-xs text-muted-foreground">
        {{ footerHint }}
      </span>
      <Button @click="emit('close')">{{ t("common.cancel") }}</Button>
      <Button variant="primary" :disabled="!canSubmit" @click="submit">
        <Clock v-if="isScheduled" class="h-[15px] w-[15px]" aria-hidden="true" />
        <Download v-else class="h-[15px] w-[15px]" aria-hidden="true" />
        {{ confirmLabel }}
        <kbd v-if="canSubmit" class="font-mono text-xs font-medium opacity-70">
          {{ t("addUrl.enterKey") }}
        </kbd>
      </Button>
    </template>
  </Dialog>

  <MediaFormatsDialog
    v-else
    :open="showFormatsDialog"
    :probe="probeResult"
    :ffmpeg-available="ffmpegAvailable"
    @close="showFormatsDialog = false; emit('close')"
    @submitted="onMediaSubmitted"
  />
</template>
