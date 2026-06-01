// Settings persistence + change-aware reader.
//
// The background modules only need read-on-demand access to the
// `Settings` shape declared in `types.ts`. The options page is what
// *writes* this store; without it, settings live with their defaults
// unless someone toggles them via the devtools console
// (`chrome.storage.sync.set(...)`).
//
// The reader is cache-then-revalidate: synchronous `current()` returns the
// last known value (defaults until the first load resolves), and a
// `chrome.storage.onChanged` listener keeps it fresh. Service-worker
// consumers call `current()` at the moment they need the value — never at
// module load — so hot-applied settings reach the right callsite.

import { log } from "./log.js";
import type {
  ExtensionSettings,
  HostRuleEntry,
  Settings,
  SettingsPatch,
} from "./types.js";

export const SETTINGS_KEY = "settings";

export const DEFAULT_SETTINGS: Settings = {
  enabled: true,
  nativeHostName: "com.unduhin.host",
  minSizeMb: 1,
  extensionAllowlist: [],
  extensionBlocklist: ["html", "pdf", "txt", "json"],
  blockedHosts: [],
  alwaysInterceptHosts: [],
  detectHls: true,
  detectDash: true,
  verboseLogging: false,
  mode: "catch-all",
  installContextMenu: true,
  hideShelf: true,
  forwardCookies: true,
  fileTypes: [],
};

export async function loadSettings(): Promise<Settings> {
  return new Promise((resolve) => {
    chrome.storage.sync.get({ [SETTINGS_KEY]: null }, (items) => {
      const raw = items[SETTINGS_KEY];
      if (!raw || typeof raw !== "object") {
        resolve(DEFAULT_SETTINGS);
        return;
      }
      resolve(mergeWithDefaults(raw as Partial<Settings>));
    });
  });
}

export async function saveSettings(patch: Partial<Settings>): Promise<void> {
  const current = await loadSettings();
  const next: Settings = mergeWithDefaults({ ...current, ...patch });
  return new Promise((resolve) => {
    chrome.storage.sync.set({ [SETTINGS_KEY]: next }, () => resolve());
  });
}

/**
 * Apply a full `ExtensionSettings` snapshot pushed by the Tauri panel
 * via `Outbound::Settings` / `Outbound::SettingsChanged`. Writes
 * through `chrome.storage.sync` so the existing `SettingsReader`
 * hot-applies via `chrome.storage.onChanged`.
 *
 * Dedupes against the current storage shape: if `full` is structurally
 * equal to what we already have, this is a no-op (a fan-out echo from
 * a SetSettings we just sent up). The check is intentionally
 * field-by-field — JSON.stringify drift on array element order would
 * otherwise cause a writeback loop.
 */
export async function applyServerSettings(full: ExtensionSettings): Promise<void> {
  const next = mergeWithDefaults(full as Partial<Settings>);
  const current = await loadSettings();
  if (settingsEqual(current, next)) {
    log.debug("applyServerSettings: incoming settings match storage; no-op");
    return;
  }
  return new Promise((resolve) => {
    chrome.storage.sync.set({ [SETTINGS_KEY]: next }, () => {
      log.debug("applyServerSettings: storage updated from Tauri push");
      resolve();
    });
  });
}

/**
 * Build a `SettingsPatch` carrying every field of the local store.
 * Used by the service worker after a local options-page edit
 * (detected through `chrome.storage.onChanged`) so the Tauri panel
 * sees the user's choice without a round-trip.
 *
 * The patch shape matches the Rust `SettingsPatch`'s `serde(default)`
 * — every field present, none null. The Tauri pipe handler applies it
 * idempotently on top of the cache and broadcasts back to other
 * connected clients (this sender will dedupe via `applyServerSettings`).
 */
export function toSettingsPatch(settings: Settings): SettingsPatch {
  return {
    enabled: settings.enabled,
    nativeHostName: settings.nativeHostName,
    minSizeMb: settings.minSizeMb,
    extensionAllowlist: [...settings.extensionAllowlist],
    extensionBlocklist: [...settings.extensionBlocklist],
    blockedHosts: settings.blockedHosts.map((r) => ({ ...r })),
    alwaysInterceptHosts: settings.alwaysInterceptHosts.map((r) => ({ ...r })),
    detectHls: settings.detectHls,
    detectDash: settings.detectDash,
    verboseLogging: settings.verboseLogging,
    mode: settings.mode,
    installContextMenu: settings.installContextMenu,
    hideShelf: settings.hideShelf,
    forwardCookies: settings.forwardCookies,
    fileTypes: [...settings.fileTypes],
  };
}

function settingsEqual(a: Settings, b: Settings): boolean {
  return (
    a.enabled === b.enabled &&
    a.nativeHostName === b.nativeHostName &&
    a.minSizeMb === b.minSizeMb &&
    a.detectHls === b.detectHls &&
    a.detectDash === b.detectDash &&
    a.verboseLogging === b.verboseLogging &&
    a.mode === b.mode &&
    a.installContextMenu === b.installContextMenu &&
    a.hideShelf === b.hideShelf &&
    a.forwardCookies === b.forwardCookies &&
    arrEq(a.extensionAllowlist, b.extensionAllowlist) &&
    arrEq(a.extensionBlocklist, b.extensionBlocklist) &&
    rulesEq(a.blockedHosts, b.blockedHosts) &&
    rulesEq(a.alwaysInterceptHosts, b.alwaysInterceptHosts) &&
    arrEq(a.fileTypes, b.fileTypes)
  );
}

function arrEq(a: readonly string[], b: readonly string[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

function rulesEq(
  a: readonly HostRuleEntry[],
  b: readonly HostRuleEntry[],
): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i]!;
    const y = b[i]!;
    if (x.pattern !== y.pattern) return false;
    if (x.addedAt !== y.addedAt) return false;
  }
  return true;
}

export interface SettingsReader {
  /** Returns the latest known settings. Cheap and synchronous. */
  current(): Settings;
  /** Resolves once the first load from `chrome.storage.sync` completes. */
  ready: Promise<void>;
  dispose(): void;
}

/**
 * Build a reader that hot-applies storage updates. Consumers should call
 * `.current()` at the moment they need a value, not at module load.
 */
export function createSettingsReader(): SettingsReader {
  let snapshot: Settings = DEFAULT_SETTINGS;
  let resolveReady!: () => void;
  const ready = new Promise<void>((r) => {
    resolveReady = r;
  });

  loadSettings()
    .then((s) => {
      snapshot = s;
      resolveReady();
    })
    .catch((err) => {
      log.warn("settings: initial load failed; using defaults", err);
      resolveReady();
    });

  const onChange = (
    changes: { [key: string]: chrome.storage.StorageChange },
    area: chrome.storage.AreaName,
  ): void => {
    if (area !== "sync") return;
    const entry = changes[SETTINGS_KEY];
    if (!entry) return;
    const next = entry.newValue;
    if (next && typeof next === "object") {
      snapshot = mergeWithDefaults(next as Partial<Settings>);
    } else {
      snapshot = DEFAULT_SETTINGS;
    }
    log.debug("settings updated", snapshot);
  };
  chrome.storage.onChanged.addListener(onChange);

  return {
    current: () => snapshot,
    ready,
    dispose: () => chrome.storage.onChanged.removeListener(onChange),
  };
}

const HANDOFF_MODES = ["catch-all", "ask-first", "rules-only", "passthrough"] as const;
type HandoffModeLiteral = (typeof HANDOFF_MODES)[number];

function mergeWithDefaults(patch: Partial<Settings>): Settings {
  // Defensive: keep array shapes even if storage hands us garbage.
  const arr = (v: unknown, fallback: readonly string[]): readonly string[] =>
    Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : fallback;
  const num = (v: unknown, fallback: number): number =>
    typeof v === "number" && Number.isFinite(v) ? v : fallback;
  const bool = (v: unknown, fallback: boolean): boolean =>
    typeof v === "boolean" ? v : fallback;
  const str = (v: unknown, fallback: string): string =>
    typeof v === "string" && v.length > 0 ? v : fallback;
  const mode = (v: unknown, fallback: HandoffModeLiteral): HandoffModeLiteral =>
    typeof v === "string" && (HANDOFF_MODES as readonly string[]).includes(v)
      ? (v as HandoffModeLiteral)
      : fallback;
  return {
    enabled: bool(patch.enabled, DEFAULT_SETTINGS.enabled),
    nativeHostName: str(patch.nativeHostName, DEFAULT_SETTINGS.nativeHostName),
    minSizeMb: num(patch.minSizeMb, DEFAULT_SETTINGS.minSizeMb),
    extensionAllowlist: arr(patch.extensionAllowlist, DEFAULT_SETTINGS.extensionAllowlist),
    extensionBlocklist: arr(patch.extensionBlocklist, DEFAULT_SETTINGS.extensionBlocklist),
    blockedHosts: rules(patch.blockedHosts, DEFAULT_SETTINGS.blockedHosts),
    alwaysInterceptHosts: rules(
      patch.alwaysInterceptHosts,
      DEFAULT_SETTINGS.alwaysInterceptHosts,
    ),
    detectHls: bool(patch.detectHls, DEFAULT_SETTINGS.detectHls),
    detectDash: bool(patch.detectDash, DEFAULT_SETTINGS.detectDash),
    verboseLogging: bool(patch.verboseLogging, DEFAULT_SETTINGS.verboseLogging),
    mode: mode(patch.mode, DEFAULT_SETTINGS.mode),
    installContextMenu: bool(patch.installContextMenu, DEFAULT_SETTINGS.installContextMenu),
    hideShelf: bool(patch.hideShelf, DEFAULT_SETTINGS.hideShelf),
    forwardCookies: bool(patch.forwardCookies, DEFAULT_SETTINGS.forwardCookies),
    fileTypes: arr(patch.fileTypes, DEFAULT_SETTINGS.fileTypes),
  };
}

/**
 * Migrate / sanitise a host-rule list. Accepts:
 * - the new structured shape `{ pattern, addedAt }[]` — passed through
 *   after pattern/addedAt validation,
 * - the legacy flat `string[]` shape (older storage) — each string is
 *   upgraded to `{ pattern, addedAt: 0 }` so the UI renders "added —"
 *   rather than guessing a date,
 * - anything else (undefined / malformed) — falls back to `fallback`.
 *
 * Deduplicates by pattern (front wins) so a corrupt store doesn't end
 * up with two rows for the same host.
 */
function rules(
  v: unknown,
  fallback: readonly HostRuleEntry[],
): readonly HostRuleEntry[] {
  if (!Array.isArray(v)) return fallback;
  const out: HostRuleEntry[] = [];
  const seen = new Set<string>();
  for (const item of v) {
    let pattern: string | null = null;
    let addedAt = 0;
    if (typeof item === "string") {
      pattern = item.trim();
    } else if (item && typeof item === "object") {
      const obj = item as { pattern?: unknown; addedAt?: unknown };
      if (typeof obj.pattern === "string") pattern = obj.pattern.trim();
      if (typeof obj.addedAt === "number" && Number.isFinite(obj.addedAt)) {
        addedAt = Math.max(0, Math.trunc(obj.addedAt));
      }
    }
    if (!pattern) continue;
    if (seen.has(pattern)) continue;
    seen.add(pattern);
    out.push({ pattern, addedAt });
  }
  return out;
}
