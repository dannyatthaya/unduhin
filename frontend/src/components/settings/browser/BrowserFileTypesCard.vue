<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { Plus, X } from "lucide-vue-next";

import Button from "@/components/ui/Button.vue";
import Input from "@/components/ui/Input.vue";
import type { useBrowserSettings } from "@/composables/useBrowserSettings";

const props = defineProps<{
  settings: ReturnType<typeof useBrowserSettings>;
}>();

const { t } = useI18n();

// Seed shown in the picker when the user has not yet customized their
// list. Matches the seed list (21 entries).
const SEED_TYPES = [
  "zip", "rar", "7z", "tar.gz", "iso",
  "exe", "msi", "dmg",
  "mp4", "mkv", "webm", "mov",
  "mp3", "flac", "wav",
  "pdf", "epub",
  "docx", "xlsx", "csv",
  "torrent",
] as const;

const active = computed(() => props.settings.bindings.fileTypes.value);

const pillRows = computed<string[]>(() => {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const item of active.value) {
    if (!seen.has(item)) {
      seen.add(item);
      out.push(item);
    }
  }
  for (const item of SEED_TYPES) {
    if (!seen.has(item)) {
      seen.add(item);
      out.push(item);
    }
  }
  return out;
});

const VALID_EXT = /^[a-z0-9.]{1,8}$/;

const adding = ref(false);
const draft = ref("");
const error = ref<string | null>(null);

function startAdd(): void {
  adding.value = true;
  draft.value = "";
  error.value = null;
}

function cancelAdd(): void {
  adding.value = false;
  draft.value = "";
  error.value = null;
}

function commit(): void {
  const ext = draft.value.trim().toLowerCase().replace(/^\./, "");
  if (!ext) {
    cancelAdd();
    return;
  }
  if (!VALID_EXT.test(ext)) {
    error.value = t("settings.browserFileTypeInvalid");
    return;
  }
  if (!active.value.includes(ext)) {
    props.settings.toggleFileType(ext);
  }
  cancelAdd();
}

function toggle(ext: string): void {
  props.settings.toggleFileType(ext);
}
</script>

<template>
  <article
    class="overflow-hidden rounded-lg border border-border bg-card text-card-foreground"
  >
    <header class="flex items-start justify-between gap-4 px-5 py-4">
      <div class="flex flex-col gap-1">
        <h2 class="text-sm font-semibold">
          {{ t("settings.cardBrowserFileTypes") }}
        </h2>
        <p class="text-xs text-muted-foreground">
          {{ t("settings.cardBrowserFileTypesDesc") }}
        </p>
      </div>
    </header>
    <div class="border-t border-border px-5 py-4">
      <div class="flex flex-wrap gap-2">
        <button
          v-for="ext in pillRows"
          :key="ext"
          type="button"
          :aria-pressed="active.includes(ext)"
          class="inline-flex items-center gap-1 rounded-full border px-3 py-1 text-xs font-medium transition-colors"
          :class="
            active.includes(ext)
              ? 'border-primary bg-primary text-primary-foreground shadow-sm'
              : 'border-border bg-background text-muted-foreground hover:bg-accent'
          "
          @click="toggle(ext)"
        >
          <span>{{ ext }}</span>
          <X
            v-if="active.includes(ext) && !SEED_TYPES.includes(ext as never)"
            class="h-3 w-3 opacity-70"
          />
        </button>
        <Button
          v-if="!adding"
          variant="secondary"
          size="sm"
          class="h-7 rounded-full px-3"
          @click="startAdd"
        >
          <Plus class="h-3 w-3" />
          {{ t("settings.browserFileTypeAdd") }}
        </Button>
        <div v-else class="flex items-center gap-2">
          <Input
            v-model="draft"
            class="h-7 w-24 text-xs"
            :placeholder="t('settings.browserFileTypePlaceholder')"
            autofocus
            @keydown.enter="commit"
            @keydown.escape="cancelAdd"
          />
          <Button size="sm" class="h-7 px-3" @click="commit">
            {{ t("common.add") }}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            class="h-7 px-3"
            @click="cancelAdd"
          >
            {{ t("common.cancel") }}
          </Button>
        </div>
      </div>
      <p v-if="error" class="mt-2 text-xs text-destructive">{{ error }}</p>
      <p v-else-if="active.length === 0" class="mt-3 text-xs text-muted-foreground">
        {{ t("settings.browserFileTypesEmptyHint") }}
      </p>
    </div>
  </article>
</template>
