// Settings search state. The title bar owns the input; the sidebar, the
// results view and every row read the same instance, so one query drives
// all of them. Matching runs on the translated strings the rows show.

import { computed, ref } from "vue";

import { tGlobal } from "@/i18n";
import {
  SECTION_LABEL_KEYS,
  SECTION_ORDER,
  SETTINGS_INDEX,
  type SettingsSectionKey,
} from "@/lib/settingsManifest";
import {
  matchesQuery,
  normalizeQuery,
  suggestQuery,
  type SearchableSetting,
} from "@/lib/settingsSearch";

interface IndexedSetting extends SearchableSetting {
  section: SettingsSectionKey;
}

function create() {
  const query = ref("");

  const searchable = computed<IndexedSetting[]>(() =>
    SETTINGS_INDEX.map((entry) => ({
      id: entry.id,
      section: entry.section,
      texts: [
        tGlobal(entry.labelKey),
        tGlobal(entry.descKey),
        tGlobal(SECTION_LABEL_KEYS[entry.section]),
        ...entry.keywords,
      ],
    })),
  );

  /** True while a query is typed: the results view replaces the section. */
  const active = computed(() => normalizeQuery(query.value) !== "");

  const matched = computed(() =>
    active.value ? searchable.value.filter((s) => matchesQuery(s, query.value)) : [],
  );
  const matchedIds = computed(() => new Set(matched.value.map((s) => s.id)));

  const countBySection = computed(() => {
    const counts = new Map<SettingsSectionKey, number>();
    for (const s of matched.value) counts.set(s.section, (counts.get(s.section) ?? 0) + 1);
    return counts;
  });

  /** Sections with at least one match, in sidebar order. */
  const matchedSections = computed(() =>
    SECTION_ORDER.filter((section) => countBySection.value.has(section)),
  );

  /** For a query with no results: the word it most likely meant. */
  const suggestion = computed(() => {
    if (!active.value || matched.value.length > 0) return null;
    const word = suggestQuery(query.value, searchable.value);
    if (!word) return null;
    const count = searchable.value.filter((s) => matchesQuery(s, word)).length;
    return count > 0 ? { word, count } : null;
  });

  /** A row is hidden while a search is on and it is not among the matches. */
  function isHidden(id: string): boolean {
    return active.value && !matchedIds.value.has(id);
  }

  function reset(): void {
    query.value = "";
  }

  return { query, active, matched, countBySection, matchedSections, suggestion, isHidden, reset };
}

let instance: ReturnType<typeof create> | null = null;

export function useSettingsFilter() {
  instance ??= create();
  return instance;
}
