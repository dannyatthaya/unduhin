import { describe, it, expect } from "vitest";

import {
  DEFAULT_SORT,
  compareRows,
  parseSort,
  sortKey,
  type SortColumn,
  type SortDir,
} from "./useDownloadsSort";
import type { DownloadRecord } from "@/types/tauri-bindings";

function row(over: Partial<DownloadRecord> = {}): DownloadRecord {
  return {
    id: 1,
    url: "https://example.com/x",
    filename: "x.bin",
    output_path: "/tmp/x.bin",
    total_bytes: 1000,
    downloaded_bytes: 0,
    status: "active",
    error: null,
    category_id: null,
    priority: 0,
    segments: 1,
    created_at: "2026-05-20T00:00:00Z",
    completed_at: null,
    etag: null,
    last_modified: null,
    segments_meta: null,
    media_info: null,
    speed_samples: null,
    kind: "http",
    torrent: null,
    ...over,
  };
}

function nullStats() {
  return null;
}

describe("parseSort", () => {
  it("falls back to defaults on null / wrong shape", () => {
    expect(parseSort(null)).toEqual(DEFAULT_SORT);
    expect(parseSort(42)).toEqual(DEFAULT_SORT);
    expect(parseSort({ view: "potato" })).toEqual(DEFAULT_SORT);
  });

  it("accepts a valid object", () => {
    expect(parseSort({ view: "flat", column: "size", dir: "asc" })).toEqual({
      view: "flat",
      column: "size",
      dir: "asc",
    });
  });

  it("repairs an object with one bad field", () => {
    expect(parseSort({ view: "flat", column: "filename", dir: "sideways" }))
      .toEqual({ view: "flat", column: "filename", dir: "desc" });
  });
});

describe("sortKey", () => {
  it("uses lowercase filename for filename column", () => {
    expect(sortKey(row({ filename: "Banana.iso" }), "filename", null)).toBe(
      "banana.iso",
    );
  });

  it("returns 0 for size when total_bytes is null", () => {
    expect(sortKey(row({ total_bytes: null }), "size", null)).toBe(0);
  });

  it("uses status ordinal", () => {
    expect(sortKey(row({ status: "active" }), "status", null))
      .toBeLessThan(sortKey(row({ status: "completed" }), "status", null) as number);
  });

  it("uses speed when stats present, sentinel when absent", () => {
    expect(sortKey(row(), "speed", { speed_bps: 1234, eta: 30 })).toBe(1234);
    expect(sortKey(row(), "speed", null)).toBe(-1);
  });
});

describe("compareRows", () => {
  const cases: Array<{
    label: string;
    a: DownloadRecord;
    b: DownloadRecord;
    column: SortColumn;
    dir: SortDir;
    expect: "a-first" | "b-first";
  }> = [
    {
      label: "filename asc — apple before banana",
      a: row({ id: 1, filename: "apple.iso" }),
      b: row({ id: 2, filename: "banana.iso" }),
      column: "filename",
      dir: "asc",
      expect: "a-first",
    },
    {
      label: "size desc — larger first",
      a: row({ id: 1, total_bytes: 1_000_000 }),
      b: row({ id: 2, total_bytes: 100 }),
      column: "size",
      dir: "desc",
      expect: "a-first",
    },
    {
      label: "added_at desc — newer first",
      a: row({ id: 1, created_at: "2026-05-20T00:00:00Z" }),
      b: row({ id: 2, created_at: "2026-05-19T00:00:00Z" }),
      column: "added_at",
      dir: "desc",
      expect: "a-first",
    },
    {
      label: "status asc — active before completed",
      a: row({ id: 1, status: "active" }),
      b: row({ id: 2, status: "completed" }),
      column: "status",
      dir: "asc",
      expect: "a-first",
    },
  ];

  for (const c of cases) {
    it(c.label, () => {
      const cmp = compareRows(c.a, c.b, c.column, c.dir, nullStats);
      if (c.expect === "a-first") expect(cmp).toBeLessThan(0);
      else expect(cmp).toBeGreaterThan(0);
    });
  }

  it("breaks ties by created_at desc", () => {
    const a = row({ id: 1, filename: "x.iso", created_at: "2026-05-20T00:00:00Z" });
    const b = row({ id: 2, filename: "x.iso", created_at: "2026-05-19T00:00:00Z" });
    // identical primary key (filename) → newer (a) comes first
    expect(compareRows(a, b, "filename", "asc", nullStats)).toBeLessThan(0);
  });
});
