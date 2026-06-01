<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import { useRoute } from "vue-router";

import SettingsSidebar from "@/components/settings/SettingsSidebar.vue";
import { useSettingsStore } from "@/stores/settings";
import { useCategoriesStore } from "@/stores/categories";
import { useSystemStore } from "@/stores/system";

// Make sure the stores backing the Settings UI are populated when the
// user navigates here for the first time. These calls are idempotent —
// re-invoking them on subsequent visits just re-fetches.
const settings = useSettingsStore();
const categories = useCategoriesStore();
const system = useSystemStore();

onMounted(() => {
  void settings.refresh();
  void categories.refresh();
  void system.refresh();
});

const route = useRoute();
const mainRef = ref<HTMLElement | null>(null);

watch(
  () => route.path,
  () => {
    mainRef.value?.scrollTo({ top: 0, behavior: "instant" });
  },
);
</script>

<template>
  <div class="flex min-h-0 flex-1 bg-muted/30">
    <SettingsSidebar />
    <main ref="mainRef" class="relative min-w-0 flex-1 overflow-y-auto">
      <RouterView />
    </main>
  </div>
</template>
