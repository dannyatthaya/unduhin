// Feeds the Settings → Browser status card.
//
// Pulls `get_browser_integration_status` on mount and refreshes
// opportunistically on `pipe_listening` (the listener bound for the
// first time this process) and `download_added` (handoff counters,
// once provenance wiring lands). Surfaces a `testHandoff()` action so the
// status card's "Test handoff" button has somewhere to call.

import { onScopeDispose, ref } from "vue";

import { invoke } from "@tauri-apps/api/core";

import { type CoreEvent, onCoreEvent } from "@/types/tauri-bindings";

export type BrowserId = "chrome" | "edge" | "brave" | "firefox" | "safari";

export type BrowserFamily = "chromium" | "firefox" | "safari";

export interface BrowserRow {
  id: BrowserId;
  label: string;
  family: BrowserFamily;
  installed: boolean;
  host_registered: boolean;
}

export interface PipeStatus {
  /** Bound pipe path, e.g. `\\.\pipe\unduhin`. `null` until the
   *  listener has accepted for the first time this app lifetime. */
  name: string | null;
  listening: boolean;
}

export interface BrowserIntegrationStatus {
  pipe: PipeStatus;
  browsers: BrowserRow[];
  /** ISO-8601 UTC, `null` until the provenance column is wired. */
  last_handoff_at: string | null;
  handoffs_this_week: number;
  handoffs_total: number;
}

export interface PipeHandoffTest {
  round_trip_us: number;
  pipe: string;
}

// Tauri command wrappers — kept inline so the composable stays
// self-contained (mirrors the `useToolingStatus.ts` shape).
function fetchStatus(): Promise<BrowserIntegrationStatus> {
  return invoke<BrowserIntegrationStatus>("get_browser_integration_status");
}

function runPipeHandoffTest(): Promise<PipeHandoffTest> {
  return invoke<PipeHandoffTest>("test_pipe_handoff");
}

export function useBrowserStatus() {
  const status = ref<BrowserIntegrationStatus | null>(null);
  const loading = ref(true);
  const error = ref<string | null>(null);
  const testing = ref(false);

  let unlisten: (() => void) | null = null;

  async function refresh() {
    try {
      status.value = await fetchStatus();
      error.value = null;
    } catch (e: unknown) {
      error.value = (e as { message?: string })?.message ?? String(e);
    } finally {
      loading.value = false;
    }
  }

  async function testHandoff(): Promise<PipeHandoffTest> {
    testing.value = true;
    try {
      const result = await runPipeHandoffTest();
      return result;
    } finally {
      testing.value = false;
    }
  }

  function handle(event: CoreEvent) {
    // The listener bound for the first time — flip from "starting" to
    // "connected" without polling.
    if (event.type === "pipe_listening") {
      void refresh();
      return;
    }
    // Handoff counters only move when a new extension-sourced row
    // lands. The counters are pinned at zero until the provenance
    // column is wired, but wiring the refresh now means it lights up
    // without further changes here.
    if (event.type === "download_added") {
      void refresh();
    }
  }

  void (async () => {
    unlisten = await onCoreEvent(handle);
  })();
  void refresh();

  onScopeDispose(() => {
    if (unlisten) unlisten();
  });

  return {
    status,
    loading,
    error,
    testing,
    refresh,
    testHandoff,
  };
}
