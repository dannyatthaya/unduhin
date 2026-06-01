<script setup lang="ts">
// Placeholder card shown during first-load. Mirrors DownloadRow's
// outer shell so the layout doesn't jump when real rows replace it.
// Uses `animate-pulse` (Tailwind built-in) for the shimmer; the
// shimmer block colors live on the card so they swap with the theme.
import { useI18n } from "vue-i18n";
const { t } = useI18n();
</script>

<template>
  <article
    class="relative overflow-hidden rounded-lg border border-border bg-card"
    aria-busy="true"
    :aria-label="t('common.loading')"
  >
    <div class="flex items-start gap-3 px-4 py-3 animate-pulse">
      <div class="h-9 w-9 shrink-0 rounded-md bg-muted" />
      <div class="min-w-0 flex-1 space-y-2">
        <div class="flex items-center gap-2">
          <div class="h-3 w-1/2 rounded-full bg-muted" />
          <div class="h-3 w-14 rounded-full bg-muted/70" />
        </div>
        <div class="h-3 w-2/5 rounded-full bg-muted/70" />
      </div>
      <div class="h-7 w-7 shrink-0 rounded-md bg-muted" />
    </div>
  </article>
</template>
