// Typed accessors over `useSettingsStore` for the Torrent section. Each
// returned ref is writable; writes go to the store (which fires the
// `setting_changed` event and persists through the backend) and propagate
// locally so the UI stays in sync without a round-trip.
//
// Mirrors `useNetworkSettings` / `useGeneralSettings`. The six keys + their
// defaults are seeded by the torrent migration (design §3.G):
//   torrent_listen_port      0      0 = OS-assigned random port
//   torrent_enable_dht       true   required for trackerless magnets
//   torrent_enable_upnp      true   port-map for inbound peers
//   torrent_max_peers        100    per-torrent peer budget
//   torrent_download_dir     ""     empty = fall back to default output path
//   torrent_seed_ratio_milli 0      0 = stop at 100%, no seeding

import { computed, type WritableComputedRef } from "vue";

import { useSettingsStore } from "@/stores/settings";

function typedNumber(
  key: string,
  fallback: number,
  bounds?: { min?: number; max?: number },
): WritableComputedRef<number> {
  const s = useSettingsStore();
  return computed({
    get() {
      const v = s.values[key];
      if (typeof v === "number" && Number.isFinite(v)) return v;
      return fallback;
    },
    set(next) {
      let n = Math.floor(next);
      if (!Number.isFinite(n)) n = fallback;
      if (bounds?.min != null) n = Math.max(bounds.min, n);
      if (bounds?.max != null) n = Math.min(bounds.max, n);
      void s.set(key, n);
    },
  });
}

function typedString(key: string, fallback = ""): WritableComputedRef<string> {
  const s = useSettingsStore();
  return computed({
    get() {
      const v = s.values[key];
      return typeof v === "string" ? v : fallback;
    },
    set(next) {
      void s.set(key, next);
    },
  });
}

function typedBoolean(
  key: string,
  fallback: boolean,
): WritableComputedRef<boolean> {
  const s = useSettingsStore();
  return computed({
    get() {
      const v = s.values[key];
      return typeof v === "boolean" ? v : fallback;
    },
    set(next) {
      void s.set(key, next);
    },
  });
}

export function useTorrentSettings() {
  return {
    listenPort: typedNumber("torrent_listen_port", 0, { min: 0, max: 65535 }),
    enableDht: typedBoolean("torrent_enable_dht", true),
    enableUpnp: typedBoolean("torrent_enable_upnp", true),
    maxPeers: typedNumber("torrent_max_peers", 100, { min: 1, max: 2000 }),
    downloadDir: typedString("torrent_download_dir", ""),
    seedRatioMilli: typedNumber("torrent_seed_ratio_milli", 0, {
      min: 0,
      max: 100_000,
    }),
  };
}
