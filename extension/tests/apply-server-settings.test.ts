// `applyServerSettings` writes through to
// `chrome.storage.sync` and dedupes when the incoming snapshot matches
// what's already there (the loop-back from a SetSettings we just sent
// up should not double-write storage).

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// `shared/log.ts` calls `chrome.storage.local.get` at module load, so
// the global must exist BEFORE the dynamic imports below.
interface FakeStorage {
  store: Record<string, unknown>;
  setSpy: ReturnType<typeof vi.fn>;
}

function installFakeChrome(): FakeStorage {
  const fake: FakeStorage = {
    store: {},
    setSpy: vi.fn((items: Record<string, unknown>, cb: () => void) => {
      Object.assign(fake.store, items);
      cb();
    }),
  };
  (globalThis as unknown as { chrome: unknown }).chrome = {
    storage: {
      sync: {
        get: (
          defaults: Record<string, unknown>,
          cb: (items: Record<string, unknown>) => void,
        ) => {
          const result: Record<string, unknown> = { ...defaults };
          for (const key of Object.keys(defaults)) {
            if (key in fake.store) result[key] = fake.store[key];
          }
          cb(result);
        },
        set: fake.setSpy,
      },
      local: {
        // log.ts probes this on boot for the verboseLogging flag.
        get: (
          _defaults: Record<string, unknown>,
          cb: (items: Record<string, unknown>) => void,
        ) => cb({ verboseLogging: false }),
      },
      onChanged: {
        addListener: () => {},
        removeListener: () => {},
      },
    },
  };
  return fake;
}

// Install a baseline chrome stub at module-load time so the static
// imports inside `../src/shared/settings.js` don't blow up. Per-test
// state is replaced in `beforeEach`.
installFakeChrome();

const settingsMod = await import("../src/shared/settings.js");
const { applyServerSettings, DEFAULT_SETTINGS, SETTINGS_KEY, toSettingsPatch } = settingsMod;
type ExtensionSettings = import("../src/shared/types.js").ExtensionSettings;
type Settings = import("../src/shared/types.js").Settings;

const FULL: ExtensionSettings = {
  enabled: true,
  nativeHostName: "com.unduhin.host",
  minSizeMb: 7,
  extensionAllowlist: [],
  extensionBlocklist: ["html", "pdf", "txt", "json"],
  blockedHosts: [],
  alwaysInterceptHosts: [],
  detectHls: true,
  detectDash: true,
  verboseLogging: false,
  mode: "ask-first",
  installContextMenu: false,
  hideShelf: true,
  forwardCookies: true,
  fileTypes: ["zip", "mkv"],
};

describe("applyServerSettings", () => {
  let fake: FakeStorage;

  beforeEach(() => {
    fake = installFakeChrome();
  });

  afterEach(() => {
    delete (globalThis as unknown as { chrome?: unknown }).chrome;
  });

  it("writes through to chrome.storage.sync on first apply", async () => {
    await applyServerSettings(FULL);
    expect(fake.setSpy).toHaveBeenCalledTimes(1);
    const stored = fake.store[SETTINGS_KEY] as Settings;
    expect(stored.mode).toBe("ask-first");
    expect(stored.minSizeMb).toBe(7);
    expect(stored.installContextMenu).toBe(false);
    expect(stored.fileTypes).toEqual(["zip", "mkv"]);
  });

  it("dedupes when incoming snapshot equals current storage", async () => {
    fake.store[SETTINGS_KEY] = { ...FULL };
    await applyServerSettings(FULL);
    expect(fake.setSpy).not.toHaveBeenCalled();
  });

  it("rewrites when a single field differs", async () => {
    fake.store[SETTINGS_KEY] = { ...FULL };
    const next: ExtensionSettings = { ...FULL, mode: "passthrough" };
    await applyServerSettings(next);
    expect(fake.setSpy).toHaveBeenCalledTimes(1);
    const stored = fake.store[SETTINGS_KEY] as Settings;
    expect(stored.mode).toBe("passthrough");
  });

  it("falls back to defaults for missing flat fields", async () => {
    // Simulate an older server pushing a partial shape (e.g. before
    // the flat fields were added). mergeWithDefaults inside
    // applyServerSettings fills the gaps with DEFAULT_SETTINGS.
    const partial = {
      enabled: false,
      nativeHostName: "com.unduhin.host",
      minSizeMb: 1,
      extensionAllowlist: [],
      extensionBlocklist: [],
      blockedHosts: [],
      alwaysInterceptHosts: [],
      detectHls: true,
      detectDash: true,
      verboseLogging: false,
    } as unknown as ExtensionSettings;
    await applyServerSettings(partial);
    const stored = fake.store[SETTINGS_KEY] as Settings;
    expect(stored.enabled).toBe(false);
    expect(stored.mode).toBe(DEFAULT_SETTINGS.mode);
    expect(stored.installContextMenu).toBe(DEFAULT_SETTINGS.installContextMenu);
    expect(stored.fileTypes).toEqual(DEFAULT_SETTINGS.fileTypes);
  });
});

describe("toSettingsPatch", () => {
  it("emits every Settings field on the patch", () => {
    const patch = toSettingsPatch(DEFAULT_SETTINGS);
    expect(Object.keys(patch).sort()).toEqual(
      [
        "alwaysInterceptHosts",
        "blockedHosts",
        "detectDash",
        "detectHls",
        "enabled",
        "extensionAllowlist",
        "extensionBlocklist",
        "fileTypes",
        "forwardCookies",
        "hideShelf",
        "installContextMenu",
        "minSizeMb",
        "mode",
        "nativeHostName",
        "verboseLogging",
      ].sort(),
    );
    expect(patch.mode).toBe("catch-all");
  });
});
