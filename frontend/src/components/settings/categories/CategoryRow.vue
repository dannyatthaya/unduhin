<script setup lang="ts">
import { computed } from "vue";
import { GripVertical, Pencil, Trash2 } from "lucide-vue-next";

import { useDownloadsStore } from "@/stores/downloads";
import type { Category } from "@/types/tauri-bindings";

import { iconFor } from "@/lib/categoryIcons";

const props = defineProps<{
  category: Category;
  isFallback?: boolean;
}>();
const emit = defineEmits<{
  edit: [category: Category];
  remove: [category: Category];
}>();

const downloads = useDownloadsStore();

const downloadCount = computed(
  () => downloads.all.filter((d) => d.category_id === props.category.id).length,
);

const folder = computed(() => props.category.default_output_path ?? "—");
const iconOpt = computed(() => iconFor(props.category.icon));
</script>

<template>
  <div
    class="flex items-center gap-3 border-t border-border/60 px-4 py-3 first:border-t-0"
  >
    <span
      class="drag-handle flex h-8 w-5 cursor-grab items-center justify-center text-muted-foreground/60 transition-colors hover:text-foreground active:cursor-grabbing"
      :class="isFallback ? 'pointer-events-none opacity-0' : ''"
      aria-label="Drag to reorder"
    >
      <GripVertical class="h-4 w-4" />
    </span>

    <span
      class="flex h-10 w-10 shrink-0 items-center justify-center rounded-md"
      :class="iconOpt.background"
    >
      <component :is="iconOpt.icon" class="h-5 w-5" :class="iconOpt.tone" />
    </span>

    <div class="flex min-w-0 flex-1 flex-col gap-0.5">
      <div class="flex items-center gap-2">
        <span class="text-sm font-medium">{{ category.name }}</span>
        <span
          v-if="isFallback"
          class="rounded bg-muted px-1.5 py-0.5 font-mono text-[10px] uppercase text-muted-foreground"
        >
          fallback
        </span>
      </div>
      <p
        v-if="category.extension_rules.length > 0"
        class="truncate font-mono text-[11px] text-muted-foreground"
      >
        {{ category.extension_rules.map((e) => "." + e).join(" · ") }}
      </p>
      <p v-else-if="isFallback" class="text-[11px] text-muted-foreground">
        Anything no other rule catches
      </p>
    </div>

    <div class="hidden w-48 shrink-0 truncate text-xs text-muted-foreground sm:block">
      {{ folder }}
    </div>

    <div class="w-12 shrink-0 text-right text-sm font-semibold tabular-nums">
      {{ downloadCount }}
    </div>

    <div class="flex shrink-0 items-center gap-1">
      <button
        type="button"
        class="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        title="Edit"
        @click="emit('edit', category)"
      >
        <Pencil class="h-3.5 w-3.5" />
      </button>
      <button
        type="button"
        :disabled="isFallback"
        class="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-danger disabled:cursor-not-allowed disabled:opacity-30"
        :title="isFallback ? 'Fallback category cannot be deleted' : 'Delete'"
        @click="emit('remove', category)"
      >
        <Trash2 class="h-3.5 w-3.5" />
      </button>
    </div>
  </div>
</template>
