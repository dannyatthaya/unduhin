<script setup lang="ts">
// Coloured tile that says what kind of file a download is. Replaces the
// old text extension badge; the kind is also exposed as an accessible
// label so colour and icon are never the only signal.

import { computed, type Component } from "vue";
import { useI18n } from "vue-i18n";
import {
  AppWindow,
  Disc,
  File,
  FileArchive,
  FileAudio,
  FileImage,
  FileText,
  FileVideo,
  Magnet,
} from "lucide-vue-next";

import { fileKindOf, type FileKind } from "@/lib/fileKind";
import type { DownloadKind } from "@/types/tauri-bindings";

const props = withDefaults(
  defineProps<{
    filename: string;
    downloadKind?: DownloadKind;
    size?: "md" | "lg";
  }>(),
  { downloadKind: "http", size: "md" },
);

const { t } = useI18n();

interface KindStyle {
  icon: Component;
  tone: string;
  labelKey: string;
}

const KIND_STYLES: Record<FileKind, KindStyle> = {
  video: { icon: FileVideo, tone: "dl-kind-video", labelKey: "downloads.kindVideo" },
  audio: { icon: FileAudio, tone: "dl-kind-audio", labelKey: "downloads.kindAudio" },
  archive: { icon: FileArchive, tone: "dl-kind-archive", labelKey: "downloads.kindArchive" },
  app: { icon: AppWindow, tone: "dl-kind-app", labelKey: "downloads.kindApp" },
  doc: { icon: FileText, tone: "dl-kind-doc", labelKey: "downloads.kindDoc" },
  image: { icon: FileImage, tone: "dl-kind-doc", labelKey: "downloads.kindImage" },
  disc: { icon: Disc, tone: "dl-kind-file", labelKey: "downloads.kindDisc" },
  torrent: { icon: Magnet, tone: "dl-kind-archive", labelKey: "downloads.kindTorrent" },
  file: { icon: File, tone: "dl-kind-file", labelKey: "downloads.kindFile" },
};

const style = computed(
  () => KIND_STYLES[fileKindOf(props.filename, props.downloadKind)],
);
const label = computed(() => t(style.value.labelKey));
</script>

<template>
  <span
    class="flex shrink-0 items-center justify-center"
    :class="[
      style.tone,
      size === 'lg' ? 'h-10 w-10 rounded-[11px]' : 'h-8 w-8 rounded-[9px]',
    ]"
    :title="label"
  >
    <component
      :is="style.icon"
      :class="size === 'lg' ? 'h-5 w-5' : 'h-[17px] w-[17px]'"
      role="img"
      :aria-label="label"
    />
  </span>
</template>
