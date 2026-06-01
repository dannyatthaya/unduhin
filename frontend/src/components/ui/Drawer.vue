<script setup lang="ts">
import { computed } from "vue";
import {
  DialogContent,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
} from "reka-ui";

import { cn } from "@/lib/utils";

type Side = "right" | "left";

const props = withDefaults(
  defineProps<{
    open: boolean;
    side?: Side;
    /** Tailwind width class for the drawer body. */
    widthClass?: string;
    /**
     * When true, clicking the overlay or pressing Escape will NOT close
     * the drawer (the parent must drive the close itself). Used by
     * DownloadsView, which routes Escape through a priority chain.
     */
    blockAutoClose?: boolean;
  }>(),
  { side: "right", widthClass: "w-[420px]" },
);

const emit = defineEmits<{ close: [] }>();

function onUpdateOpen(value: boolean) {
  if (!value) emit("close");
}

const sideClasses = computed(() =>
  props.side === "right"
    ? "right-0 border-l data-[state=open]:slide-in-from-right data-[state=closed]:slide-out-to-right"
    : "left-0 border-r data-[state=open]:slide-in-from-left data-[state=closed]:slide-out-to-left",
);
</script>

<template>
  <DialogRoot :open="open" :modal="!blockAutoClose" @update:open="onUpdateOpen">
    <DialogPortal>
      <DialogOverlay
        class="fixed inset-0 z-40 bg-black/40 backdrop-blur-[1px] data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=open]:fade-in-0 data-[state=closed]:fade-out-0"
      />
      <DialogContent
        :class="
          cn(
            'fixed top-0 bottom-0 z-50 flex flex-col border-border bg-background shadow-2xl outline-none transition data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=open]:duration-200 data-[state=closed]:duration-150',
            sideClasses,
            widthClass,
          )
        "
        :trap-focus="!blockAutoClose"
      >
        <slot />
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
