<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";

import Dialog from "@/components/ui/Dialog.vue";
import Button from "@/components/ui/Button.vue";
import Input from "@/components/ui/Input.vue";
import MediaFormatRow from "./MediaFormatRow.vue";

import { useCategoriesStore } from "@/stores/categories";
import { useDownloadsStore } from "@/stores/downloads";
import type { ProbeResult } from "@/types/tauri-bindings";

type Preset = "video_audio" | "audio_only" | "custom";

const props = defineProps<{
  open: boolean;
  probe: ProbeResult | null;
  ffmpegAvailable: boolean;
}>();
const emit = defineEmits<{ close: []; submitted: [id: number] }>();

const downloads = useDownloadsStore();
const categories = useCategoriesStore();

const preset = ref<Preset>("video_audio");
const customSelector = ref("");
const categoryId = ref<number | null>(null);
const outputPath = ref("");
const submitting = ref(false);
const errorMessage = ref<string | null>(null);

watch(
  () => props.open,
  (v) => {
    if (!v) return;
    preset.value = props.probe?.recommended_video_audio ? "video_audio" : "audio_only";
    customSelector.value = "";
    categoryId.value = null;
    outputPath.value = "";
    errorMessage.value = null;
  }
);

const recommended = computed(() => ({
  video_audio: props.probe?.recommended_video_audio ?? null,
  audio_only: props.probe?.recommended_audio_only ?? null,
}));

const effectiveSelector = computed(() => {
  switch (preset.value) {
    case "video_audio":
      return recommended.value.video_audio ?? "bv*+ba/b";
    case "audio_only":
      return recommended.value.audio_only ?? "ba";
    case "custom":
      return customSelector.value.trim();
  }
});

const needsFfmpeg = computed(() => effectiveSelector.value.includes("+"));

const durationLabel = computed(() => {
  const s = props.probe?.duration_secs;
  if (s == null) return null;
  const mins = Math.floor(s / 60);
  const secs = s % 60;
  return `${mins}:${secs.toString().padStart(2, "0")}`;
});

const presetDisabled = (p: Preset): boolean => {
  if (p === "video_audio") return !recommended.value.video_audio;
  if (p === "audio_only") return !recommended.value.audio_only;
  return false;
};

async function pickFolder() {
  const picked = await openFileDialog({ directory: true, multiple: false });
  if (typeof picked === "string") outputPath.value = picked;
}

async function submit() {
  if (!props.probe) return;
  errorMessage.value = null;
  const selector = effectiveSelector.value;
  if (!selector) {
    errorMessage.value = "Please type a custom format selector or pick a preset.";
    return;
  }
  if (needsFfmpeg.value && !props.ffmpegAvailable) {
    errorMessage.value =
      "FFmpeg is required to mux the chosen streams. Install it from Settings → Media.";
    return;
  }
  submitting.value = true;
  try {
    const id = await downloads.add({
      url: props.probe.url,
      filename: null, // engine derives from the title in core::download::insert
      output_path: outputPath.value.trim() || null,
      category_id: categoryId.value,
      media_info: {
        extractor: props.probe.extractor,
        format_selector: selector,
        title: props.probe.title,
        original_url: props.probe.url,
        needs_ffmpeg: needsFfmpeg.value,
      },
    });
    emit("submitted", id);
    emit("close");
  } catch (e: unknown) {
    errorMessage.value =
      (e as { message?: string })?.message ?? "Failed to start media download.";
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <Dialog
    :open="open"
    size="2xl"
    title="Download media"
    @close="emit('close')"
  >
    <div v-if="probe" class="space-y-4">
      <header class="flex items-start gap-3">
        <img
          v-if="probe.thumbnail_url"
          :src="probe.thumbnail_url"
          alt=""
          class="h-20 w-32 shrink-0 rounded-md border border-border object-cover"
        />
        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2 text-[10px] uppercase tracking-wider text-muted-foreground">
            <span>{{ probe.extractor }}</span>
            <span v-if="durationLabel">· {{ durationLabel }}</span>
            <span v-if="probe.is_live" class="rounded bg-danger/15 px-1 text-danger">LIVE</span>
            <span v-if="probe.age_limit && probe.age_limit >= 18" class="rounded bg-amber-500/15 px-1 text-amber-500">18+</span>
          </div>
          <h3 class="mt-0.5 text-sm font-semibold leading-snug">{{ probe.title }}</h3>
          <p v-if="probe.uploader" class="mt-0.5 text-xs text-muted-foreground">
            {{ probe.uploader }}
          </p>
        </div>
      </header>

      <section>
        <label class="mb-1 block text-xs font-medium text-muted-foreground">Preset</label>
        <div class="grid grid-cols-3 gap-2">
          <button
            type="button"
            :class="[
              'rounded-md border px-3 py-2 text-left text-xs transition-colors',
              preset === 'video_audio'
                ? 'border-primary bg-primary/10'
                : 'border-border hover:bg-accent hover:text-accent-foreground',
              presetDisabled('video_audio') && 'opacity-50',
            ]"
            :disabled="presetDisabled('video_audio')"
            @click="preset = 'video_audio'"
          >
            <div class="font-medium">Best video + audio</div>
            <div class="text-muted-foreground">Highest quality available</div>
          </button>
          <button
            type="button"
            :class="[
              'rounded-md border px-3 py-2 text-left text-xs transition-colors',
              preset === 'audio_only'
                ? 'border-primary bg-primary/10'
                : 'border-border hover:bg-accent hover:text-accent-foreground',
              presetDisabled('audio_only') && 'opacity-50',
            ]"
            :disabled="presetDisabled('audio_only')"
            @click="preset = 'audio_only'"
          >
            <div class="font-medium">Audio only</div>
            <div class="text-muted-foreground">Best audio stream</div>
          </button>
          <button
            type="button"
            :class="[
              'rounded-md border px-3 py-2 text-left text-xs transition-colors',
              preset === 'custom'
                ? 'border-primary bg-primary/10'
                : 'border-border hover:bg-accent hover:text-accent-foreground',
            ]"
            @click="preset = 'custom'"
          >
            <div class="font-medium">Custom</div>
            <div class="text-muted-foreground">Pick a format below</div>
          </button>
        </div>
      </section>

      <section v-if="preset === 'custom'">
        <label class="mb-1 block text-xs font-medium text-muted-foreground">
          yt-dlp format selector
        </label>
        <Input
          v-model="customSelector"
          placeholder="e.g. 137+140 or bv*+ba/b"
        />
        <p class="mt-1 text-[11px] text-muted-foreground">
          Pick one of the formats below to fill this in, or type a yt-dlp selector by hand.
        </p>

        <div class="mt-2 max-h-64 overflow-y-auto rounded-md border border-border">
          <div class="space-y-1 p-1">
            <MediaFormatRow
              v-for="f in probe.formats"
              :key="f.format_id"
              :format="f"
              :selected="customSelector === f.format_id"
              :requires-ffmpeg="(f.vcodec ?? 'none') !== 'none' && (f.acodec ?? 'none') === 'none'"
              :ffmpeg-available="ffmpegAvailable"
              @select="(id) => (customSelector = id)"
            />
          </div>
        </div>
      </section>

      <section class="grid grid-cols-2 gap-3">
        <div>
          <label class="mb-1 block text-xs font-medium text-muted-foreground">Category</label>
          <select
            v-model.number="categoryId"
            class="h-9 w-full rounded-md border border-input bg-background px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <option :value="null">Auto-detect</option>
            <option v-for="c in categories.list" :key="c.id" :value="c.id">
              {{ c.name }}
            </option>
          </select>
        </div>
        <div>
          <label class="mb-1 block text-xs font-medium text-muted-foreground">
            Output folder
          </label>
          <div class="flex gap-2">
            <Input v-model="outputPath" placeholder="leave blank for category default" />
            <Button variant="secondary" size="md" @click="pickFolder">…</Button>
          </div>
        </div>
      </section>

      <div
        v-if="needsFfmpeg && !ffmpegAvailable"
        class="rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs text-foreground"
      >
        FFmpeg is required to combine separate video and audio streams.
        Install it from Settings → Media or pick a format that doesn't need muxing.
      </div>

      <p v-if="errorMessage" class="text-xs text-danger">{{ errorMessage }}</p>
    </div>

    <template #footer>
      <Button variant="ghost" @click="emit('close')">Cancel</Button>
      <Button
        variant="primary"
        :disabled="submitting || !probe || (needsFfmpeg && !ffmpegAvailable)"
        @click="submit"
      >
        {{ submitting ? "Adding…" : "Start download" }}
      </Button>
    </template>
  </Dialog>
</template>
