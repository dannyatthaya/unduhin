<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { Status } from "@/types/tauri-bindings";

const { t } = useI18n();
const props = defineProps<{ status: Status; queuePosition?: number | null }>();

const label = computed(() => {
  switch (props.status) {
    case "active":
      return t("downloads.statusActive");
    case "muxing":
      return t("downloads.statusMuxing");
    case "paused":
      return t("downloads.statusPaused");
    case "queued":
      return props.queuePosition != null
        ? t("downloads.statusQueuedAt", { position: props.queuePosition })
        : t("downloads.statusQueued");
    case "completed":
      return t("downloads.statusDone");
    case "failed":
      return t("downloads.statusFailed");
    case "cancelled":
      return t("downloads.statusCancelled");
  }
});

const classes = computed(() => {
  const base =
    "inline-flex items-center gap-1.5 rounded-md px-2 py-0.5 text-xs font-medium";
  switch (props.status) {
    case "active":
      return `${base} bg-primary/10 text-primary`;
    case "muxing":
      return `${base} bg-info/15 text-info`;
    case "paused":
      return `${base} bg-warning/15 text-warning`;
    case "queued":
      return `${base} bg-muted text-muted-foreground`;
    case "completed":
      return `${base} bg-success/15 text-success`;
    case "failed":
      return `${base} bg-danger/15 text-danger`;
    case "cancelled":
      return `${base} bg-muted text-muted-foreground`;
  }
});

const dotClass = computed(() => {
  switch (props.status) {
    case "active":
      return "bg-primary";
    case "muxing":
      return "bg-info";
    case "paused":
      return "bg-warning";
    case "queued":
      return "bg-muted-foreground";
    case "completed":
      return "bg-success";
    case "failed":
      return "bg-danger";
    case "cancelled":
      return "bg-muted-foreground";
  }
});
</script>

<template>
  <span :class="classes">
    <span class="h-1.5 w-1.5 rounded-full" :class="dotClass" />
    {{ label }}
  </span>
</template>
