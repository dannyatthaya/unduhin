<script setup lang="ts">
// Placeholder row shown during first-load. Uses the same grid template
// and height as DownloadRow so the layout doesn't jump when real rows
// replace it. `animate-pulse` (Tailwind built-in) provides the shimmer.
import { useI18n } from "vue-i18n";

defineProps<{
  /** Size and Added columns are hidden. */
  compact?: boolean;
}>();

const { t } = useI18n();
</script>

<template>
  <div
    class="dl-grid-row h-[52px] animate-pulse border-b border-dl-line-soft pl-7 pr-5"
    aria-busy="true"
    :aria-label="t('common.loading')"
  >
    <span class="h-3.5 w-3.5 rounded-sm bg-muted" />
    <span class="flex min-w-0 items-center gap-2.5">
      <span class="h-8 w-8 shrink-0 rounded-[9px] bg-muted" />
      <span class="flex min-w-0 flex-1 flex-col gap-2">
        <span class="h-3 w-1/2 rounded-full bg-muted" />
        <span class="h-2.5 w-1/4 rounded-full bg-muted/70" />
      </span>
    </span>
    <span class="flex flex-col gap-2 pr-5">
      <span class="h-3 w-2/5 rounded-full bg-muted" />
      <span class="h-1 rounded-sm bg-muted/70" />
    </span>
    <template v-if="!compact">
      <span class="ml-auto h-3 w-12 rounded-full bg-muted/70" />
      <span class="ml-auto h-3 w-10 rounded-full bg-muted/70" />
    </template>
    <span class="ml-auto h-8 w-8 rounded-lg bg-muted" />
  </div>
</template>
