// Unit tests for the pure torrent-input detection / parsing helpers.
//
// These back the AddUrlDialog branch decision and the clipboard-capture
// magnet recognition, so the cases below pin the boundaries: scheme
// case-insensitivity, hex vs base32 info-hashes, `dn=` decoding, `.torrent`
// path/extension detection, and `file:` URL → path conversion.

import { describe, expect, it } from "vitest";

import {
  detectTorrentSource,
  displayNameFromMagnet,
  infoHashFromMagnet,
  isMagnetUri,
  isTorrentFile,
  selectedFileIndices,
} from "@/lib/torrentInput";

const HEX = "0123456789abcdef0123456789abcdef01234567";

describe("isMagnetUri", () => {
  it("recognizes a lowercase magnet scheme", () => {
    expect(isMagnetUri(`magnet:?xt=urn:btih:${HEX}`)).toBe(true);
  });

  it("is case-insensitive on the scheme", () => {
    expect(isMagnetUri(`MAGNET:?xt=urn:btih:${HEX}`)).toBe(true);
  });

  it("trims leading whitespace", () => {
    expect(isMagnetUri(`   magnet:?xt=urn:btih:${HEX}`)).toBe(true);
  });

  it("rejects http and bare strings", () => {
    expect(isMagnetUri("https://example.com/x.zip")).toBe(false);
    expect(isMagnetUri("magnet-link")).toBe(false);
    expect(isMagnetUri("")).toBe(false);
  });
});

describe("isTorrentFile", () => {
  it("recognizes a .torrent path", () => {
    expect(isTorrentFile("C:\\Users\\me\\ubuntu.torrent")).toBe(true);
    expect(isTorrentFile("/home/me/ubuntu.torrent")).toBe(true);
  });

  it("is case-insensitive on the extension", () => {
    expect(isTorrentFile("ubuntu.TORRENT")).toBe(true);
  });

  it("tolerates a trailing query / hash (file: URL form)", () => {
    expect(isTorrentFile("file:///tmp/x.torrent?foo=1")).toBe(true);
  });

  it("rejects non-torrent paths", () => {
    expect(isTorrentFile("ubuntu.iso")).toBe(false);
    expect(isTorrentFile("torrent")).toBe(false);
    expect(isTorrentFile("")).toBe(false);
  });
});

describe("infoHashFromMagnet", () => {
  it("extracts a 40-hex info-hash, lowercased", () => {
    expect(
      infoHashFromMagnet(`magnet:?xt=urn:btih:${HEX.toUpperCase()}&dn=X`),
    ).toBe(HEX);
  });

  it("extracts a 32-char base32 info-hash, lowercased", () => {
    const b32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    expect(infoHashFromMagnet(`magnet:?xt=urn:btih:${b32}`)).toBe(
      b32.toLowerCase(),
    );
  });

  it("scans multiple xt topics for the btih one", () => {
    expect(
      infoHashFromMagnet(`magnet:?xt=urn:ed2k:abc&xt=urn:btih:${HEX}`),
    ).toBe(HEX);
  });

  it("returns null when there's no btih topic", () => {
    expect(infoHashFromMagnet("magnet:?xt=urn:ed2k:abc")).toBeNull();
  });

  it("returns null for a malformed hash", () => {
    expect(infoHashFromMagnet("magnet:?xt=urn:btih:nothex")).toBeNull();
  });

  it("returns null for a non-magnet input", () => {
    expect(infoHashFromMagnet("https://example.com")).toBeNull();
  });
});

describe("displayNameFromMagnet", () => {
  it("decodes the dn= parameter", () => {
    expect(
      displayNameFromMagnet(`magnet:?xt=urn:btih:${HEX}&dn=Ubuntu+24.04`),
    ).toBe("Ubuntu 24.04");
  });

  it("returns null when dn is absent", () => {
    expect(displayNameFromMagnet(`magnet:?xt=urn:btih:${HEX}`)).toBeNull();
  });

  it("returns null for a non-magnet", () => {
    expect(displayNameFromMagnet("not-a-magnet")).toBeNull();
  });
});

describe("detectTorrentSource", () => {
  it("classifies a magnet", () => {
    const uri = `magnet:?xt=urn:btih:${HEX}`;
    expect(detectTorrentSource(uri)).toEqual({ kind: "magnet", uri });
  });

  it("classifies a plain .torrent path", () => {
    expect(detectTorrentSource("C:\\dl\\x.torrent")).toEqual({
      kind: "file",
      path: "C:\\dl\\x.torrent",
    });
  });

  it("decodes a file: URL to a path", () => {
    const src = detectTorrentSource("file:///C:/dl/my%20file.torrent");
    expect(src?.kind).toBe("file");
    expect(src?.kind === "file" && src.path).toBe("C:/dl/my file.torrent");
  });

  it("returns null for an ordinary http URL", () => {
    expect(detectTorrentSource("https://example.com/x.zip")).toBeNull();
  });

  it("returns null for empty input", () => {
    expect(detectTorrentSource("   ")).toBeNull();
  });
});

describe("selectedFileIndices", () => {
  it("returns null when every file is selected (download all)", () => {
    expect(selectedFileIndices(new Set([0, 1, 2]), 3)).toBeNull();
  });

  it("returns the sorted index list for a partial selection", () => {
    expect(selectedFileIndices(new Set([2, 0]), 3)).toEqual([0, 2]);
  });

  it("returns an empty list when nothing is selected", () => {
    // The dialog rejects an empty selection before this point; the helper
    // still returns the empty list rather than null (which would mean "all").
    expect(selectedFileIndices(new Set(), 3)).toEqual([]);
  });

  it("does not treat an empty torrent (0 files) as all-selected", () => {
    expect(selectedFileIndices(new Set(), 0)).toEqual([]);
  });
});
