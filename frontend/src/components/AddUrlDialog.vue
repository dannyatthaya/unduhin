<script setup lang="ts">
import { ref, watch } from "vue";
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
import { api, type ProbeResult } from "@/types/tauri-bindings";

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
 */
async function maybePrefillFilename() {
  const trimmed = url.value.trim();
  if (!trimmed || trimmed === lastPreviewedUrl) return;
  if (filename.value !== "") return;
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

async function submit() {
  errorMessage.value = null;
  const trimmedUrl = url.value.trim();
  if (!trimmedUrl) {
    errorMessage.value = t("addUrl.errorMissingUrl");
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
      </div>

      <MediaInstallBanner
        v-if="showInstallBanner"
        tool="yt-dlp"
        @close="emit('close')"
      />

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
        <div>
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
      <Button variant="primary" :disabled="submitting" @click="submit">
        {{ submitting ? t("addUrl.submitting") : t("addUrl.submit") }}
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
