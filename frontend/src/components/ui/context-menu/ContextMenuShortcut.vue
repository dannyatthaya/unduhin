<script setup lang="ts">
import { computed } from "vue";

import { cn } from "@/lib/utils";

const props = defineProps<{ class?: string }>();

const classes = computed(() =>
  cn(
    "ml-auto font-mono text-[11px] tracking-widest text-muted-foreground",
    props.class,
  ),
);
</script>

<template>
  <span :class="classes">
    <slot />
  </span>
</template>
