// Singleton toast/snackbar system. One <Toaster /> mounts at the root;
// any component can `useToast().push(...)` to surface a transient
// message. No new deps — module-scoped reactive state.

import { ref } from "vue";

export type ToastKind = "info" | "success" | "error";

export interface ToastAction {
  label: string;
  /** Fired when the user clicks the action. The toast is dismissed
   *  automatically afterwards so the same toast can't be acted on
   *  twice. */
  run: () => void;
}

export interface Toast {
  id: number;
  text: string;
  kind: ToastKind;
  /** Optional inline action (e.g. clipboard-capture "Capture"). When
   *  present, the Toaster renders the label as a button. */
  action?: ToastAction;
}

const toasts = ref<Toast[]>([]);
let nextId = 1;
const DEFAULT_TIMEOUT_MS = 3000;

function dismiss(id: number) {
  const i = toasts.value.findIndex((t) => t.id === id);
  if (i !== -1) toasts.value.splice(i, 1);
}

function push(text: string, kind: ToastKind = "info", timeoutMs = DEFAULT_TIMEOUT_MS): number {
  const id = nextId++;
  toasts.value.push({ id, text, kind });
  if (timeoutMs > 0) {
    window.setTimeout(() => dismiss(id), timeoutMs);
  }
  return id;
}

/**
 * Same as [`push`] but attaches an inline action button. The clipboard
 * watcher uses this for the "Capture this?" prompt — the user clicks
 * Capture or lets the toast auto-dismiss to ignore.
 */
function pushAction(
  text: string,
  action: ToastAction,
  kind: ToastKind = "info",
  timeoutMs = DEFAULT_TIMEOUT_MS,
): number {
  const id = nextId++;
  const wrapped: ToastAction = {
    label: action.label,
    run() {
      try {
        action.run();
      } finally {
        dismiss(id);
      }
    },
  };
  toasts.value.push({ id, text, kind, action: wrapped });
  if (timeoutMs > 0) {
    window.setTimeout(() => dismiss(id), timeoutMs);
  }
  return id;
}

export function useToast() {
  return { toasts, push, pushAction, dismiss };
}
