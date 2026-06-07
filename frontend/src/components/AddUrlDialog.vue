<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";

import Dialog from "./ui/Dialog.vue";
import Button from "./ui/Button.vue";
import Input from "./ui/Input.vue";
import MediaInstallBanner from "./media/MediaInstallBanner.vue";
import MediaFormatsDialog from "./media/MediaFormatsDialog.vue";

import { useCategoriesStore } from "@/stores/categories";
import { useDownloadsStore } from "@/stores/downloads";
import { useToolingStatus } from "@/composables/useToolingStatus";
import { useGeneralSettings } from "@/composables/useGeneralSettings";
import {
  api,
  type ProbeResult,
  type TorrentMetadataResult,
  type TorrentMeta,
} from "@/types/tauri-bindings";
import {
  detectTorrentSource,
  displayNameFromMagnet,
  isMagnetUri,
  isTorrentFile,
  selectedFileIndices,
} from "@/lib/torrentInput";
import { formatBytes } from "@/lib/format";

const { t } = useI18n();
const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ close: []; submitted: [id: number] }>();

const downloads = useDownloadsStore();
const categories = useCategoriesStore();
const { ytdlpAvailable, ffmpegAvailable } = useToolingStatus();
const generalSettings = useGeneralSettings();

const url = ref("");
const filename = ref("");
const categoryId = ref<number | null>(null);
const segments = ref<number | null>(null);
const outputPath = ref("");
const submitting = ref(false);
const errorMessage = ref<string | null>(null);

// Optional "Start at" — when filled, a `start_at` schedule row is added
// after the download row is created. The downloads.add() insertion lands
// the row as `queued`; the schedule then keeps it queued until the time
// arrives. Local-time `datetime-local` value, serialized to RFC3339 UTC
// on submit.
const scheduleEnabled = ref(false);
const scheduleStartAt = ref("");

// Collapsible "Use a different filename" disclosure. Auto-expands when
// the user has the `always_ask_filename` preference on. Initial value
// is read from the live setting; we re-sync each time the dialog opens.
const showOverride = ref(false);
const previewing = ref(false);
let lastPreviewedUrl = "";

const showInstallBanner = ref(false);
const probeResult = ref<ProbeResult | null>(null);
const showFormatsDialog = ref(false);

// ── Torrent path ─────────────────────────────────────────────────────────
// When the URL field holds a `magnet:` URI or a `.torrent` path, the dialog
// switches to a two-step torrent flow: the first confirm probes the file
// list (librqbit `list_only`) and reveals a per-file picker; the second
// confirm builds an `AddDownload { kind: "torrent", torrent }` from the
// selection. `isTorrentInput` drives the segments-field hiding and the
// confirm-button label.
const torrentMeta = ref<TorrentMetadataResult | null>(null);
const torrentSelected = ref<Set<number>>(new Set());
const probingTorrent = ref(false);

const isTorrentInput = computed(() => {
  const trimmed = url.value.trim();
  return isMagnetUri(trimmed) || isTorrentFile(trimmed);
});

/** Bytes the user has selected to download (selected files only). */
const torrentSelectedBytes = computed(() => {
  const meta = torrentMeta.value;
  if (!meta) return 0;
  let n = 0;
  for (const f of meta.files) {
    if (torrentSelected.value.has(f.index)) n += f.length;
  }
  return n;
});

function resetTorrentState() {
  torrentMeta.value = null;
  torrentSelected.value = new Set();
  probingTorrent.value = false;
}

watch(
  () => props.open,
  (v) => {
    if (v) {
      url.value = "";
      filename.value = "";
      categoryId.value = null;
      segments.value = null;
      outputPath.value = "";
      errorMessage.value = null;
      showInstallBanner.value = false;
      probeResult.value = null;
      showFormatsDialog.value = false;
      showOverride.value = generalSettings.alwaysAskFilename.value;
      lastPreviewedUrl = "";
      scheduleEnabled.value = false;
      resetTorrentState();
      // Default to "1 minute from now" so the field is populated with
      // something sensible when the disclosure opens.
      const d = new Date(Date.now() + 60_000);
      const pad = (n: number) => n.toString().padStart(2, "0");
      scheduleStartAt.value = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(
        d.getDate(),
      )}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
    }
  }
);

/**
 * Lazy HEAD-probe to pre-fill `filename` with the engine's best guess.
 * Fires only when the override field becomes visible (either the user
 * clicks the disclosure or `always_ask_filename` flipped it on),
 * gated by URL change so we don't refire on every render.
 *
 * Never clobbers user input: we only write when `filename === ""`.
 * Torrent inputs have no HTTP resource to HEAD-probe, so we skip them.
 */
async function maybePrefillFilename() {
  const trimmed = url.value.trim();
  if (!trimmed || trimmed === lastPreviewedUrl) return;
  if (filename.value !== "") return;
  if (isTorrentInput.value) return;
  previewing.value = true;
  try {
    const guess = await api.previewFilename(trimmed);
    if (guess && filename.value === "") {
      filename.value = guess;
    }
    lastPreviewedUrl = trimmed;
  } catch {
    // Preview is best-effort; leave the field empty for the user.
  } finally {
    previewing.value = false;
  }
}

watch(showOverride, (open) => {
  if (open) void maybePrefillFilename();
});
watch(url, () => {
  // Editing the input invalidates any probed torrent file list — drop it
  // so a corrected magnet re-probes on the next confirm.
  resetTorrentState();
  // If the field is visible and empty, re-probe when the URL changes
  // so a paste-then-correct cycle gets a fresh suggestion.
  if (showOverride.value && filename.value === "") {
    void maybePrefillFilename();
  }
});

async function pickFolder() {
  const picked = await openFileDialog({ directory: true, multiple: false });
  if (typeof picked === "string") outputPath.value = picked;
}

/** Browse for a local `.torrent` file; fills the URL field with its path
 *  so the standard torrent flow picks it up. */
async function pickTorrentFile() {
  const picked = await openFileDialog({
    directory: false,
    multiple: false,
    filters: [{ name: "Torrent", extensions: ["torrent"] }],
  });
  if (typeof picked === "string") {
    url.value = picked;
  }
}

function toggleTorrentFile(index: number) {
  const next = new Set(torrentSelected.value);
  if (next.has(index)) {
    next.delete(index);
  } else {
    next.add(index);
  }
  torrentSelected.value = next;
}

function selectAllTorrentFiles() {
  const meta = torrentMeta.value;
  if (!meta) return;
  torrentSelected.value = new Set(meta.files.map((f) => f.index));
}

function selectNoTorrentFiles() {
  torrentSelected.value = new Set();
}

/**
 * Step one of the torrent flow: probe the file list (no download) and reveal
 * the per-file picker. Errors (no peers / DHT off / malformed input) surface
 * inline. Selects every file by default so a one-click confirm downloads the
 * whole torrent.
 */
async function probeTorrent() {
  const trimmed = url.value.trim();
  const source = detectTorrentSource(trimmed);
  if (!source) {
    errorMessage.value = t("addUrl.torrentInvalid");
    return;
  }
  probingTorrent.value = true;
  errorMessage.value = null;
  try {
    const meta = await api.fetchTorrentMetadata(source);
    torrentMeta.value = meta;
    // Default to "all files selected" so a one-click confirm grabs the whole
    // torrent. The display name comes from `torrentDisplayName`; we leave the
    // filename-override field empty and let the backend reconcile the real
    // name once librqbit resolves metadata (`FilenameLearned`).
    torrentSelected.value = new Set(meta.files.map((f) => f.index));
  } catch (e: unknown) {
    errorMessage.value =
      (e as { message?: string })?.message ?? t("addUrl.torrentProbeFailed");
  } finally {
    probingTorrent.value = false;
  }
}

/**
 * Step two of the torrent flow: build the `TorrentMeta` from the user's file
 * selection and add the download. `selected_files = null` means "all files"
 * (the librqbit `only_files` default); otherwise the explicit index list.
 */
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

  const selectedFiles = selectedFileIndices(
    torrentSelected.value,
    meta.files.length,
  );

  const torrent: TorrentMeta = {
    info_hash: meta.info_hash,
    source,
    selected_files: selectedFiles,
    files: meta.files.map((f) => ({
      index: f.index,
      path: f.path,
      length: f.length,
      selected: torrentSelected.value.has(f.index),
    })),
    swarm: null,
  };

  submitting.value = true;
  try {
    // For a magnet the URL field is the magnet URI; the backend keys de-dup
    // and the provisional name off `torrent.source`, not this string.
    const id = await downloads.add({
      url: trimmed,
      filename: filename.value.trim() || null,
      output_path: outputPath.value.trim() || null,
      category_id: categoryId.value,
      segments: null,
      kind: "torrent",
      torrent,
    });
    await attachScheduleIfRequested(id);
    emit("submitted", id);
    emit("close");
  } catch (e: unknown) {
    errorMessage.value =
      (e as { message?: string })?.message ??
      t("errors.addDownload", { error: "" });
  } finally {
    submitting.value = false;
  }
}

/** Shared "Start at" attach used by both the HTTP and torrent paths. */
async function attachScheduleIfRequested(id: number) {
  if (scheduleEnabled.value && scheduleStartAt.value) {
    try {
      await api.addSchedule({
        kind: "start_at",
        download_id: id,
        start_iso: new Date(scheduleStartAt.value).toISOString(),
      });
    } catch (e) {
      // The download itself succeeded; surfacing a schedule-add failure
      // as an inline error would be confusing. Log and move on; the
      // user can re-open ScheduleDialog from the detail pane to retry.
      console.warn("Failed to attach start_at schedule:", e);
    }
  }
}

async function submit() {
  errorMessage.value = null;
  const trimmedUrl = url.value.trim();
  if (!trimmedUrl) {
    errorMessage.value = t("addUrl.errorMissingUrl");
    return;
  }

  // Torrent branch: a magnet URI or `.torrent` path never goes through the
  // yt-dlp probe / HTTP engine. The first confirm probes the file list; once
  // we have it, the same button confirms the selection and adds the row.
  if (isTorrentInput.value) {
    if (!torrentMeta.value) {
      await probeTorrent();
    } else {
      await confirmTorrent();
    }
    return;
  }

  // Probe with yt-dlp first when it's installed. A null result means
  // "not recognized" — fall through to the engine path. A throw means
  // yt-dlp couldn't run (typically NotInstalled, DRM, or AuthRequired);
  // we surface NotInstalled as a banner and other errors as inline text.
  submitting.value = true;
  try {
    if (ytdlpAvailable.value) {
      try {
        const probe = await api.probeMediaUrl(trimmedUrl);
        if (probe) {
          probeResult.value = probe;
          showFormatsDialog.value = true;
          return;
        }
      } catch (e: unknown) {
        const message = (e as { message?: string })?.message ?? "";
        if (message.toLowerCase().includes("not installed")) {
          showInstallBanner.value = true;
          return;
        }
        errorMessage.value = message || t("addUrl.errorProbeFailed");
        return;
      }
    }

    const id = await downloads.add({
      url: trimmedUrl,
      filename: filename.value.trim() || null,
      output_path: outputPath.value.trim() || null,
      category_id: categoryId.value,
      segments: segments.value,
    });
    await attachScheduleIfRequested(id);
    emit("submitted", id);
    emit("close");
  } catch (e: unknown) {
    errorMessage.value =
      (e as { message?: string })?.message ??
      t("errors.addDownload", { error: "" });
  } finally {
    submitting.value = false;
  }
}

const confirmLabel = computed(() => {
  if (submitting.value) return t("addUrl.submitting");
  if (isTorrentInput.value) {
    if (probingTorrent.value) return t("addUrl.torrentProbing");
    return torrentMeta.value ? t("addUrl.torrentAdd") : t("addUrl.torrentFetch");
  }
  return t("addUrl.submit");
});

const confirmDisabled = computed(
  () => submitting.value || probingTorrent.value,
);

const torrentDisplayName = computed(() => {
  if (torrentMeta.value) return torrentMeta.value.name;
  const trimmed = url.value.trim();
  return displayNameFromMagnet(trimmed) ?? "";
});

function onMediaSubmitted(id: number) {
  emit("submitted", id);
  emit("close");
}
</script>

<template>
  <Dialog
    v-if="!showFormatsDialog"
    :open="open"
    :title="t('addUrl.title')"
    @close="emit('close')"
  >
    <div class="space-y-3">
      <div>
        <label class="mb-1 block text-xs font-medium text-muted-foreground">{{ t("addUrl.urlLabel") }}</label>
        <Input v-model="url" :placeholder="t('addUrl.urlPlaceholder')" />
        <div class="mt-1 flex items-center justify-between gap-2">
          <p
            v-if="isTorrentInput"
            :title="torrentDisplayName"
            class="min-w-0 truncate text-[11px] text-primary"
          >
            {{ t("addUrl.torrentDetected") }}
            <span v-if="torrentDisplayName" class="text-muted-foreground">
              · {{ torrentDisplayName }}
            </span>
          </p>
          <button
            type="button"
            class="ml-auto shrink-0 whitespace-nowrap text-[11px] font-medium text-primary hover:underline"
            @click="pickTorrentFile"
          >
            {{ t("addUrl.torrentBrowse") }}
          </button>
        </div>
      </div>

      <MediaInstallBanner
        v-if="showInstallBanner"
        tool="yt-dlp"
        @close="emit('close')"
      />

      <!-- Torrent file picker (revealed after a successful metadata probe). -->
      <div v-if="torrentMeta" class="rounded-md border border-border">
        <div
          class="flex items-center justify-between border-b border-border px-3 py-2"
        >
          <span class="text-xs font-medium text-muted-foreground">
            {{
              t("addUrl.torrentFilesSummary", {
                selected: torrentSelected.size,
                total: torrentMeta.files.length,
                size: formatBytes(torrentSelectedBytes),
              })
            }}
          </span>
          <div class="flex gap-2">
            <button
              type="button"
              class="text-[11px] font-medium text-primary hover:underline"
              @click="selectAllTorrentFiles"
            >
              {{ t("addUrl.torrentSelectAll") }}
            </button>
            <button
              type="button"
              class="text-[11px] font-medium text-primary hover:underline"
              @click="selectNoTorrentFiles"
            >
              {{ t("addUrl.torrentSelectNone") }}
            </button>
          </div>
        </div>
        <ul class="max-h-48 overflow-y-auto">
          <li
            v-for="f in torrentMeta.files"
            :key="f.index"
            class="flex items-center gap-2 px-3 py-1.5 text-sm hover:bg-muted/40"
          >
            <input
              :id="`torrent-file-${f.index}`"
              type="checkbox"
              class="h-4 w-4 shrink-0 rounded border-input"
              :checked="torrentSelected.has(f.index)"
              @change="toggleTorrentFile(f.index)"
            />
            <label
              :for="`torrent-file-${f.index}`"
              class="min-w-0 flex-1 cursor-pointer truncate"
              :title="f.path"
            >
              {{ f.path }}
            </label>
            <span class="shrink-0 text-xs text-muted-foreground">
              {{ formatBytes(f.length) }}
            </span>
          </li>
        </ul>
      </div>

      <div v-if="!showOverride">
        <button
          type="button"
          class="text-xs font-medium text-primary hover:underline"
          @click="showOverride = true"
        >
          {{ t("addUrl.filenameToggle") }}
        </button>
      </div>
      <div v-else>
        <label class="mb-1 block text-xs font-medium text-muted-foreground">
          {{ t("addUrl.filenameLabel") }}
        </label>
        <Input
          v-model="filename"
          :placeholder="previewing ? t('addUrl.filenameDetecting') : t('addUrl.filenameHint')"
        />
      </div>

      <div class="grid grid-cols-2 gap-3">
        <div>
          <label class="mb-1 block text-xs font-medium text-muted-foreground">
            {{ t("addUrl.categoryLabel") }}
          </label>
          <select
            v-model.number="categoryId"
            class="h-9 w-full rounded-md border border-input bg-background px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <option :value="null">{{ t("addUrl.categoryAuto") }}</option>
            <option v-for="c in categories.list" :key="c.id" :value="c.id">
              {{ c.name }}
            </option>
          </select>
        </div>
        <div v-if="!isTorrentInput">
          <label class="mb-1 block text-xs font-medium text-muted-foreground">
            {{ t("addUrl.segmentsLabel") }}
          </label>
          <Input
            :model-value="segments?.toString() ?? ''"
            @update:model-value="
              (v: string) => (segments = v ? Math.max(1, parseInt(v, 10) || 1) : null)
            "
            :placeholder="t('addUrl.segmentsPlaceholder', { n: 8 })"
            type="number"
          />
        </div>
      </div>

      <div>
        <label class="mb-1 block text-xs font-medium text-muted-foreground">
          {{ t("addUrl.outputLabel") }}
        </label>
        <div class="flex gap-2">
          <Input v-model="outputPath" :placeholder="t('addUrl.outputPlaceholder')" />
          <Button variant="secondary" size="md" @click="pickFolder">{{ t("common.browse") }}</Button>
        </div>
      </div>

      <div>
        <button
          v-if="!scheduleEnabled"
          type="button"
          class="text-xs font-medium text-primary hover:underline"
          @click="scheduleEnabled = true"
        >
          {{ t("addUrl.startAtToggle") }}
        </button>
        <div v-else>
          <div class="mb-1 flex items-center justify-between">
            <label class="text-xs font-medium text-muted-foreground">
              {{ t("addUrl.startAtLabel") }}
            </label>
            <button
              type="button"
              class="text-xs text-muted-foreground hover:underline"
              @click="scheduleEnabled = false"
            >
              {{ t("addUrl.startAtClear") }}
            </button>
          </div>
          <Input v-model="scheduleStartAt" type="datetime-local" />
          <p class="mt-1 text-[11px] text-muted-foreground">
            {{ t("addUrl.startAtHint") }}
          </p>
        </div>
      </div>

      <p v-if="errorMessage" class="text-xs text-danger">{{ errorMessage }}</p>
    </div>

    <template #footer>
      <Button variant="ghost" @click="emit('close')">{{ t("common.cancel") }}</Button>
      <Button variant="primary" :disabled="confirmDisabled" @click="submit">
        {{ confirmLabel }}
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
