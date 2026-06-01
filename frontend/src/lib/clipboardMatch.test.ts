// Table-driven matcher tests for the clipboard watcher.
//
// The matcher decides whether a freshly-copied clipboard payload is
// worth surfacing as a capture toast. The cases below cover the
// boundary conditions we'd otherwise re-discover at runtime: scheme
// gates, multiline guards, allowlist hits/misses, dotless filenames,
// odd-but-legal URLs.

import { describe, expect, it } from "vitest";

import { matchClipboardPayload } from "@/lib/clipboardMatch";

const FILE_TYPES = ["zip", "iso", "mp4", "tar.gz"];

describe("matchClipboardPayload", () => {
  it("captures an https URL whose tail extension is allowlisted", () => {
    const m = matchClipboardPayload(
      "https://example.com/files/setup.zip",
      FILE_TYPES,
    );
    expect(m).not.toBeNull();
    expect(m!.url).toBe("https://example.com/files/setup.zip");
    expect(m!.ext).toBe("zip");
  });

  it("matches mp4 case-insensitively", () => {
    const m = matchClipboardPayload(
      "https://example.com/video.MP4",
      FILE_TYPES,
    );
    expect(m?.ext).toBe("mp4");
  });

  it("trims surrounding whitespace before parsing", () => {
    const m = matchClipboardPayload(
      "   https://example.com/setup.iso  ",
      FILE_TYPES,
    );
    expect(m?.url).toBe("https://example.com/setup.iso");
  });

  it("rejects an HTML page", () => {
    expect(
      matchClipboardPayload(
        "https://example.com/page/index.html",
        FILE_TYPES,
      ),
    ).toBeNull();
  });

  it("rejects a non-allowlisted extension", () => {
    expect(
      matchClipboardPayload("https://example.com/notes.pdf", FILE_TYPES),
    ).toBeNull();
  });

  it("rejects a directory URL with no file tail", () => {
    expect(
      matchClipboardPayload("https://example.com/files/", FILE_TYPES),
    ).toBeNull();
  });

  it("rejects FTP and file:// schemes", () => {
    expect(
      matchClipboardPayload("ftp://example.com/setup.zip", FILE_TYPES),
    ).toBeNull();
    expect(
      matchClipboardPayload("file:///C:/tmp/setup.zip", FILE_TYPES),
    ).toBeNull();
  });

  it("rejects a multiline payload even if a line parses as a URL", () => {
    expect(
      matchClipboardPayload(
        "Here is the link:\nhttps://example.com/setup.zip",
        FILE_TYPES,
      ),
    ).toBeNull();
  });

  it("rejects a payload over the size cap", () => {
    const huge = `https://example.com/${"x".repeat(5000)}.zip`;
    expect(matchClipboardPayload(huge, FILE_TYPES)).toBeNull();
  });

  it("rejects when the allowlist is empty", () => {
    expect(
      matchClipboardPayload("https://example.com/setup.zip", []),
    ).toBeNull();
  });

  it("rejects garbage and empty strings", () => {
    expect(matchClipboardPayload("", FILE_TYPES)).toBeNull();
    expect(matchClipboardPayload("not a url", FILE_TYPES)).toBeNull();
    expect(matchClipboardPayload("   ", FILE_TYPES)).toBeNull();
  });

  it("normalises a leading dot in the allowlist entry", () => {
    expect(
      matchClipboardPayload(
        "https://example.com/setup.zip",
        [".zip"],
      )?.ext,
    ).toBe("zip");
  });

  it("ignores a query-string suffix when extracting the extension", () => {
    const m = matchClipboardPayload(
      "https://example.com/setup.zip?token=abc",
      FILE_TYPES,
    );
    expect(m?.ext).toBe("zip");
  });

  it("rejects suspicious extensions longer than 8 chars", () => {
    expect(
      matchClipboardPayload(
        "https://example.com/file.thisisahugeextension",
        ["thisisahugeextension"],
      ),
    ).toBeNull();
  });
});
