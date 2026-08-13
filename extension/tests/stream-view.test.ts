import { describe, expect, it } from "vitest";

import { assembleStreams, sameStreams } from "../src/background/stream-view";
import type { MediaStream, MediaVariant, PopupMediaStream } from "../src/shared/types";

const TAB_ID = 7;
const MASTER_URL = "https://example.com/abc/playlist.m3u8";
const VARIANT_720_URL = "https://example.com/abc/1280x720/video.m3u8";
const VARIANT_480_URL = "https://example.com/abc/842x480/video.m3u8";

function sniffed(manifestUrl: string, over: Partial<MediaStream> = {}): MediaStream {
  return {
    kind: "hls",
    manifestUrl,
    pageUrl: "https://example.com/watch",
    tabId: TAB_ID,
    suggestedFilename: "clip",
    referrer: null,
    userAgent: null,
    cookieHeader: null,
    requestHeaders: [],
    ...over,
  };
}

function variant(url: string, height: number): MediaVariant {
  return { url, height, resolution: `x${height}`, bandwidth: null, label: `${height}p` };
}

const MASTER_VARIANTS = [variant(VARIANT_720_URL, 720), variant(VARIANT_480_URL, 480)];

/** Stand-in for the resolved state: the master knows its qualities. */
const resolved = (url: string): readonly MediaVariant[] =>
  url === MASTER_URL ? MASTER_VARIANTS : [];

/** Stand-in for the cold cache: nothing has resolved yet. */
const unresolved = (): readonly MediaVariant[] => [];

describe("assembleStreams", () => {
  it("attaches a master's qualities and drops the twin row for its rendition", () => {
    // hls.js fetched the master and then the 720p rendition, so the
    // sniffer caught both.
    const streams = assembleStreams(
      [sniffed(MASTER_URL), sniffed(VARIANT_720_URL)],
      TAB_ID,
      resolved,
    );

    expect(streams).toHaveLength(1);
    expect(streams[0]!.manifestUrl).toBe(MASTER_URL);
    expect(streams[0]!.variants).toEqual(MASTER_VARIANTS);
  });

  it("keeps the rendition row while the master is still unresolved", () => {
    // The snapshot fast path: dropping the rendition here would leave the
    // user with a master that has no quality rows to offer yet and no
    // plain row either — i.e. nothing to download.
    const streams = assembleStreams(
      [sniffed(MASTER_URL), sniffed(VARIANT_720_URL)],
      TAB_ID,
      unresolved,
    );

    expect(streams.map((s) => s.manifestUrl)).toEqual([MASTER_URL, VARIANT_720_URL]);
    expect(streams.every((s) => s.variants === undefined)).toBe(true);
  });

  it("omits `variants` entirely rather than emitting an empty array", () => {
    const [stream] = assembleStreams([sniffed(MASTER_URL)], TAB_ID, unresolved);

    expect(stream).not.toHaveProperty("variants");
  });

  it("never consults the variant source for a DASH manifest", () => {
    const url = "https://example.com/abc/manifest.mpd";
    const streams = assembleStreams([sniffed(url, { kind: "dash" })], TAB_ID, () => {
      throw new Error("should not be called for DASH");
    });

    expect(streams.map((s) => s.manifestUrl)).toEqual([url]);
  });

  it("falls back to the supplied tabId when the stream carries none", () => {
    const [stream] = assembleStreams(
      [sniffed(MASTER_URL, { tabId: null })],
      TAB_ID,
      unresolved,
    );

    expect(stream!.tabId).toBe(TAB_ID);
  });
});

describe("sameStreams", () => {
  const plain: PopupMediaStream[] = assembleStreams([sniffed(MASTER_URL)], TAB_ID, unresolved);
  const grouped: PopupMediaStream[] = assembleStreams([sniffed(MASTER_URL)], TAB_ID, resolved);

  it("matches a list against itself", () => {
    expect(sameStreams(plain, plain)).toBe(true);
    expect(sameStreams(grouped, grouped)).toBe(true);
  });

  it("separates a stream that gained quality rows from one that has none", () => {
    expect(sameStreams(plain, grouped)).toBe(false);
  });

  it("separates lists of different length", () => {
    expect(sameStreams(grouped, [])).toBe(false);
  });
});
