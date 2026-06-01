<script setup lang="ts">
import { useRouter } from "vue-router";
import { Plus, Pause, Search, RotateCw, Filter, LayoutGrid, Settings, Sun, Moon } from "lucide-vue-next";
import { useI18n } from "vue-i18n";
import Button from "./ui/Button.vue";
import { useTheme } from "@/composables/useTheme";

const { t } = useI18n();
const router = useRouter();

defineProps<{
  search: string;
  isEmpty: boolean;
  loading?: boolean;
}>();
const emit = defineEmits<{
  "update:search": [value: string];
  "add-url": [];
  "pause-all": [];
  refresh: [];
}>();

const { isDark, toggle } = useTheme();
</script>

<template>
  <!-- Transparent so the main content's bg tint shows through. The
       row cards (and the welcome cards) carry the visual structure
       on their own. -->
  <header class="flex items-center gap-2 bg-transparent px-5 py-3">
    <Button
      variant="primary"
      size="md"
      @click="emit('add-url')"
    >
      <Plus class="h-4 w-4" />
      {{ t("downloads.addUrl") }}
    </Button>
    <Button
      v-if="!isEmpty && !loading"
      variant="secondary"
      size="md"
      @click="emit('pause-all')"
    >
      <Pause class="h-4 w-4" />
      {{ t("downloads.pauseAll") }}
    </Button>

    <div class="relative ml-2 w-90">
      <Search class="pointer-events-none absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
      <input
        :value="search"
        @input="emit('update:search', ($event.target as HTMLInputElement).value)"
        type="search"
        :placeholder="t('downloads.searchPlaceholder')"
        class="h-9 w-full rounded-md border border-input bg-card pl-8 pr-3 text-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      />
    </div>

    <div class="ml-auto flex items-center gap-1">
      <Button
        size="icon"
        variant="ghost"
        :title="t('downloads.settings')"
        @click="router.push('/settings')"
      >
        <Settings class="h-4 w-4" />
      </Button>
      <template v-if="!isEmpty && !loading">
        <Button
          size="icon"
          variant="ghost"
          :title="t('downloads.refresh')"
          @click="emit('refresh')"
        >
          <RotateCw class="h-4 w-4" />
        </Button>
      </template>
    </div>
  </header>
</template>
