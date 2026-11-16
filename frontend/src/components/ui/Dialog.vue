<script setup lang="ts">
// Shared dialog shell: one 560px column, a title row with the close
// button, a scrolling body and a pinned footer. Tall content never grows
// the dialog past 624px — the header and footer stay, the body scrolls.

import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { X } from "lucide-vue-next";
import {
  DialogClose,
  DialogContent,
  DialogDescription,
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
    /** One muted line under the title. Use the `subtitle` slot for markup. */
    subtitle?: string;
    size?: DialogSize;
    /** Hide the built-in close (X) button — useful for required dialogs. */
    hideClose?: boolean;
    /** A confirmation that interrupts: announced as an alert dialog. */
    alert?: boolean;
  }>(),
  { size: "lg" },
);

const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();

function onUpdate(value: boolean) {
  if (!value) emit("close");
}

const sizeClass = computed(
  () =>
    (
      {
        sm: "w-[400px]",
        md: "w-[560px]",
        lg: "w-[560px]",
        xl: "w-[640px]",
        "2xl": "w-[720px]",
      } as const
    )[props.size],
);
</script>

<template>
  <DialogRoot :open="open" @update:open="onUpdate">
    <DialogPortal>
      <DialogOverlay
        class="fixed inset-0 z-50 bg-dl-scrim data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=open]:fade-in-0 data-[state=closed]:fade-out-0"
      />
      <DialogContent
        :role="alert ? 'alertdialog' : undefined"
        :class="
          cn(
            'fixed left-1/2 top-1/2 z-50 flex max-h-[min(624px,calc(100vh-32px))] max-w-[calc(100vw-32px)] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-2xl border border-border bg-card text-[13px] text-card-foreground shadow-dl-dialog data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=open]:fade-in-0 data-[state=closed]:fade-out-0 data-[state=open]:zoom-in-95 data-[state=closed]:zoom-out-95',
            sizeClass,
          )
        "
      >
        <header
          v-if="title || $slots.header || !hideClose"
          class="flex shrink-0 items-start gap-3 pb-1 pl-6 pr-3.5 pt-[18px]"
        >
          <div class="flex min-w-0 flex-1 flex-col gap-1 pt-1">
            <DialogTitle
              v-if="title"
              class="text-[17px] font-semibold leading-snug tracking-[-0.01em]"
            >
              {{ title }}
            </DialogTitle>
            <slot name="header" />
            <DialogDescription
              v-if="subtitle || $slots.subtitle"
              class="flex min-w-0 items-center gap-2 text-[13px] leading-[1.45] text-muted-foreground"
            >
              <slot name="subtitle">{{ subtitle }}</slot>
            </DialogDescription>
          </div>
          <DialogClose
            v-if="!hideClose"
            class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
            :aria-label="t('common.close')"
            :title="t('common.closeHint')"
          >
            <X class="h-[15px] w-[15px]" aria-hidden="true" />
          </DialogClose>
        </header>
        <div
          class="flex min-h-0 flex-1 flex-col gap-[18px] overflow-y-auto px-6 pb-[22px] pt-4"
        >
          <slot />
        </div>
        <footer
          v-if="$slots.footer"
          class="flex shrink-0 items-center justify-end gap-2 border-t border-dl-line bg-dl-chrome py-3.5 pl-6 pr-4"
        >
          <slot name="footer" />
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
