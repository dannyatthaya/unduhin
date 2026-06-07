// Round-trip + coercion tests for the Torrent settings accessors. The
// settings store's `set` calls `api.setSetting` (Tauri `invoke`), so we mock
// the `api` surface to a no-op and assert the local store mirror + the typed
// getters/setters instead.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

vi.mock("@/types/tauri-bindings", () => ({
  api: {
    getSettings: vi.fn(async () => ({})),
    setSetting: vi.fn(async () => undefined),
  },
}));

import { useSettingsStore } from "@/stores/settings";
import { useTorrentSettings } from "./useTorrentSettings";

/** The store's `set` is async (`await api.setSetting` before mirroring the
 *  value locally) and the typed setters call it fire-and-forget. Flush the
 *  microtask queue so the local mirror is observable before asserting. */
const flush = () => new Promise<void>((r) => setTimeout(r, 0));

describe("useTorrentSettings", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });
  afterEach(() => {
    setActivePinia(undefined);
    vi.clearAllMocks();
  });

  it("returns the seeded defaults when nothing is stored", () => {
    const s = useTorrentSettings();
    expect(s.listenPort.value).toBe(0);
    expect(s.enableDht.value).toBe(true);
    expect(s.enableUpnp.value).toBe(true);
    expect(s.maxPeers.value).toBe(100);
    expect(s.downloadDir.value).toBe("");
    expect(s.seedRatioMilli.value).toBe(0);
  });

  it("reads typed values out of the store", () => {
    const store = useSettingsStore();
    store.values = {
      torrent_listen_port: 51413,
      torrent_enable_dht: false,
      torrent_enable_upnp: false,
      torrent_max_peers: 250,
      torrent_download_dir: "D:\\torrents",
      torrent_seed_ratio_milli: 1500,
    };
    const s = useTorrentSettings();
    expect(s.listenPort.value).toBe(51413);
    expect(s.enableDht.value).toBe(false);
    expect(s.enableUpnp.value).toBe(false);
    expect(s.maxPeers.value).toBe(250);
    expect(s.downloadDir.value).toBe("D:\\torrents");
    expect(s.seedRatioMilli.value).toBe(1500);
  });

  it("falls back to the default when a stored value has the wrong type", () => {
    const store = useSettingsStore();
    store.values = {
      torrent_listen_port: "not-a-number",
      torrent_enable_dht: "yes",
      torrent_max_peers: null,
    };
    const s = useTorrentSettings();
    expect(s.listenPort.value).toBe(0);
    expect(s.enableDht.value).toBe(true);
    expect(s.maxPeers.value).toBe(100);
  });

  it("clamps numeric writes to their bounds and floors fractionals", async () => {
    const store = useSettingsStore();
    const s = useTorrentSettings();

    s.listenPort.value = 70000; // > 65535
    await flush();
    expect(store.values.torrent_listen_port).toBe(65535);

    s.listenPort.value = -5; // < 0
    await flush();
    expect(store.values.torrent_listen_port).toBe(0);

    s.maxPeers.value = 0; // < 1
    await flush();
    expect(store.values.torrent_max_peers).toBe(1);

    s.maxPeers.value = 50.9; // floored
    await flush();
    expect(store.values.torrent_max_peers).toBe(50);

    s.seedRatioMilli.value = -10; // < 0
    await flush();
    expect(store.values.torrent_seed_ratio_milli).toBe(0);
  });

  it("writes boolean + string values straight through", async () => {
    const store = useSettingsStore();
    const s = useTorrentSettings();

    s.enableDht.value = false;
    await flush();
    expect(store.values.torrent_enable_dht).toBe(false);

    s.downloadDir.value = "E:\\dl";
    await flush();
    expect(store.values.torrent_download_dir).toBe("E:\\dl");
  });
});
