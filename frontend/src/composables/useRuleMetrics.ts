// Live per-rule match counters pushed by the extension's `chrome.alarms`
// tick. The pipe server caches the snapshot and emits
// `CoreEvent::RuleMetricsUpdated` whenever a fresh push lands. The
// composable refreshes on that event plus on mount.

import { computed, onScopeDispose, ref } from "vue";

import { invoke } from "@tauri-apps/api/core";

import { onCoreEvent, type CoreEvent } from "@/types/tauri-bindings";
import type { RuleMetric } from "@/types/wire";

export function useRuleMetrics() {
  const all = ref<RuleMetric[]>([]);
  const loading = ref(true);

  async function refresh(): Promise<void> {
    try {
      all.value = await invoke<RuleMetric[]>("get_rule_metrics");
    } catch (err) {
      console.warn("get_rule_metrics failed", err);
    } finally {
      loading.value = false;
    }
  }

  const byPattern = computed<Record<string, RuleMetric>>(() => {
    const out: Record<string, RuleMetric> = {};
    for (const m of all.value) out[m.pattern] = m;
    return out;
  });

  function getMatchCount(pattern: string): number {
    return byPattern.value[pattern]?.matchCount ?? 0;
  }

  function getLastMatchAt(pattern: string): number | null {
    return byPattern.value[pattern]?.lastMatchAt ?? null;
  }

  let unlisten: (() => void) | null = null;
  function handle(event: CoreEvent): void {
    if (event.type === "rule_metrics_updated" || event.type === "pipe_listening") {
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
    all,
    loading,
    refresh,
    byPattern,
    getMatchCount,
    getLastMatchAt,
  };
}
