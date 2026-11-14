import { describe, expect, it } from "vitest";

import { fileKindOf } from "@/lib/fileKind";

describe("fileKindOf", () => {
  it("recognises the common families by extension, case-insensitively", () => {
    expect(fileKindOf("clip.mp4")).toBe("video");
    expect(fileKindOf("Song.FLAC")).toBe("audio");
    expect(fileKindOf("archive-set.part12.rar")).toBe("archive");
    expect(fileKindOf("installer-x64.exe")).toBe("app");
    expect(fileKindOf("report.pdf")).toBe("doc");
    expect(fileKindOf("photo.jpeg")).toBe("image");
    expect(fileKindOf("ubuntu.iso")).toBe("disc");
  });

  it("matches extensions longer than four characters by their cut form", () => {
    expect(fileKindOf("tool.AppImage")).toBe("app");
  });

  it("falls back to a plain file for unknown or missing extensions", () => {
    expect(fileKindOf("bonus-content.bin")).toBe("file");
    expect(fileKindOf("README")).toBe("file");
    expect(fileKindOf("trailing-dot.")).toBe("file");
  });

  it("labels every torrent row as a torrent, whatever its name", () => {
    expect(fileKindOf("Some.Movie.2026.mkv", "torrent")).toBe("torrent");
  });
});
