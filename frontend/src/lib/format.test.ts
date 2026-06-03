// Tests for the shared formatting helpers. `truncateFilename` is the one
// with real edge cases — it guards the title bar (and any caller) from the
// 100+ char opaque slugs direct-download hosts hand out.

import { describe, expect, it } from "vitest";

import { truncateFilename } from "@/lib/format";

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
