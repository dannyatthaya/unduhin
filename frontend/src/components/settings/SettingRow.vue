<script setup lang="ts">
defineProps<{
  id: string;
  label: string;
  description?: string;
  /** Hide the row when the filter doesn't include it. */
  hidden?: boolean;
}>();
</script>

<template>
  <div
    v-show="!hidden"
    :data-setting-id="id"
    class="flex items-start gap-6 border-b border-border/60 px-5 py-4 last:border-b-0 transition-shadow data-[settings-highlight=true]:shadow-[inset_0_0_0_2px_var(--ring,theme(colors.primary.DEFAULT/40))]"
  >
    <div class="flex min-w-0 flex-1 flex-col gap-1.5">
      <div class="text-sm font-medium leading-tight">{{ label }}</div>
      <p v-if="description" class="text-xs text-muted-foreground">
        {{ description }}
      </p>
    </div>
    <div class="flex shrink-0 items-center gap-3">
      <slot />
    </div>
  </div>
</template>
