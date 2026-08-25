import { describe, expect, it } from "vitest";

import { deriveFilename } from "../src/background/media-sniffer";

describe("deriveFilename", () => {
  it("names a rendition after its quality folder", () => {
    // The reported collision: a master whose renditions could not be
    // resolved rendered as two rows both reading "video".
    expect(deriveFilename("https://cdn.example.com/abc/720p/video.m3u8", "hls")).toBe("720p");
    expect(deriveFilename("https://cdn.example.com/abc/480p/video.m3u8", "hls")).toBe("480p");
    expect(deriveFilename("https://cdn.example.com/x/1280x720/index.m3u8", "hls")).toBe(
      "1280x720",
    );
  });

  it("keeps a basename that already says something", () => {
    expect(deriveFilename("https://cdn.example.com/720p/my-show-ep1.m3u8", "hls")).toBe(
      "my-show-ep1",
    );
  });

  it("does not borrow an opaque folder id", () => {
    // A UUID on the row is worse than the generic word it replaced. The
    // service worker already falls back to the page title for these.
    expect(
      deriveFilename("https://surrit.com/db3324d5-6caa-4dac-82e0-78c5fbcaf577/playlist.m3u8", "hls"),
    ).toBe("playlist");
    expect(deriveFilename("https://cdn.example.com/hls/v2/master.m3u8", "hls")).toBe("master");
  });

  it("still strips the manifest extension and handles DASH", () => {
    expect(deriveFilename("https://cdn.example.com/a/clip.mpd", "dash")).toBe("clip");
    expect(deriveFilename("https://cdn.example.com/a/1080p/manifest.mpd", "dash")).toBe("1080p");
  });

  it("survives a query string and percent-encoding", () => {
    expect(
      deriveFilename("https://cdn.example.com/720p/video.m3u8?token=abc&x=1", "hls"),
    ).toBe("720p");
    expect(deriveFilename("https://cdn.example.com/a/My%20Clip.m3u8", "hls")).toBe("My Clip");
  });

  it("returns null for a URL it cannot parse, and the kind for an empty name", () => {
    expect(deriveFilename("not a url", "hls")).toBeNull();
    expect(deriveFilename("https://cdn.example.com/a/.m3u8", "hls")).toBe("hls");
  });
});
