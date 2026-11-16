// The link card in New download: what the text in the Link field is, and
// what the server says about it. One place in the dialog changes what it
// says — checking, ready, not a link, can't reach — instead of stacking
// separate messages.

import { computed, ref, watch, type Ref } from "vue";
import { useDebounceFn } from "@vueuse/core";

import { classifyLink, fileNameFromLink, type LinkKind } from "@/lib/linkInput";
import { hostOf } from "@/lib/format";
import { api, type LinkProbe } from "@/types/tauri-bindings";

/** Pause after the last keystroke before asking the server. */
const PROBE_DEBOUNCE_MS = 450;

export type LinkProbeState =
  | "empty"
  | "invalid"
  | "torrent"
  | "checking"
  | "ready"
  | "unreachable";

export function useLinkProbe(url: Ref<string>, enabled: Ref<boolean>) {
  const result = ref<LinkProbe | null>(null);
  const failed = ref(false);
  const checking = ref(false);
  /** Guards against an older, slower answer overwriting a newer one. */
  let latestRequest = 0;

  const kind = computed<LinkKind>(() => classifyLink(url.value));
  const host = computed(() => result.value?.host ?? hostOf(url.value.trim()));

  const state = computed<LinkProbeState>(() => {
    if (kind.value === "empty") return "empty";
    if (kind.value === "invalid") return "invalid";
    if (kind.value === "torrent") return "torrent";
    if (failed.value) return "unreachable";
    if (result.value) return "ready";
    return "checking";
  });

  /** The server's name for the file, or the link's last path segment. */
  const detectedName = computed(
    () => result.value?.filename ?? fileNameFromLink(url.value),
  );

  async function run() {
    const target = url.value.trim();
    if (!enabled.value || classifyLink(target) !== "http") return;
    const request = ++latestRequest;
    checking.value = true;
    failed.value = false;
    try {
      const probe = await api.probeLink(target);
      if (request !== latestRequest) return;
      result.value = probe;
    } catch {
      if (request !== latestRequest) return;
      // The reason is not shown: the server may refuse a check yet still
      // serve the file, so the card only says it got no answer.
      result.value = null;
      failed.value = true;
    } finally {
      if (request === latestRequest) checking.value = false;
    }
  }

  const runDebounced = useDebounceFn(run, PROBE_DEBOUNCE_MS);

  function clear() {
    latestRequest += 1;
    result.value = null;
    failed.value = false;
    checking.value = false;
  }

  watch([url, enabled], () => {
    clear();
    if (enabled.value && kind.value === "http") void runDebounced();
  });

  return { state, kind, result, host, detectedName, checking, retry: run, clear };
}
