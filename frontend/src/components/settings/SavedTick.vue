<script setup lang="ts">
// "Saved", shown beside a control for a moment after its value is stored.

import { useI18n } from "vue-i18n";
import { Check } from "lucide-vue-next";

defineProps<{ show: boolean }>();

const { t } = useI18n();
</script>

<template>
  <span role="status" class="flex items-center gap-[5px] text-xs font-medium text-success">
    <template v-if="show">
      <Check class="h-[13px] w-[13px]" aria-hidden="true" />
      {{ t("settings.saved") }}
    </template>
  </span>
</template>
