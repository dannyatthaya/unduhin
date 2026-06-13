import { describe, expect, it } from "vitest";

import { isMasterPlaylist, parseMasterPlaylist } from "../src/background/hls-master";

const MASTER = `#EXTM3U
#EXT-X-VERSION:3
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360
640x360/video.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=1400000,RESOLUTION=842x480
842x480/video.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2800000,RESOLUTION=1280x720
1280x720/video.m3u8
`;

const MEDIA = `#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:6
#EXTINF:6.0,
segment0.ts
#EXTINF:6.0,
segment1.ts
#EXT-X-ENDLIST
`;

const BASE = "https://example.com/abc/playlist.m3u8";

describe("isMasterPlaylist", () => {
  it("detects a master by its stream-inf tag", () => {
    expect(isMasterPlaylist(MASTER)).toBe(true);
    expect(isMasterPlaylist(MEDIA)).toBe(false);
  });
});

describe("parseMasterPlaylist", () => {
  it("parses each rendition, best quality first, with absolute URLs", () => {
    const variants = parseMasterPlaylist(MASTER, BASE);
    expect(variants.map((v) => v.label)).toEqual(["720p", "480p", "360p"]);
    expect(variants[0]).toMatchObject({
      url: "https://example.com/abc/1280x720/video.m3u8",
      height: 720,
      resolution: "1280x720",
      bandwidth: 2800000,
    });
    expect(variants[2]!.url).toBe("https://example.com/abc/640x360/video.m3u8");
  });

  it("returns nothing for a media playlist", () => {
    expect(parseMasterPlaylist(MEDIA, BASE)).toEqual([]);
  });

  it("falls back to a bandwidth label when resolution is absent", () => {
    const noRes = `#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=1500000
audio/only.m3u8
`;
    const [variant] = parseMasterPlaylist(noRes, BASE);
    expect(variant?.label).toBe("1500 kbps");
    expect(variant?.height).toBeNull();
  });
});
