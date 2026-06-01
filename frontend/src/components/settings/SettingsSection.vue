<script setup lang="ts">
defineProps<{
  eyebrow?: string;
  title: string;
  description?: string;
}>();
</script>

<template>
  <section class="mx-auto flex max-w-3xl flex-col gap-6 px-8 py-8">
    <header class="flex flex-col gap-2 border-b border-border pb-6">
      <span
        v-if="eyebrow"
        class="text-[10px] font-semibold uppercase tracking-[0.18em] text-primary"
        >{{ eyebrow }}</span
      >
      <div class="flex items-start justify-between gap-4">
        <div class="flex flex-col gap-1.5">
          <h1 class="text-2xl font-semibold leading-tight">{{ title }}</h1>
          <p
            v-if="description"
            class="max-w-2xl text-sm text-muted-foreground"
          >
            {{ description }}
          </p>
        </div>
        <div v-if="$slots.actions" class="shrink-0">
          <slot name="actions" />
        </div>
      </div>
    </header>

    <div class="flex flex-col gap-5">
      <slot />
    </div>
  </section>
</template>
