<script setup lang="ts">
import { computed } from "vue";
import { extOf } from "@/lib/format";

const props = defineProps<{ filename: string; status?: string }>();

const ext = computed(() => extOf(props.filename) || "FILE");

// Subtle colored tile per extension family. Keeps the row visually
// distinct without leaning on a real icon set.
const tone = computed(() => {
  const e = ext.value;
  if (["MP3", "FLAC", "WAV", "M4A", "OGG", "AAC"].includes(e))
    return "bg-emerald-100 text-emerald-700 dark:bg-emerald-500/15 dark:text-emerald-300";
  if (["MP4", "MKV", "MOV", "AVI", "WEBM"].includes(e))
    return "bg-rose-100 text-rose-700 dark:bg-rose-500/15 dark:text-rose-300";
  if (["ZIP", "RAR", "7Z", "TAR", "GZ"].includes(e))
    return "bg-amber-100 text-amber-700 dark:bg-amber-500/15 dark:text-amber-300";
  if (["EXE", "MSI", "APK", "DMG"].includes(e))
    return "bg-violet-100 text-violet-700 dark:bg-violet-500/15 dark:text-violet-300";
  if (["PDF", "DOC", "DOCX", "XLSX", "TXT"].includes(e))
    return "bg-blue-100 text-blue-700 dark:bg-blue-500/15 dark:text-blue-300";
  if (["ISO", "IMG"].includes(e))
    return "bg-sky-100 text-sky-700 dark:bg-sky-500/15 dark:text-sky-300";
  return "bg-muted text-muted-foreground";
});
</script>

<template>
  <div
    class="flex h-10 w-12 shrink-0 items-center justify-center rounded-md text-[10px] font-semibold tracking-wide"
    :class="tone"
  >
    {{ ext }}
  </div>
</template>
