import { computed, ref } from "vue";
import { useRouter } from "vue-router";

import {
  SETTINGS_INDEX,
  SECTION_LABELS,
  type SettingsIndexEntry,
  type SettingsSectionKey,
} from "@/lib/settingsManifest";

let _instance: ReturnType<typeof create> | null = null;

/**
 * Settings search/filter state. The Settings layout owns the input; every
 * section uses the same instance so the filter persists across navigation
 * within Settings. Singleton scope is fine here — the index is static.
 */
export function useSettingsFilter() {
  if (!_instance) _instance = create();
  return _instance;
}

function create() {
  const router = useRouter();
  const query = ref("");

  function matches(entry: SettingsIndexEntry): boolean {
    const q = query.value.trim().toLowerCase();
    if (!q) return true;
    if (entry.label.toLowerCase().includes(q)) return true;
    if (entry.description.toLowerCase().includes(q)) return true;
    if (SECTION_LABELS[entry.section].toLowerCase().includes(q)) return true;
    if (entry.keywords?.some((k) => k.toLowerCase().includes(q))) return true;
    return false;
  }

  const filtered = computed(() => SETTINGS_INDEX.filter(matches));

  const matchedSections = computed<Set<SettingsSectionKey>>(() => {
    const out = new Set<SettingsSectionKey>();
    for (const entry of filtered.value) out.add(entry.section);
    return out;
  });

  function isHidden(id: string): boolean {
    if (!query.value.trim()) return false;
    return !filtered.value.some((e) => e.id === id);
  }

  async function gotoAndHighlight(entry: SettingsIndexEntry) {
    await router.push(entry.route);
    // The view may mount on the next tick; let the DOM settle.
    await nextRaf();
    await nextRaf();
    const el = document.querySelector<HTMLElement>(
      `[data-setting-id="${cssEscape(entry.id)}"]`,
    );
    if (!el) return;
    el.scrollIntoView({ block: "center", behavior: "smooth" });
    el.dataset.settingsHighlight = "true";
    el.style.setProperty(
      "box-shadow",
      "inset 0 0 0 2px hsl(var(--ring, var(--primary)))",
    );
    setTimeout(() => {
      el.style.removeProperty("box-shadow");
      delete el.dataset.settingsHighlight;
    }, 1500);
  }

  function reset() {
    query.value = "";
  }

  return {
    query,
    matches,
    filtered,
    matchedSections,
    isHidden,
    gotoAndHighlight,
    reset,
  };
}

function nextRaf(): Promise<void> {
  return new Promise((r) => requestAnimationFrame(() => r()));
}

function cssEscape(s: string): string {
  // CSS.escape is widely available in modern browsers; fall back to a
  // conservative replacement for older runtimes.
  if (typeof CSS !== "undefined" && typeof CSS.escape === "function") {
    return CSS.escape(s);
  }
  return s.replace(/["\\]/g, "\\$&");
}
