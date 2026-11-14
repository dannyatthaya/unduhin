// Tests for the shared formatting helpers. `truncateFilename` is the one
// with real edge cases — it guards the title bar (and any caller) from the
// 100+ char opaque slugs direct-download hosts hand out.

import { describe, expect, it } from "vitest";

import {
  approxDuration,
  calendarDaysAgo,
  formatAdded,
  formatSpeedLimit,
  hostOf,
  parentFolderLabel,
  splitSpeed,
  truncateFilename,
} from "@/lib/format";

describe("truncateFilename", () => {
  it("leaves short names untouched", () => {
    expect(truncateFilename("clip.mp4")).toBe("clip.mp4");
    expect(truncateFilename("")).toBe("");
  });

  it("never exceeds the cap", () => {
    const slug =
      "BWImVeeBXzQpnkCSnOk7PLUjH_Rp1lbBgmANdOqj8m4DaZBLIcX4htPDGHm4fYWCxquDTcOqT7xkNZ-nO5QX-gvTFMq";
    const out = truncateFilename(slug, 64);
    expect(out.length).toBeLessThanOrEqual(64);
    expect(out).toContain("…");
  });

  it("keeps the extension visible when eliding the middle", () => {
    const name = `${"a".repeat(80)}.mkv`;
    const out = truncateFilename(name, 32);
    expect(out.endsWith(".mkv")).toBe(true);
    expect(out).toContain("…");
    expect(out.length).toBeLessThanOrEqual(32);
  });

  it("respects a custom max", () => {
    expect(truncateFilename("a".repeat(40), 10).length).toBeLessThanOrEqual(10);
  });

  it("does not treat a dotted slug as an extension", () => {
    // A long dotless-but-for-version slug shouldn't lose its tail to a
    // bogus 'extension' — the trailing chunk after the last dot is too long
    // to be one, so the whole string is treated as the stem.
    const name = "report.2026.final.draft.superlongtrailingsegmentwithoutext";
    const out = truncateFilename(name, 30);
    expect(out.length).toBeLessThanOrEqual(30);
    expect(out).toContain("…");
  });
});

describe("hostOf", () => {
  it("returns the hostname of a URL", () => {
    expect(hostOf("https://cdn1.example.com/media/video.mp4")).toBe("cdn1.example.com");
  });

  it("returns an empty string when there is no host", () => {
    expect(hostOf("magnet:?xt=urn:btih:abc")).toBe("");
    expect(hostOf("not a url")).toBe("");
  });
});

describe("parentFolderLabel", () => {
  it("keeps the last two folders of a Windows path", () => {
    expect(parentFolderLabel("C:\\Users\\me\\Downloads\\Video\\clip.mp4")).toBe(
      "Downloads\\Video",
    );
  });

  it("keeps forward slashes on POSIX paths", () => {
    expect(parentFolderLabel("/home/me/Downloads/clip.mp4")).toBe("me/Downloads");
  });

  it("handles shallow and empty paths", () => {
    expect(parentFolderLabel("/tmp/y.bin")).toBe("tmp");
    expect(parentFolderLabel("y.bin")).toBe("");
    expect(parentFolderLabel("")).toBe("");
  });
});

describe("splitSpeed", () => {
  it("separates the number from the unit", () => {
    expect(splitSpeed(10 * 1024 * 1024)).toEqual({ value: "10.0", unit: " MB/s" });
  });

  it("returns a bare dash when idle", () => {
    expect(splitSpeed(0)).toEqual({ value: "—", unit: "" });
    expect(splitSpeed(null)).toEqual({ value: "—", unit: "" });
  });
});

describe("formatSpeedLimit", () => {
  it("drops a trailing .0", () => {
    expect(formatSpeedLimit(10 * 1024 * 1024)).toBe("10 MB/s");
  });

  it("keeps a meaningful decimal", () => {
    expect(formatSpeedLimit(1.5 * 1024 * 1024)).toBe("1.5 MB/s");
  });
});

describe("approxDuration", () => {
  it("uses seconds under a minute, never zero", () => {
    expect(approxDuration(19)).toEqual({ n: 19, unit: "s" });
    expect(approxDuration(0.2)).toEqual({ n: 1, unit: "s" });
  });

  it("uses minutes under an hour", () => {
    expect(approxDuration(197)).toEqual({ n: 3, unit: "min" });
  });

  it("uses hours beyond that", () => {
    expect(approxDuration(2 * 3600 + 100)).toEqual({ n: 2, unit: "h" });
  });
});

describe("formatAdded", () => {
  const now = new Date(2026, 10, 14, 18, 30).getTime();

  it("shows a clock time for today", () => {
    const iso = new Date(2026, 10, 14, 9, 5).toISOString();
    expect(formatAdded(iso, "Yesterday", "en-GB", now)).toBe("09:05");
  });

  it("shows the supplied label for yesterday, even under 24 hours ago", () => {
    const iso = new Date(2026, 10, 13, 23, 50).toISOString();
    expect(calendarDaysAgo(Date.parse(iso), now)).toBe(1);
    expect(formatAdded(iso, "Yesterday", "en-GB", now)).toBe("Yesterday");
  });

  it("shows a short date for older rows, with the year once it differs", () => {
    const thisYear = new Date(2026, 10, 12, 8, 0).toISOString();
    const lastYear = new Date(2025, 10, 12, 8, 0).toISOString();
    expect(formatAdded(thisYear, "Yesterday", "en-GB", now)).toBe("12 Nov");
    expect(formatAdded(lastYear, "Yesterday", "en-GB", now)).toBe("12 Nov 2025");
  });

  it("returns a dash for missing or unparseable dates", () => {
    expect(formatAdded(null, "Yesterday")).toBe("—");
    expect(formatAdded("nope", "Yesterday")).toBe("—");
  });
});
