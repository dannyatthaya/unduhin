// Singleton bridge for the Rust → Vue confirm-on-quit prompt.
//
// The Rust window handler owns the close-behavior policy now. When it
// needs the user's answer (`close_behavior = "ask"` or
// `confirm_on_quit + exit + has-inflight`), it emits one
// `unduhin:confirm-quit` event with a `request_id`. We mirror it into
// `pending` and the `<ConfirmOnQuitDialog/>` (mounted once in App.vue)
// renders against it. Picking an answer calls
// `api.confirmQuitResponse(request_id, allow)` which unblocks the
// awaiting Rust handler.
//
// Shape mirrors `useDeleteConfirm` — a module-scoped ref, no provider.

import { ref } from "vue";

import {
  api,
  onConfirmQuit,
  type ConfirmQuitRequest,
} from "@/types/tauri-bindings";

const pending = ref<ConfirmQuitRequest | null>(null);

let unlisten: (() => void) | null = null;
let installPromise: Promise<void> | null = null;

/**
 * Idempotent listener installer. Call once on app boot from
 * `App.vue`'s `onMounted`. Calling again returns the same promise.
 */
export function installConfirmOnQuitBridge(): Promise<void> {
  if (installPromise) return installPromise;
  installPromise = (async () => {
    unlisten = await onConfirmQuit((req) => {
      pending.value = req;
    });
  })();
  return installPromise;
}

/** Tear-down hook, used by `onBeforeUnmount` in App.vue. */
export function uninstallConfirmOnQuitBridge() {
  unlisten?.();
  unlisten = null;
  installPromise = null;
  pending.value = null;
}

export function useConfirmOnQuit() {
  async function respond(allow: boolean) {
    const req = pending.value;
    if (!req) return;
    pending.value = null;
    await api.confirmQuitResponse(req.request_id, allow);
  }

  return { pending, respond };
}
