<script setup lang="ts">
import { CheckCircle2, Info, X, XCircle } from "lucide-vue-next";
import {
  ToastAction,
  ToastClose,
  ToastDescription,
  ToastPortal,
  ToastProvider,
  ToastRoot,
  ToastViewport,
} from "reka-ui";

import { useToast } from "@/composables/useToast";

const { toasts, dismiss } = useToast();

const iconFor = {
  info: Info,
  success: CheckCircle2,
  error: XCircle,
} as const;

// `useToast.push()` already schedules its own dismiss timer, so reka-ui's
// auto-dismiss is suppressed (Infinity). Swipe-to-dismiss + manual close
// both flow back through `dismiss(id)` to keep the composable's list in
// sync with what's actually on screen.
function onUpdateOpen(id: number, open: boolean) {
  if (!open) dismiss(id);
}
</script>

<template>
  <ToastProvider swipe-direction="right" :duration="Number.POSITIVE_INFINITY">
    <ToastRoot
      v-for="t in toasts"
      :key="t.id"
      :duration="Number.POSITIVE_INFINITY"
      class="pointer-events-auto flex max-w-sm items-start gap-2 rounded-md border border-border bg-card px-3 py-2 text-sm shadow-lg data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-80 data-[state=open]:slide-in-from-bottom-2 data-[swipe=cancel]:translate-x-0 data-[swipe=end]:translate-x-[var(--reka-toast-swipe-end-x)] data-[swipe=move]:translate-x-[var(--reka-toast-swipe-move-x)] data-[swipe=cancel]:transition-[transform_200ms_ease-out] data-[swipe=end]:animate-out"
      :class="{
        'border-success/40': t.kind === 'success',
        'border-danger/40': t.kind === 'error',
      }"
      @update:open="(o) => onUpdateOpen(t.id, o)"
    >
      <component
        :is="iconFor[t.kind]"
        class="mt-0.5 h-4 w-4 shrink-0"
        :class="{
          'text-success': t.kind === 'success',
          'text-danger': t.kind === 'error',
          'text-muted-foreground': t.kind === 'info',
        }"
      />
      <ToastDescription class="flex-1 text-foreground">
        {{ t.text }}
      </ToastDescription>
      <ToastAction
        v-if="t.action"
        :alt-text="t.action.label"
        class="inline-flex items-center justify-center rounded-md border border-border bg-background px-2 py-1 text-xs font-medium text-foreground transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        @click="t.action.run()"
      >
        {{ t.action.label }}
      </ToastAction>
      <ToastClose
        class="-mr-1 -mt-0.5 inline-flex h-5 w-5 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        aria-label="Dismiss"
      >
        <X class="h-3.5 w-3.5" />
      </ToastClose>
    </ToastRoot>
    <ToastPortal>
      <ToastViewport
        class="pointer-events-none fixed bottom-6 right-6 z-[60] flex max-w-[420px] flex-col items-end gap-2 outline-none"
      />
    </ToastPortal>
  </ToastProvider>
</template>
