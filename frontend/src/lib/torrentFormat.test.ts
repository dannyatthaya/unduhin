// Unit tests for the pure torrent detail-pane formatting + merge helpers.

import { describe, expect, it } from "vitest";

import {
  fileProgressRows,
  formatRatio,
  torrentSourceLabel,
} from "@/lib/torrentFormat";
import type { TorrentFileLive } from "@/stores/downloads";
import type { TorrentMeta } from "@/types/tauri-bindings";

function sampleMeta(overrides: Partial<TorrentMeta> = {}): TorrentMeta {
  return {
    info_hash: "abcdef0123456789abcdef0123456789abcdef01",
    source: { kind: "magnet", uri: "magnet:?xt=urn:btih:abcdef01" },
    selected_files: null,
    files: null,
    swarm: null,
    ...overrides,
  };
}

describe("formatRatio", () => {
  it("formats a milli ratio with two decimals", () => {
    expect(formatRatio(1500)).toBe("1.50");
    expect(formatRatio(0)).toBe("0.00");
    expect(formatRatio(2000)).toBe("2.00");
    expect(formatRatio(333)).toBe("0.33");
  });

  it("clamps null / negative / non-finite to 0.00", () => {
    expect(formatRatio(null)).toBe("0.00");
    expect(formatRatio(undefined)).toBe("0.00");
    expect(formatRatio(-100)).toBe("0.00");
    expect(formatRatio(Number.NaN)).toBe("0.00");
  });
});

describe("torrentSourceLabel", () => {
  it("returns the magnet URI", () => {
    expect(
      torrentSourceLabel({ kind: "magnet", uri: "magnet:?xt=urn:btih:ab" }),
    ).toBe("magnet:?xt=urn:btih:ab");
  });
  it("returns the file path", () => {
    expect(torrentSourceLabel({ kind: "file", path: "C:\\x.torrent" })).toBe(
      "C:\\x.torrent",
    );
  });
  it("returns the bare info hash", () => {
    expect(torrentSourceLabel({ kind: "info_hash", hash: "deadbeef" })).toBe(
      "deadbeef",
    );
  });
});

describe("fileProgressRows", () => {
  it("returns an empty list when metadata has not resolved", () => {
    expect(fileProgressRows(null, undefined)).toEqual([]);
    expect(fileProgressRows(sampleMeta({ files: null }), undefined)).toEqual([]);
    expect(fileProgressRows(sampleMeta({ files: [] }), undefined)).toEqual([]);
  });

  it("renders a 0% shape row before any live tick", () => {
    const meta = sampleMeta({
      files: [{ index: 0, path: "a/x.bin", length: 1000, selected: true }],
    });
    const rows = fileProgressRows(meta, undefined);
    expect(rows).toEqual([
      {
        index: 0,
        path: "a/x.bin",
        length: 1000,
        downloaded: 0,
        pct: 0,
        selected: true,
        done: false,
      },
    ]);
  });

  it("merges live byte counts over the persisted file list and sorts by index", () => {
    const meta = sampleMeta({
      files: [
        { index: 1, path: "b/two.bin", length: 2000, selected: true },
        { index: 0, path: "a/one.bin", length: 1000, selected: true },
      ],
    });
    const live = new Map<number, TorrentFileLive>([
      [0, { index: 0, downloaded: 500, total: 1000 }],
      [1, { index: 1, downloaded: 2000, total: 2000 }],
    ]);
    const rows = fileProgressRows(meta, live);
    expect(rows.map((r) => r.index)).toEqual([0, 1]);
    expect(rows[0]).toMatchObject({ downloaded: 500, pct: 50, done: false });
    expect(rows[1]).toMatchObject({ downloaded: 2000, pct: 100, done: true });
  });

  it("prefers the live total over the persisted length", () => {
    const meta = sampleMeta({
      // persisted length is stale (0); live tick carries the real total.
      files: [{ index: 0, path: "x.bin", length: 0, selected: true }],
    });
    const live = new Map<number, TorrentFileLive>([
      [0, { index: 0, downloaded: 256, total: 1024 }],
    ]);
    const rows = fileProgressRows(meta, live);
    expect(rows[0].pct).toBe(25);
    expect(rows[0].done).toBe(false);
  });

  it("carries the per-file selection flag through", () => {
    const meta = sampleMeta({
      files: [
        { index: 0, path: "keep.bin", length: 10, selected: true },
        { index: 1, path: "skip.bin", length: 10, selected: false },
      ],
    });
    const rows = fileProgressRows(meta, undefined);
    expect(rows[0].selected).toBe(true);
    expect(rows[1].selected).toBe(false);
  });
});
