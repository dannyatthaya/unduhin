import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  isMasterPlaylist,
  loadVariants,
  parseMasterPlaylist,
  parseMediaPlaylistDuration,
  peekManifest,
  type ProbeViaApp,
} from "../src/background/hls-master";

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

  it("splits the CODECS attribute into a video and an audio identifier", () => {
    const withCodecs = `#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=2800000,RESOLUTION=1280x720,CODECS="avc1.640028,mp4a.40.2",FRAME-RATE=29.970
720/video.m3u8
`;
    const [variant] = parseMasterPlaylist(withCodecs, BASE);
    expect(variant?.videoCodec).toBe("avc1.640028");
    expect(variant?.audioCodec).toBe("mp4a.40.2");
    expect(variant?.frameRate).toBeCloseTo(29.97);
  });

  it("prefers AVERAGE-BANDWIDTH over the BANDWIDTH peak", () => {
    // The size estimate is bit rate times duration, so a peak-driven
    // estimate overstates a variable-bitrate encode. The mean is what a
    // player advertises as the rendition's real cost.
    const withAverage = `#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=5000000,AVERAGE-BANDWIDTH=3000000,RESOLUTION=1280x720
720/video.m3u8
`;
    const [variant] = parseMasterPlaylist(withAverage, BASE);
    expect(variant?.bandwidth).toBe(3000000);
  });

  it("reads BANDWIDTH correctly when AVERAGE-BANDWIDTH comes first", () => {
    // Guards the `(?<!AVERAGE-)` lookbehind: without it the BANDWIDTH
    // pattern matches the tail of AVERAGE-BANDWIDTH and both reads
    // return the same number.
    const averageFirst = `#EXTM3U
#EXT-X-STREAM-INF:AVERAGE-BANDWIDTH=3000000,BANDWIDTH=5000000,RESOLUTION=1280x720
720/video.m3u8
`;
    const [variant] = parseMasterPlaylist(averageFirst, BASE);
    expect(variant?.bandwidth).toBe(3000000);
  });

  it("leaves size and duration unknown — a master playlist states neither", () => {
    const [variant] = parseMasterPlaylist(MASTER, BASE);
    expect(variant?.durationSecs).toBeNull();
    expect(variant?.estimatedBytes).toBeNull();
  });
});

describe("parseMediaPlaylistDuration", () => {
  it("adds up the EXTINF durations of a finished playlist", () => {
    expect(parseMediaPlaylistDuration(MEDIA)).toBeCloseTo(12);
  });

  it("accepts a VOD playlist type in place of an explicit endlist", () => {
    const vod = `#EXTM3U
#EXT-X-PLAYLIST-TYPE:VOD
#EXTINF:9.009,
a.ts
#EXTINF:9.009,
b.ts
`;
    expect(parseMediaPlaylistDuration(vod)).toBeCloseTo(18.018);
  });

  it("reads an EXTINF that carries a segment title after the comma", () => {
    const titled = `#EXTM3U
#EXTINF:10,Segment one
a.ts
#EXTINF:10,Segment two
b.ts
#EXT-X-ENDLIST
`;
    expect(parseMediaPlaylistDuration(titled)).toBe(20);
  });

  it("returns null for a live playlist", () => {
    // No endlist: the segment list is a rolling window, so summing it
    // would report the window length as the duration of the broadcast.
    const live = `#EXTM3U
#EXT-X-MEDIA-SEQUENCE:9312
#EXTINF:6.0,
s9312.ts
#EXTINF:6.0,
s9313.ts
`;
    expect(parseMediaPlaylistDuration(live)).toBeNull();
  });

  it("returns null for a master playlist and for an empty one", () => {
    expect(parseMediaPlaylistDuration(MASTER)).toBeNull();
    expect(parseMediaPlaylistDuration("#EXTM3U\n#EXT-X-ENDLIST\n")).toBeNull();
  });

  it("skips a malformed EXTINF instead of failing the whole playlist", () => {
    const oneBadLine = `#EXTM3U
#EXTINF:not-a-number,
a.ts
#EXTINF:8,
b.ts
#EXT-X-ENDLIST
`;
    expect(parseMediaPlaylistDuration(oneBadLine)).toBe(8);
  });
});

// `loadVariants` fetches in two possible places: inside the tab (via
// `chrome.scripting.executeScript`, preferred) and directly from the SW
// (via global `fetch`, fallback). Both are opaque boundaries here — the
// injected function is serialised and actually runs in the page's isolated
// world, so there's nothing meaningful to unit-test about its internals
// beyond what `executeScript` resolves/rejects with.
const TAB_ID = 1;

interface FakeChrome {
  executeScript: ReturnType<typeof vi.fn>;
}

function installFakeChrome(): FakeChrome {
  const executeScript = vi.fn();
  (globalThis as unknown as { chrome: unknown }).chrome = {
    scripting: { executeScript },
  };
  return { executeScript };
}

function fakeResponse(ok: boolean, body: string): { ok: boolean; text: () => Promise<string> } {
  return { ok, text: () => Promise.resolve(body) };
}

/** A `fetch` stand-in that never settles on its own — it only rejects once
 * the caller's own `AbortController` fires, exactly like a real hung
 * request does once its deadline passes. Needed because a plain
 * `new Promise(() => {})` wouldn't ever notice the abort and would hang
 * the test forever; real `fetch` always rejects on abort. */
function hangingAbortableFetch(): (
  url: string,
  init?: { signal?: AbortSignal },
) => Promise<never> {
  return (_url, init) =>
    new Promise<never>((_resolve, reject) => {
      init?.signal?.addEventListener("abort", () => reject(new Error("aborted")));
    });
}

// Reused as a stand-in for what the app probe would return — already-parsed
// variants, matching what the native side hands back over the bridge.
const PROBED_VARIANTS = parseMasterPlaylist(MASTER, BASE);

// Comfortably past NEGATIVE_CACHE_TTL_MS (30s) but well short of
// CACHE_TTL_MS (60s), so advancing by this tells the two TTLs apart: a
// negative-cached entry has expired, a real answer has not.
const PAST_NEGATIVE_TTL_MS = 35_000;

// Resolving a master costs TWO `fetchManifest` rounds: one for the master
// body, one for the duration probe against the cheapest rendition. Every
// call count below that mentions a master is doubled for that reason —
// see `probeDuration` in `hls-master.ts` for why one probe covers every
// rendition.
const MASTER_FETCH_ROUNDS = 2;

describe("loadVariants", () => {
  let executeScript: ReturnType<typeof vi.fn>;
  let fetchSpy: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    ({ executeScript } = installFakeChrome());
    fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("uses the in-page body even though the SW tier races alongside it", async () => {
    const url = "https://example.com/loadv/success.m3u8";
    executeScript.mockResolvedValue([{ result: MASTER }]);
    // The case the in-page tier exists for: a hotlink-protected CDN 403s
    // the SW fetch. A `null` can never win the race, so the in-page body
    // is still what comes back.
    fetchSpy.mockResolvedValue(fakeResponse(false, ""));

    const { variants } = await loadVariants(url, TAB_ID);

    expect(variants.map((v) => v.label)).toEqual(["720p", "480p", "360p"]);
  });

  it("falls back to the SW fetch when the in-page fetch fails", async () => {
    const url = "https://example.com/loadv/fallback.m3u8";
    executeScript.mockResolvedValue([{ result: null }]);
    fetchSpy.mockResolvedValue(fakeResponse(true, MASTER));

    const { variants } = await loadVariants(url, TAB_ID);

    expect(variants.map((v) => v.label)).toEqual(["720p", "480p", "360p"]);
    expect(fetchSpy).toHaveBeenCalledTimes(MASTER_FETCH_ROUNDS);
  });

  it("returns no variants when both the in-page and SW fetch fail", async () => {
    const url = "https://example.com/loadv/both-fail.m3u8";
    executeScript.mockResolvedValue([{ result: null }]);
    fetchSpy.mockResolvedValue(fakeResponse(false, ""));

    const info = await loadVariants(url, TAB_ID);

    expect(info.variants).toEqual([]);
    expect(info.durationSecs).toBeNull();
  });

  it("falls back to the SW fetch when executeScript itself hangs", async () => {
    vi.useFakeTimers();
    const url = "https://example.com/loadv/timeout.m3u8";
    // Simulate a stuck injection (suspended tab): a promise that never
    // settles on its own — only the outer timeout guard (the in-page
    // tier's budget plus EXEC_IPC_OVERHEAD_MS) moves on.
    executeScript.mockReturnValue(new Promise(() => {}));
    fetchSpy.mockResolvedValue(fakeResponse(true, MASTER));

    const pending = loadVariants(url, TAB_ID);
    // Comfortably past the in-page tier's outer timeout guard.
    await vi.advanceTimersByTimeAsync(20_000);
    const { variants } = await pending;

    expect(variants.map((v) => v.label)).toEqual(["720p", "480p", "360p"]);
    expect(executeScript).toHaveBeenCalledTimes(MASTER_FETCH_ROUNDS);
    expect(fetchSpy).toHaveBeenCalledTimes(MASTER_FETCH_ROUNDS);
  });

  it("negative-caches a full failure, then retries after the short TTL", async () => {
    vi.useFakeTimers();
    const url = "https://example.com/loadv/negative-cache.m3u8";
    executeScript.mockResolvedValue([{ result: null }]);
    fetchSpy.mockResolvedValue(fakeResponse(false, ""));

    await loadVariants(url, TAB_ID);
    await loadVariants(url, TAB_ID); // within the negative TTL — must not re-fetch.

    expect(executeScript).toHaveBeenCalledTimes(1);
    expect(fetchSpy).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(PAST_NEGATIVE_TTL_MS);
    await loadVariants(url, TAB_ID);

    expect(executeScript).toHaveBeenCalledTimes(2);
    expect(fetchSpy).toHaveBeenCalledTimes(2);
  });

  it("caps an oversized body, caching the empty result for the full success TTL", async () => {
    vi.useFakeTimers();
    const url = "https://example.com/loadv/oversized.m3u8";
    // Well past the 256_000-char cap, and — unlike a real media playlist —
    // deliberately still contains #EXT-X-STREAM-INF, so this only comes
    // back empty because of the size cap, not because it looks like media.
    const oversized =
      "#EXTM3U\n" +
      "#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\n640x360/video.m3u8\n".repeat(5000);
    executeScript.mockResolvedValue([{ result: oversized }]);

    const { variants } = await loadVariants(url, TAB_ID);
    expect(variants).toEqual([]);

    // Past the negative TTL but still within the success TTL — a second
    // call must still hit the cache, proving the oversized result was
    // cached with the long TTL, not the short negative one.
    await vi.advanceTimersByTimeAsync(PAST_NEGATIVE_TTL_MS);
    await loadVariants(url, TAB_ID);

    expect(executeScript).toHaveBeenCalledTimes(1);
  });

  it("behaves exactly as before when probeViaApp is omitted", async () => {
    const url = "https://example.com/loadv/no-probe.m3u8";
    executeScript.mockResolvedValue([{ result: null }]);
    fetchSpy.mockResolvedValue(fakeResponse(false, ""));

    const { variants } = await loadVariants(url, TAB_ID);

    expect(variants).toEqual([]);
  });

  it("falls back to the app probe once both fetches fail", async () => {
    const url = "https://example.com/loadv/probe-success.m3u8";
    executeScript.mockResolvedValue([{ result: null }]);
    fetchSpy.mockResolvedValue(fakeResponse(false, ""));
    const probeViaApp: ProbeViaApp = vi.fn().mockResolvedValue(PROBED_VARIANTS);

    const { variants } = await loadVariants(url, TAB_ID, probeViaApp);

    expect(variants).toEqual(PROBED_VARIANTS);
    expect(probeViaApp).toHaveBeenCalledTimes(1);
    expect(probeViaApp).toHaveBeenCalledWith(url);
  });

  it("takes the duration from the app probe rather than fetching for it", async () => {
    // yt-dlp already reports a duration on every format, so this path
    // must not spend the extra probe fetch a manifest parse needs.
    const url = "https://example.com/loadv/probe-duration.m3u8";
    executeScript.mockResolvedValue([{ result: null }]);
    fetchSpy.mockResolvedValue(fakeResponse(false, ""));
    const probed = PROBED_VARIANTS.map((v) => ({ ...v, durationSecs: 3600 }));
    const probeViaApp: ProbeViaApp = vi.fn().mockResolvedValue(probed);

    const info = await loadVariants(url, TAB_ID, probeViaApp);

    expect(info.durationSecs).toBe(3600);
    // One round only: the master fetch that failed. No duration probe.
    expect(fetchSpy).toHaveBeenCalledTimes(1);
  });

  it("does not call the app probe when the in-page fetch succeeds", async () => {
    const url = "https://example.com/loadv/probe-skip-inpage.m3u8";
    executeScript.mockResolvedValue([{ result: MASTER }]);
    const probeViaApp: ProbeViaApp = vi.fn();

    await loadVariants(url, TAB_ID, probeViaApp);

    expect(probeViaApp).not.toHaveBeenCalled();
  });

  it("does not call the app probe when the SW fetch succeeds", async () => {
    const url = "https://example.com/loadv/probe-skip-sw.m3u8";
    executeScript.mockResolvedValue([{ result: null }]);
    fetchSpy.mockResolvedValue(fakeResponse(true, MASTER));
    const probeViaApp: ProbeViaApp = vi.fn();

    await loadVariants(url, TAB_ID, probeViaApp);

    expect(probeViaApp).not.toHaveBeenCalled();
  });

  it("does not cache a null probe answer as success — a later call retries", async () => {
    vi.useFakeTimers();
    const url = "https://example.com/loadv/probe-null.m3u8";
    executeScript.mockResolvedValue([{ result: null }]);
    fetchSpy.mockResolvedValue(fakeResponse(false, ""));
    const probeViaApp: ProbeViaApp = vi.fn().mockResolvedValue(null);

    await loadVariants(url, TAB_ID, probeViaApp);
    await loadVariants(url, TAB_ID, probeViaApp); // within the negative TTL — must not re-probe.
    expect(probeViaApp).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(PAST_NEGATIVE_TTL_MS);
    await loadVariants(url, TAB_ID, probeViaApp);

    expect(probeViaApp).toHaveBeenCalledTimes(2); // retried — null wasn't cached as success.
  });

  it("caches an empty-array probe answer as a real success", async () => {
    vi.useFakeTimers();
    const url = "https://example.com/loadv/probe-empty.m3u8";
    executeScript.mockResolvedValue([{ result: null }]);
    fetchSpy.mockResolvedValue(fakeResponse(false, ""));
    const probeViaApp: ProbeViaApp = vi.fn().mockResolvedValue([]);

    const first = await loadVariants(url, TAB_ID, probeViaApp);
    expect(first.variants).toEqual([]);

    // Past the negative TTL but still within the success TTL — a second
    // call must still hit the cache, proving the empty array was a real
    // answer cached with the long TTL, not a failure with the short one.
    await vi.advanceTimersByTimeAsync(PAST_NEGATIVE_TTL_MS);
    await loadVariants(url, TAB_ID, probeViaApp);

    expect(probeViaApp).toHaveBeenCalledTimes(1);
  });

  it("still has budget for the app probe when both fetch tiers hang", async () => {
    vi.useFakeTimers();
    const url = "https://example.com/loadv/both-hang.m3u8";
    // Both fetch tiers hang until their own internal timeout fires. They
    // race, so the pair costs the slower one alone (the in-page tier's
    // budget + EXEC_IPC_OVERHEAD_MS) rather than the two in sequence —
    // which is what leaves a real slice of TOTAL_BUDGET_MS for the probe.
    // Run sequentially, this same case exhausted the budget and skipped
    // the probe entirely.
    executeScript.mockReturnValue(new Promise(() => {}));
    fetchSpy.mockImplementation(hangingAbortableFetch());
    const probeViaApp: ProbeViaApp = vi.fn().mockResolvedValue(PROBED_VARIANTS);

    const pending = loadVariants(url, TAB_ID, probeViaApp);
    await vi.advanceTimersByTimeAsync(20_000);
    const { variants } = await pending;

    expect(probeViaApp).toHaveBeenCalledTimes(1);
    expect(variants).toEqual(PROBED_VARIANTS);
  });

  it("shares one run between concurrent calls for the same manifest", async () => {
    const url = "https://example.com/loadv/in-flight.m3u8";
    executeScript.mockResolvedValue([{ result: null }]);
    fetchSpy.mockResolvedValue(fakeResponse(false, ""));
    const probeViaApp: ProbeViaApp = vi.fn().mockResolvedValue(PROBED_VARIANTS);

    const [a, b] = await Promise.all([
      loadVariants(url, TAB_ID, probeViaApp),
      loadVariants(url, TAB_ID, probeViaApp),
    ]);

    expect(a!.variants).toEqual(PROBED_VARIANTS);
    expect(b).toBe(a);
    // The whole tier chain ran once — most importantly the probe, which
    // spawns a subprocess on the native side.
    expect(executeScript).toHaveBeenCalledTimes(1);
    expect(fetchSpy).toHaveBeenCalledTimes(1);
    expect(probeViaApp).toHaveBeenCalledTimes(1);
  });

  it("resolves from the SW fetch without waiting out a hanging in-page tier", async () => {
    vi.useFakeTimers();
    const url = "https://example.com/loadv/race-sw-wins.m3u8";
    executeScript.mockReturnValue(new Promise(() => {}));
    fetchSpy.mockResolvedValue(fakeResponse(true, MASTER));

    const pending = loadVariants(url, TAB_ID);
    // Nowhere near the in-page tier's outer guard (its budget plus
    // EXEC_IPC_OVERHEAD_MS): both rounds are settled by the SW body alone.
    await vi.advanceTimersByTimeAsync(10);
    const { variants } = await pending;

    expect(variants.map((v) => v.label)).toEqual(["720p", "480p", "360p"]);
  });
});

describe("loadVariants duration probe", () => {
  let executeScript: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    ({ executeScript } = installFakeChrome());
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(fakeResponse(false, "")));
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  /** Serve the master for the master URL and the media playlist for
   *  anything else, so the probe round gets a real segment list. */
  function serveMasterThenMedia(masterUrl: string, media = MEDIA): void {
    executeScript.mockImplementation(
      (args: { args: [string, number] }): Promise<{ result: string }[]> =>
        Promise.resolve([{ result: args.args[0] === masterUrl ? MASTER : media }]),
    );
  }

  it("fetches ONE rendition and puts the duration on every variant", async () => {
    const url = "https://example.com/dur/master.m3u8";
    serveMasterThenMedia(url);

    const info = await loadVariants(url, TAB_ID);

    expect(info.durationSecs).toBeCloseTo(12);
    expect(info.variants).toHaveLength(3);
    for (const v of info.variants) expect(v.durationSecs).toBeCloseTo(12);
    // Two rounds total: the master, then one rendition. Not one per
    // rendition — every rendition is the same content and reports the
    // same duration.
    expect(executeScript).toHaveBeenCalledTimes(MASTER_FETCH_ROUNDS);
  });

  it("probes the lowest-bandwidth rendition", async () => {
    const url = "https://example.com/dur/cheapest.m3u8";
    serveMasterThenMedia(url);

    await loadVariants(url, TAB_ID);

    const probedUrl = executeScript.mock.calls[1]![0].args[0];
    expect(probedUrl).toBe("https://example.com/dur/640x360/video.m3u8");
  });

  it("estimates each variant's size from its own bit rate", async () => {
    const url = "https://example.com/dur/sizes.m3u8";
    serveMasterThenMedia(url);

    const { variants } = await loadVariants(url, TAB_ID);

    // 2_800_000 bits/s * 12 s / 8 = 4_200_000 bytes for the 720p row,
    // and proportionally less for the cheaper ones.
    expect(variants[0]!.estimatedBytes).toBe(4_200_000);
    expect(variants[1]!.estimatedBytes).toBe(2_100_000);
    expect(variants[2]!.estimatedBytes).toBe(1_200_000);
  });

  it("leaves size and duration unset when the rendition is live", async () => {
    const url = "https://example.com/dur/live.m3u8";
    const live = `#EXTM3U
#EXT-X-MEDIA-SEQUENCE:44
#EXTINF:6.0,
s44.ts
`;
    serveMasterThenMedia(url, live);

    const info = await loadVariants(url, TAB_ID);

    expect(info.durationSecs).toBeNull();
    for (const v of info.variants) {
      expect(v.durationSecs).toBeNull();
      expect(v.estimatedBytes).toBeNull();
    }
  });

  it("still returns the variants when the duration probe fails", async () => {
    const url = "https://example.com/dur/probe-fails.m3u8";
    executeScript.mockImplementation(
      (args: { args: [string, number] }): Promise<{ result: string | null }[]> =>
        Promise.resolve([{ result: args.args[0] === url ? MASTER : null }]),
    );

    const info = await loadVariants(url, TAB_ID);

    expect(info.variants.map((v) => v.label)).toEqual(["720p", "480p", "360p"]);
    expect(info.durationSecs).toBeNull();
  });

  it("reads a media playlist's duration without any extra fetch", async () => {
    // The manifest the sniffer caught is itself the segment list. The
    // body is already in hand, so no probe round is needed — this is the
    // stream that used to render as a bare URL with no facts at all.
    const url = "https://example.com/dur/plain.m3u8";
    executeScript.mockResolvedValue([{ result: MEDIA }]);

    const info = await loadVariants(url, TAB_ID);

    expect(info.variants).toEqual([]);
    expect(info.durationSecs).toBeCloseTo(12);
    expect(executeScript).toHaveBeenCalledTimes(1);
  });
});

describe("peekManifest", () => {
  beforeEach(() => {
    installFakeChrome();
    vi.stubGlobal("fetch", vi.fn());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("returns undefined for a manifest nothing has resolved yet", () => {
    expect(peekManifest("https://example.com/peek/unknown.m3u8")).toBeUndefined();
  });

  it("returns the cached info once it has resolved", async () => {
    const url = "https://example.com/peek/resolved.m3u8";
    const { executeScript } = installFakeChrome();
    executeScript.mockResolvedValue([{ result: MASTER }]);

    expect(peekManifest(url)).toBeUndefined();
    const loaded = await loadVariants(url, TAB_ID);

    expect(peekManifest(url)).toBe(loaded);
  });

  it("distinguishes an unresolved manifest from one resolved as a plain playlist", async () => {
    const url = "https://example.com/peek/media-playlist.m3u8";
    const { executeScript } = installFakeChrome();
    executeScript.mockResolvedValue([{ result: MEDIA }]);

    await loadVariants(url, TAB_ID);

    // Empty variants — a real answer ("not a master") — not `undefined`.
    expect(peekManifest(url)?.variants).toEqual([]);
  });
});
