import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  isMasterPlaylist,
  loadVariants,
  parseMasterPlaylist,
  peekVariants,
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

    const variants = await loadVariants(url, TAB_ID);

    expect(variants.map((v) => v.label)).toEqual(["720p", "480p", "360p"]);
  });

  it("falls back to the SW fetch when the in-page fetch fails", async () => {
    const url = "https://example.com/loadv/fallback.m3u8";
    executeScript.mockResolvedValue([{ result: null }]);
    fetchSpy.mockResolvedValue(fakeResponse(true, MASTER));

    const variants = await loadVariants(url, TAB_ID);

    expect(variants.map((v) => v.label)).toEqual(["720p", "480p", "360p"]);
    expect(fetchSpy).toHaveBeenCalledTimes(1);
  });

  it("returns no variants when both the in-page and SW fetch fail", async () => {
    const url = "https://example.com/loadv/both-fail.m3u8";
    executeScript.mockResolvedValue([{ result: null }]);
    fetchSpy.mockResolvedValue(fakeResponse(false, ""));

    const variants = await loadVariants(url, TAB_ID);

    expect(variants).toEqual([]);
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
    await vi.advanceTimersByTimeAsync(10_000);
    const variants = await pending;

    expect(variants.map((v) => v.label)).toEqual(["720p", "480p", "360p"]);
    expect(executeScript).toHaveBeenCalledTimes(1);
    expect(fetchSpy).toHaveBeenCalledTimes(1);
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

    const variants = await loadVariants(url, TAB_ID);
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

    const variants = await loadVariants(url, TAB_ID);

    expect(variants).toEqual([]);
  });

  it("falls back to the app probe once both fetches fail", async () => {
    const url = "https://example.com/loadv/probe-success.m3u8";
    executeScript.mockResolvedValue([{ result: null }]);
    fetchSpy.mockResolvedValue(fakeResponse(false, ""));
    const probeViaApp: ProbeViaApp = vi.fn().mockResolvedValue(PROBED_VARIANTS);

    const variants = await loadVariants(url, TAB_ID, probeViaApp);

    expect(variants).toEqual(PROBED_VARIANTS);
    expect(probeViaApp).toHaveBeenCalledTimes(1);
    expect(probeViaApp).toHaveBeenCalledWith(url);
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
    expect(first).toEqual([]);

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
    await vi.advanceTimersByTimeAsync(15_000);
    const variants = await pending;

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

    expect(a).toEqual(PROBED_VARIANTS);
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
    // EXEC_IPC_OVERHEAD_MS): the race is settled by the SW body alone.
    await vi.advanceTimersByTimeAsync(10);
    const variants = await pending;

    expect(variants.map((v) => v.label)).toEqual(["720p", "480p", "360p"]);
  });
});

describe("peekVariants", () => {
  beforeEach(() => {
    installFakeChrome();
    vi.stubGlobal("fetch", vi.fn());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("returns undefined for a manifest nothing has resolved yet", () => {
    expect(peekVariants("https://example.com/peek/unknown.m3u8")).toBeUndefined();
  });

  it("returns the cached variants once they have resolved", async () => {
    const url = "https://example.com/peek/resolved.m3u8";
    const { executeScript } = installFakeChrome();
    executeScript.mockResolvedValue([{ result: MASTER }]);

    expect(peekVariants(url)).toBeUndefined();
    const loaded = await loadVariants(url, TAB_ID);

    expect(peekVariants(url)).toBe(loaded);
  });

  it("distinguishes an unresolved manifest from one resolved as a plain playlist", async () => {
    const url = "https://example.com/peek/media-playlist.m3u8";
    const { executeScript } = installFakeChrome();
    executeScript.mockResolvedValue([{ result: MEDIA }]);

    await loadVariants(url, TAB_ID);

    // `[]` — a real answer ("not a master") — not `undefined`.
    expect(peekVariants(url)).toEqual([]);
  });
});
