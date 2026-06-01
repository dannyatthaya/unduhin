<script setup lang="ts">
import { useRouter } from "vue-router";

import Button from "@/components/ui/Button.vue";

defineProps<{
  tool: "yt-dlp" | "ffmpeg";
}>();
const emit = defineEmits<{ close: [] }>();

const router = useRouter();

function openMediaSettings() {
  emit("close");
  router.push("/settings/media");
}
</script>

<template>
  <div
    class="rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs text-foreground"
  >
    <p class="font-medium">
      {{ tool === "yt-dlp" ? "yt-dlp" : "FFmpeg" }} is not installed.
    </p>
    <p class="mt-0.5 text-muted-foreground">
      {{
        tool === "yt-dlp"
          ? "Install yt-dlp to download from YouTube, Vimeo, Twitter and other media sites. Direct file URLs work without it."
          : "FFmpeg is needed when yt-dlp combines separate video and audio streams (typical for YouTube)."
      }}
    </p>
    <div class="mt-2 flex justify-end">
      <Button variant="secondary" size="sm" @click="openMediaSettings">
        Open Media settings
      </Button>
    </div>
  </div>
</template>
