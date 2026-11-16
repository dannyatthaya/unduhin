import { describe, expect, test } from "vitest";

import {
  classifyLink,
  extensionOf,
  fileNameFromLink,
  isLocalNetworkLink,
  splitFileName,
} from "./linkInput";

describe("isLocalNetworkLink", () => {
  test.each([
    "http://localhost:8080/a.zip",
    "http://api.localhost/a.zip",
    "http://127.0.0.1/a.zip",
    "http://10.0.0.5/a.zip",
    "http://172.16.4.1/a.zip",
    "http://172.31.255.255/a.zip",
    "http://192.168.1.20:5000/a.zip",
    "http://169.254.169.254/latest/meta-data",
    "http://0.0.0.0/a.zip",
    "http://[::1]/a.zip",
    "http://[fd12:3456::1]/a.zip",
    "http://[fe80::1]/a.zip",
  ])("treats %s as local", (link) => {
    expect(isLocalNetworkLink(link)).toBe(true);
  });

  test.each([
    "https://cdn1.example.com/media/video.mp4",
    "http://8.8.8.8/a.zip",
    "http://172.32.0.1/a.zip",
    "http://192.169.1.1/a.zip",
    "http://[2001:db8::1]/a.zip",
    "https://localhost.example.com/a.zip",
    "not a link",
  ])("treats %s as not local", (link) => {
    expect(isLocalNetworkLink(link)).toBe(false);
  });
});

describe("classifyLink", () => {
  test("treats blank text as empty", () => {
    expect(classifyLink("   ")).toBe("empty");
  });

  test("accepts an https link", () => {
    expect(classifyLink("https://cdn1.example.com/media/video.mp4")).toBe("http");
  });

  test("accepts a link with surrounding space", () => {
    expect(classifyLink("  http://example.com/a.zip \n")).toBe("http");
  });

  test("recognises a magnet link as a torrent", () => {
    expect(classifyLink("magnet:?xt=urn:btih:abcdef0123456789abcdef0123456789abcdef01")).toBe(
      "torrent",
    );
  });

  test("recognises a .torrent path as a torrent", () => {
    expect(classifyLink("C:\\Users\\me\\ubuntu.torrent")).toBe("torrent");
  });

  test("rejects text with spaces in it", () => {
    expect(classifyLink("cdn1 example com/media/video-file-09")).toBe("invalid");
  });

  test("rejects a scheme the engine cannot fetch", () => {
    expect(classifyLink("ftp://example.com/file.bin")).toBe("invalid");
  });

  test("rejects a bare word", () => {
    expect(classifyLink("video-file-09")).toBe("invalid");
  });
});

describe("fileNameFromLink", () => {
  test("returns the decoded last path segment", () => {
    expect(fileNameFromLink("https://example.com/dl/my%20file.zip?t=1")).toBe("my file.zip");
  });

  test("returns an empty string when the link has no path", () => {
    expect(fileNameFromLink("https://example.com/")).toBe("");
  });

  test("returns an empty string for text that is not a link", () => {
    expect(fileNameFromLink("not a link")).toBe("");
  });
});

describe("splitFileName", () => {
  test("splits at the last dot", () => {
    expect(splitFileName("archive.tar.gz")).toEqual({ stem: "archive.tar", extension: ".gz" });
  });

  test("keeps a leading dot in the stem", () => {
    expect(splitFileName(".gitignore")).toEqual({ stem: ".gitignore", extension: "" });
  });

  test("returns no extension for a name without a dot", () => {
    expect(splitFileName("README")).toEqual({ stem: "README", extension: "" });
  });
});

describe("extensionOf", () => {
  test("lower-cases the extension and drops the dot", () => {
    expect(extensionOf("Movie.MP4")).toBe("mp4");
  });

  test("returns an empty string when there is none", () => {
    expect(extensionOf("README")).toBe("");
  });
});
