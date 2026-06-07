<script setup lang="ts">
import { computed } from "vue";
import { X } from "lucide-vue-next";
import {
  DialogClose,
  DialogContent,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from "reka-ui";

import { cn } from "@/lib/utils";

type DialogSize = "sm" | "md" | "lg" | "xl" | "2xl";

const props = withDefaults(
  defineProps<{
    open: boolean;
    title?: string;
    size?: DialogSize;
    /** Hide the built-in close (X) button — useful for required dialogs. */
    hideClose?: boolean;
  }>(),
  { size: "lg" },
);

const emit = defineEmits<{ close: [] }>();

function onUpdate(value: boolean) {
  if (!value) emit("close");
}

const sizeClass = computed(
  () =>
    (
      {
        sm: "max-w-sm",
        md: "max-w-md",
        lg: "max-w-lg",
        xl: "max-w-xl",
        "2xl": "max-w-2xl",
      } as const
    )[props.size],
);
</script>

<template>
  <DialogRoot :open="open" @update:open="onUpdate">
    <DialogPortal>
      <DialogOverlay
        class="fixed inset-0 z-50 bg-black/50 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=open]:fade-in-0 data-[state=closed]:fade-out-0"
      />
      <!--
        `grid-cols-[minmax(0,1fr)]` clamps the single grid column to the
        dialog's own width. Without it a grid item defaults to min-width:auto,
        so a long unbreakable string (e.g. the torrent name on the
        "Torrent detected" line, which truncate forces to nowrap) dictates the
        track width and grows the dialog rightward instead of being clipped.
      -->
      <DialogContent
        :class="
          cn(
            'fixed left-1/2 top-1/2 z-50 grid grid-cols-[minmax(0,1fr)] w-full -translate-x-1/2 -translate-y-1/2 gap-0 rounded-lg border border-border bg-card text-card-foreground shadow-xl data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=open]:fade-in-0 data-[state=closed]:fade-out-0 data-[state=open]:zoom-in-95 data-[state=closed]:zoom-out-95',
            sizeClass,
          )
        "
      >
        <header
          v-if="title || $slots.header || !hideClose"
          class="flex items-center justify-between gap-3 border-b border-border px-5 py-3"
        >
          <DialogTitle v-if="title" class="text-base font-semibold">
            {{ title }}
          </DialogTitle>
          <slot name="header" />
          <DialogClose
            v-if="!hideClose"
            class="-mr-1 inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label="Close"
          >
            <X class="h-4 w-4" />
          </DialogClose>
        </header>
        <div class="px-5 py-4">
          <slot />
        </div>
        <footer
          v-if="$slots.footer"
          class="flex justify-end gap-2 border-t border-border px-5 py-3"
        >
          <slot name="footer" />
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
