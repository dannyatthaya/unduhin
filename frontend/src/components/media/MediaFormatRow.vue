<script setup lang="ts">
import { computed } from "vue";

import type { Format } from "@/types/tauri-bindings";

const props = defineProps<{
  format: Format;
  selected: boolean;
  /** Whether this format requires ffmpeg muxing (separate streams). */
  requiresFfmpeg?: boolean;
  ffmpegAvailable: boolean;
}>();
defineEmits<{ select: [formatId: string] }>();

const isAudioOnly = computed(
  () => (props.format.vcodec ?? "none") === "none" && (props.format.acodec ?? "none") !== "none"
);
const isVideoOnly = computed(
  () => (props.format.vcodec ?? "none") !== "none" && (props.format.acodec ?? "none") === "none"
);

const kind = computed(() => {
  if (isAudioOnly.value) return "Audio";
  if (isVideoOnly.value) return "Video";
  return "Video + Audio";
});

const sizeLabel = computed(() => {
  const s = props.format.filesize_bytes;
  if (s == null) return "—";
  const mb = s / (1024 * 1024);
  return mb < 1 ? `${(s / 1024).toFixed(0)} KB` : `${mb.toFixed(1)} MB`;
});

const blocked = computed(
  () => (props.requiresFfmpeg ?? false) && !props.ffmpegAvailable
);
</script>

<template>
  <button
    type="button"
    :class="[
      'flex w-full items-center justify-between gap-3 rounded-md border px-3 py-2 text-left text-xs transition-colors',
      selected
        ? 'border-primary bg-primary/10'
        : 'border-border hover:bg-accent hover:text-accent-foreground',
      blocked && 'opacity-50',
    ]"
    :disabled="blocked"
    :title="blocked ? 'FFmpeg required — install it from Settings → Media' : undefined"
    @click="$emit('select', format.format_id)"
  >
    <div class="flex min-w-0 flex-col">
      <div class="flex items-center gap-2 font-medium">
        <span>{{ format.resolution ?? format.format_id }}</span>
        <span class="text-muted-foreground">·</span>
        <span class="text-muted-foreground">{{ kind }}</span>
        <span v-if="format.fps" class="text-muted-foreground">{{ format.fps }}fps</span>
      </div>
      <div class="truncate text-muted-foreground">
        {{ format.ext }}
        <template v-if="format.vcodec && format.vcodec !== 'none'">
          · {{ format.vcodec }}
        </template>
        <template v-if="format.acodec && format.acodec !== 'none'">
          · {{ format.acodec }}
        </template>
        <template v-if="format.note"> · {{ format.note }}</template>
      </div>
    </div>
    <div class="shrink-0 text-right text-muted-foreground">
      <div>{{ sizeLabel }}</div>
      <div v-if="format.tbr_kbps">~{{ format.tbr_kbps.toFixed(0) }} kbps</div>
    </div>
  </button>
</template>
