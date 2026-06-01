<script setup lang="ts">
defineProps<{
  title: string;
  description?: string;
}>();
</script>

<template>
  <article
    class="overflow-hidden rounded-lg border border-border bg-card text-card-foreground"
  >
    <header class="flex items-start justify-between gap-4 px-5 py-4">
      <div class="flex flex-col gap-1">
        <h2 class="text-sm font-semibold">{{ title }}</h2>
        <p v-if="description" class="text-xs text-muted-foreground">
          {{ description }}
        </p>
      </div>
      <div v-if="$slots.actions" class="shrink-0">
        <slot name="actions" />
      </div>
    </header>
    <div class="border-t border-border">
      <slot />
    </div>
  </article>
</template>
