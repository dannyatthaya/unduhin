import { describe, expect, it } from "vitest";

import {
  codecName,
  formatBitrate,
  formatBytes,
  formatDuration,
  formatFrameRate,
  splitCodecs,
} from "../src/shared/format";

describe("formatBytes", () => {
  it("picks a unit that keeps the number short", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1024 * 40)).toBe("40 KB");
    expect(formatBytes(1024 * 1024 * 412)).toBe("412 MB");
    expect(formatBytes(1024 * 1024 * 1024 * 2.5)).toBe("2.5 GB");
  });

  it("returns null rather than a placeholder for nothing to show", () => {
    // A null tells the popup to omit the chip. A "—" would make the row
    // carry a dash the user has to read past.
    expect(formatBytes(null)).toBeNull();
    expect(formatBytes(0)).toBeNull();
    expect(formatBytes(Number.NaN)).toBeNull();
  });
});

describe("formatDuration", () => {
  it("uses m:ss below an hour and h:mm:ss above it", () => {
    expect(formatDuration(247)).toBe("4:07");
    expect(formatDuration(5020)).toBe("1:23:40");
    expect(formatDuration(59)).toBe("0:59");
  });

  it("rounds to whole seconds", () => {
    expect(formatDuration(12.4)).toBe("0:12");
    expect(formatDuration(12.6)).toBe("0:13");
  });

  it("returns null for a length that is not known", () => {
    expect(formatDuration(null)).toBeNull();
    expect(formatDuration(0)).toBeNull();
  });
});

describe("formatBitrate", () => {
  it("switches to Mbps at a million bits per second", () => {
    expect(formatBitrate(800_000)).toBe("800 kbps");
    expect(formatBitrate(6_000_000)).toBe("6.0 Mbps");
    expect(formatBitrate(2_800_000)).toBe("2.8 Mbps");
  });

  it("returns null when there is no bit rate", () => {
    expect(formatBitrate(null)).toBeNull();
    expect(formatBitrate(0)).toBeNull();
  });
});

describe("formatFrameRate", () => {
  it("keeps two decimals only for a fractional rate", () => {
    expect(formatFrameRate(30)).toBe("30 fps");
    expect(formatFrameRate(29.97)).toBe("29.97 fps");
    expect(formatFrameRate(60.0)).toBe("60 fps");
  });

  it("returns null when there is no frame rate", () => {
    expect(formatFrameRate(null)).toBeNull();
  });
});

describe("codecName", () => {
  it("maps the identifiers both discovery paths produce", () => {
    expect(codecName("avc1.640028")).toBe("H.264");
    expect(codecName("hvc1.1.6.L93.B0")).toBe("H.265");
    expect(codecName("av01.0.08M.08")).toBe("AV1");
    expect(codecName("mp4a.40.2")).toBe("AAC");
    expect(codecName("opus")).toBe("Opus");
    expect(codecName("ec-3")).toBe("E-AC-3");
  });

  it("prefers the longest matching prefix", () => {
    // `mp4a.40.5` is HE-AAC and must not fall back to the plain `mp4a`
    // entry, which would call it AAC.
    expect(codecName("mp4a.40.5")).toBe("HE-AAC");
  });

  it("returns an unknown identifier unchanged", () => {
    // A gap in the table is not a reason to hide the fact from the user.
    expect(codecName("xyz1.2.3")).toBe("xyz1.2.3");
  });

  it("returns null for nothing and for yt-dlp's `none` sentinel", () => {
    expect(codecName(null)).toBeNull();
    expect(codecName("")).toBeNull();
    expect(codecName("none")).toBeNull();
  });
});

describe("splitCodecs", () => {
  it("classifies each entry by its own prefix, not by position", () => {
    expect(splitCodecs("mp4a.40.2,avc1.640028")).toEqual({
      video: "avc1.640028",
      audio: "mp4a.40.2",
    });
  });

  it("handles a video-only rendition", () => {
    // Common on a master that carries audio in a separate EXT-X-MEDIA
    // group.
    expect(splitCodecs("avc1.640028")).toEqual({ video: "avc1.640028", audio: null });
  });

  it("drops an entry that is neither video nor audio", () => {
    // Subtitle and closed-caption codecs appear in CODECS too, and they
    // are not what the row describes.
    expect(splitCodecs("avc1.4d401f,mp4a.40.2,stpp")).toEqual({
      video: "avc1.4d401f",
      audio: "mp4a.40.2",
    });
  });

  it("returns two nulls for a missing attribute", () => {
    expect(splitCodecs(null)).toEqual({ video: null, audio: null });
    expect(splitCodecs("")).toEqual({ video: null, audio: null });
  });
});
