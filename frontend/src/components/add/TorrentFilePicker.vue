<script setup lang="ts">
// Per-file picker shown in New download once a torrent's file list has
// been fetched.

import { computed } from "vue";
import { useI18n } from "vue-i18n";

import { formatBytes } from "@/lib/format";
import type { TorrentMetadataFile } from "@/types/tauri-bindings";

const props = defineProps<{
  files: TorrentMetadataFile[];
  selected: Set<number>;
}>();

const emit = defineEmits<{ "update:selected": [value: Set<number>] }>();

const { t } = useI18n();

const selectedBytes = computed(() =>
  props.files.reduce(
    (sum, file) => (props.selected.has(file.index) ? sum + file.length : sum),
    0,
  ),
);

function toggle(index: number) {
  const next = new Set(props.selected);
  if (next.has(index)) next.delete(index);
  else next.add(index);
  emit("update:selected", next);
}

const LINK_BUTTON =
  "h-6 rounded-md px-1.5 text-xs font-medium text-primary transition-colors hover:bg-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";
</script>

<template>
  <div class="overflow-hidden rounded-xl border border-dl-line">
    <div
      class="flex h-9 items-center justify-between gap-3 border-b border-dl-line bg-dl-group px-3"
    >
      <span class="text-xs font-medium text-dl-soft">
        {{ t("addUrl.torrentFilesCount", { selected: selected.size, total: files.length }) }}
        <span class="ml-2 font-mono font-normal text-muted-foreground">
          {{ formatBytes(selectedBytes) }}
        </span>
      </span>
      <div class="flex gap-1">
        <button
          type="button"
          :class="LINK_BUTTON"
          @click="emit('update:selected', new Set(files.map((f) => f.index)))"
        >
          {{ t("addUrl.torrentSelectAll") }}
        </button>
        <button type="button" :class="LINK_BUTTON" @click="emit('update:selected', new Set())">
          {{ t("addUrl.torrentSelectNone") }}
        </button>
      </div>
    </div>
    <ul class="max-h-44 overflow-y-auto">
      <li
        v-for="file in files"
        :key="file.index"
        class="flex h-8 items-center gap-2.5 border-b border-dl-line-soft px-3 last:border-b-0 hover:bg-dl-hover"
      >
        <input
          :id="`torrent-file-${file.index}`"
          type="checkbox"
          class="h-4 w-4 shrink-0 accent-[hsl(var(--primary))]"
          :checked="selected.has(file.index)"
          @change="toggle(file.index)"
        />
        <label
          :for="`torrent-file-${file.index}`"
          class="min-w-0 flex-1 cursor-pointer truncate"
          :title="file.path"
        >
          {{ file.path }}
        </label>
        <span class="shrink-0 font-mono text-xs text-muted-foreground">
          {{ formatBytes(file.length) }}
        </span>
      </li>
    </ul>
  </div>
</template>
