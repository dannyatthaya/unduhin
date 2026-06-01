// Options page entry. Loads settings on open, binds each input to its
// `data-setting` key, autosaves on blur (text/number/textarea) or change
// (checkbox), and validates fields inline.
//
// "Hot apply" is automatic: the service worker's `SettingsReader` reacts
// to `chrome.storage.onChanged`, and every consumer reads via
// `settings.current()` at decision time. So a save here propagates to
// the interceptor / sniffer / bridge without an extension reload.

import type { Settings } from "../shared/types.js";
import {
  DEFAULT_SETTINGS,
  loadSettings,
  saveSettings,
} from "../shared/settings.js";

type ListStyle = "comma" | "lines";

interface FieldElement extends HTMLElement {
  dataset: DOMStringMap;
}

interface FieldBinding {
  readonly key: keyof Settings;
  readonly element: HTMLInputElement | HTMLTextAreaElement;
  readonly listStyle?: ListStyle;
}

const els = {
  navItems: Array.from(
    document.querySelectorAll<HTMLButtonElement>(".nav__item"),
  ),
  panels: Array.from(document.querySelectorAll<HTMLElement>(".panel")),
  toast: document.querySelector<HTMLSpanElement>("#save-toast")!,
};

let toastTimer: ReturnType<typeof setTimeout> | null = null;

void boot();

async function boot(): Promise<void> {
  installNav();
  const settings = await loadSettings();
  const bindings = collectBindings();
  for (const binding of bindings) {
    hydrateBinding(binding, settings);
    attachAutosave(binding);
  }
}

function installNav(): void {
  for (const item of els.navItems) {
    item.addEventListener("click", () => {
      const target = item.dataset.section;
      if (!target) return;
      for (const navItem of els.navItems) {
        navItem.classList.toggle("is-active", navItem === item);
      }
      for (const panel of els.panels) {
        const isMatch = panel.dataset.section === target;
        panel.hidden = !isMatch;
        panel.classList.toggle("is-active", isMatch);
      }
    });
  }
}

function collectBindings(): FieldBinding[] {
  const nodes = Array.from(
    document.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>(
      "[data-setting]",
    ),
  );
  const out: FieldBinding[] = [];
  for (const el of nodes) {
    const key = (el.dataset.setting ?? "") as keyof Settings;
    if (!(key in DEFAULT_SETTINGS)) continue;
    const listStyle = (el as FieldElement).dataset["listStyle"] as
      | ListStyle
      | undefined;
    out.push({
      key,
      element: el,
      ...(listStyle ? { listStyle } : {}),
    });
  }
  return out;
}

function hydrateBinding(binding: FieldBinding, settings: Settings): void {
  const value = settings[binding.key];
  const el = binding.element;
  if (el instanceof HTMLInputElement && el.type === "checkbox") {
    el.checked = Boolean(value);
    return;
  }
  if (el instanceof HTMLInputElement && el.type === "number") {
    el.value = typeof value === "number" ? String(value) : "";
    return;
  }
  if (el instanceof HTMLTextAreaElement) {
    if (Array.isArray(value)) {
      // blockedHosts / alwaysInterceptHosts are now structured
      // `{pattern, addedAt}` rows. The textarea still edits patterns
      // only; saveSettings → mergeWithDefaults re-attaches addedAt=0
      // for new entries, and the panel preserves the original
      // timestamps for unchanged ones.
      const items: string[] = value.map((item) =>
        typeof item === "string" ? item : (item as { pattern: string }).pattern,
      );
      el.value = binding.listStyle === "lines" ? items.join("\n") : items.join(", ");
    } else {
      el.value = "";
    }
    return;
  }
  el.value = typeof value === "string" ? value : "";
}

function attachAutosave(binding: FieldBinding): void {
  const el = binding.element;
  if (el instanceof HTMLInputElement && el.type === "checkbox") {
    el.addEventListener("change", () => void persistFromBinding(binding));
    return;
  }
  el.addEventListener("blur", () => void persistFromBinding(binding));
  // Number inputs also autosave on Enter — feels right for a single field.
  el.addEventListener("keydown", (event) => {
    if (event instanceof KeyboardEvent && event.key === "Enter") {
      el.blur();
    }
  });
}

async function persistFromBinding(binding: FieldBinding): Promise<void> {
  const next = readBindingValue(binding);
  if (next === null) {
    // Validation already surfaced the inline error.
    return;
  }
  try {
    // blocked/alwaysIntercept hosts edit as patterns in the
    // textarea; promote to structured entries here so the saved shape
    // matches `Settings`. mergeWithDefaults would do the same on
    // re-read, but typing the value as `string[]` while the field is
    // `HostRuleEntry[]` would trip vue-tsc / tsc here.
    const isHostList =
      binding.key === "blockedHosts" || binding.key === "alwaysInterceptHosts";
    const value = isHostList && Array.isArray(next)
      ? promoteHostList(binding.key, await loadSettings(), next as string[])
      : next;
    await saveSettings({ [binding.key]: value } as Partial<Settings>);
    showToast("Saved");
  } catch (err) {
    showToast(err instanceof Error ? err.message : "Save failed", "error");
  }
}

function promoteHostList(
  key: "blockedHosts" | "alwaysInterceptHosts",
  current: Settings,
  patterns: string[],
): Settings[typeof key] {
  const existing = new Map(current[key].map((r) => [r.pattern, r.addedAt]));
  const now = Date.now();
  return patterns.map((pattern) => ({
    pattern,
    addedAt: existing.get(pattern) ?? now,
  })) as unknown as Settings[typeof key];
}

function readBindingValue(binding: FieldBinding): Settings[keyof Settings] | null {
  const el = binding.element;
  clearError(binding);

  if (el instanceof HTMLInputElement && el.type === "checkbox") {
    return el.checked;
  }

  if (el instanceof HTMLInputElement && el.type === "number") {
    const raw = el.value.trim();
    if (raw === "") {
      const fallback = DEFAULT_SETTINGS[binding.key];
      return typeof fallback === "number" ? fallback : 0;
    }
    const parsed = Number(raw);
    if (!Number.isFinite(parsed) || parsed < 0) {
      markError(binding, "Must be a non-negative number.");
      return null;
    }
    return parsed;
  }

  if (el instanceof HTMLTextAreaElement) {
    const items = parseList(el.value, binding.listStyle ?? "lines");
    return items;
  }

  // Text input — single-line string.
  const value = el.value.trim();
  if (binding.key === "nativeHostName" && value.length === 0) {
    markError(binding, "Host name is required.");
    return null;
  }
  return value;
}

function parseList(value: string, style: ListStyle): string[] {
  const splitter = style === "comma" ? /[,\n]/g : /\n/g;
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of value.split(splitter)) {
    const item = raw.trim().toLowerCase();
    if (item.length === 0) continue;
    if (seen.has(item)) continue;
    seen.add(item);
    out.push(item);
  }
  return out;
}

function markError(binding: FieldBinding, message: string): void {
  binding.element.classList.add("is-invalid");
  const node = document.querySelector<HTMLElement>(
    `[data-error-for="${binding.key}"]`,
  );
  if (node) {
    node.textContent = message;
    node.hidden = false;
  }
}

function clearError(binding: FieldBinding): void {
  binding.element.classList.remove("is-invalid");
  const node = document.querySelector<HTMLElement>(
    `[data-error-for="${binding.key}"]`,
  );
  if (node) {
    node.textContent = "";
    node.hidden = true;
  }
}

function showToast(message: string, tone: "info" | "error" = "info"): void {
  els.toast.textContent = message;
  els.toast.dataset.tone = tone;
  els.toast.classList.add("is-visible");
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    els.toast.classList.remove("is-visible");
  }, 1400);
}
