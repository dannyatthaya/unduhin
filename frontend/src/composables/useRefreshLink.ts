// Singleton "Refresh link" dialog state.
//
// A download whose link expired cannot be repaired by Retry: retry re-queues
// the same dead URL with the same stale cookies and fails the same way. The
// row needs a *new* URL, which only the browser can produce.
//
// Two ways to get one, and the dialog runs both at once:
//
//   1. Arm the extension, then wait. The user re-clicks the link on the site,
//      the extension matches the capture by file name and size, and the app
//      folds it into the existing row — keeping the partial file.
//   2. Paste a URL. Always available, needs no extension, and is the only
//      path when the browser integration is not connected.
//
// Mirrors `useDeleteConfirm`: module-level state, one dialog component
// mounted in App.vue, call sites just invoke `requestRefresh(id)`.

import { computed, ref } from "vue";

import { api, type DownloadId, type RefreshOutcome } from "@/types/tauri-bindings";

/** What the dialog is currently showing. */
export type RefreshPhase =
  /** Armed; waiting for the browser capture (paste is also available). */
  | "waiting"
  /** A refresh is in flight. */
  | "working"
  /** The new URL serves a different body. Needs the user's decision. */
  | "changed"
  /** Finished; the dialog reports what happened and closes. */
  | "done";

interface PendingRefresh {
  id: DownloadId;
  filename: string;
  /** Epoch ms after which the extension drops the arm. 0 when arming failed. */
  expiresAtMs: number;
}

const pending = ref<PendingRefresh | null>(null);
const phase = ref<RefreshPhase>("waiting");
const error = ref<string | null>(null);
/** Set when the backend reported a size mismatch, so the dialog can show the
 *  numbers before asking whether to discard the partial file. */
const mismatch = ref<{ oldBytes: number | null; newBytes: number | null } | null>(null);
/** URL that produced the mismatch, replayed verbatim on a forced restart. */
const mismatchUrl = ref<string | null>(null);
const lastOutcome = ref<RefreshOutcome | null>(null);

function reset() {
  pending.value = null;
  phase.value = "waiting";
  error.value = null;
  mismatch.value = null;
  mismatchUrl.value = null;
  lastOutcome.value = null;
}

function messageOf(e: unknown): string {
  return (e as { message?: string })?.message ?? String(e);
}

export function useRefreshLink() {
  /**
   * Open the dialog for `id` and arm the extension.
   *
   * Arming is best-effort: with no extension connected the push goes nowhere,
   * `expiresAtMs` stays 0, and the dialog leans on its paste field instead of
   * promising a capture that will never arrive.
   */
  async function requestRefresh(id: DownloadId, filename: string): Promise<void> {
    reset();
    pending.value = { id, filename, expiresAtMs: 0 };
    try {
      const expiresAtMs = await api.armLinkRefresh(id);
      if (pending.value?.id === id) pending.value = { id, filename, expiresAtMs };
    } catch (e: unknown) {
      // Not fatal — the paste field still works. Record it so the dialog can
      // say the browser path is unavailable rather than silently waiting.
      error.value = messageOf(e);
    }
  }

  /** Apply a URL the user pasted. */
  async function submitUrl(url: string): Promise<void> {
    const req = pending.value;
    if (!req) return;
    phase.value = "working";
    error.value = null;
    try {
      const outcome = await api.refreshDownloadSource(req.id, url, null, false);
      if (outcome.outcome === "source_changed") {
        mismatch.value = { oldBytes: outcome.old_bytes, newBytes: outcome.new_bytes };
        mismatchUrl.value = url;
        phase.value = "changed";
        return;
      }
      lastOutcome.value = outcome;
      phase.value = "done";
    } catch (e: unknown) {
      error.value = messageOf(e);
      phase.value = "waiting";
    }
  }

  /**
   * The user accepted that the file changed. Discard the partial and restart.
   */
  async function confirmRestart(): Promise<void> {
    const req = pending.value;
    const url = mismatchUrl.value;
    if (!req || !url) return;
    phase.value = "working";
    try {
      lastOutcome.value = await api.refreshDownloadSource(req.id, url, null, true);
      phase.value = "done";
    } catch (e: unknown) {
      error.value = messageOf(e);
      phase.value = "changed";
    }
  }

  /**
   * Called when the extension's capture landed and the backend applied it.
   * The pipe handler emits `unduhin:refresh-outcome`; `App.vue` forwards it.
   */
  function applyExternalOutcome(id: DownloadId, outcome: RefreshOutcome, url: string): void {
    if (pending.value?.id !== id) return;
    if (outcome.outcome === "source_changed") {
      mismatch.value = { oldBytes: outcome.old_bytes, newBytes: outcome.new_bytes };
      // Keep the captured URL so "Start again" can re-send it. Nothing was
      // committed, so without this the user would be stuck on a dialog whose
      // only working button is Cancel.
      mismatchUrl.value = url;
      phase.value = "changed";
      return;
    }
    lastOutcome.value = outcome;
    phase.value = "done";
  }

  function close(): void {
    reset();
  }

  return {
    pending: computed(() => pending.value),
    phase: computed(() => phase.value),
    error: computed(() => error.value),
    mismatch: computed(() => mismatch.value),
    canForceRestart: computed(() => mismatchUrl.value !== null),
    lastOutcome: computed(() => lastOutcome.value),
    isOpen: computed(() => pending.value !== null),
    requestRefresh,
    submitUrl,
    confirmRestart,
    applyExternalOutcome,
    close,
  };
}
