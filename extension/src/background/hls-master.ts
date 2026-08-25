// HLS master-playlist parsing.
//
// A *master* playlist lists alternate renditions via `#EXT-X-STREAM-INF`
// lines, each followed by the URI of a *media* playlist. The media sniffer
// only sees the URL/content-type, so it can't tell a master from a media
// playlist — we fetch the body and look for `#EXT-X-STREAM-INF` to decide,
// then parse out the selectable qualities for the popup.
//
// Results are cached briefly: the popup re-requests the snapshot on every
// open, and re-fetching every manifest each time would be wasteful (and
// the bodies rarely change within a session).

import { log } from "../shared/log.js";
import { splitCodecs } from "../shared/format.js";
import type { MediaVariant } from "../shared/types.js";

const STREAM_INF = "#EXT-X-STREAM-INF:";
// A media playlist states its own length one segment at a time. Summing
// `#EXTINF` is the only way to learn a stream's duration — no HLS tag
// carries a total.
const EXTINF = "#EXTINF:";
// A playlist that has ended states so. Without one of these the segment
// list is still growing (a live stream, or a recording in progress), so
// the sum of what is there now is a floor, not a duration — reporting it
// would show "12:03" for a broadcast that has been running all day.
const ENDLIST = "#EXT-X-ENDLIST";
const VOD_TYPE = "#EXT-X-PLAYLIST-TYPE:VOD";
const CACHE_TTL_MS = 60_000;
// Per-attempt cap for each of the two cheap fetch tiers (in-page, SW) — see
// TOTAL_BUDGET_MS below for how this combines with the third (app-probe)
// tier's own cap.
const FETCH_TIMEOUT_MS = 2_500;
// The in-page fetch runs inside `chrome.scripting.executeScript`, whose own
// promise can hang independently of the injected fetch's own abort signal
// (a suspended/backgrounded tab, or injection itself stalling, or just the
// IPC round-trip to marshal the already-resolved result back across the
// isolated-world boundary). This is added on top of whatever timeout the
// in-page tier was given, so the inner fetch's own timeout fires and
// returns null cleanly first, in the common case.
const EXEC_IPC_OVERHEAD_MS = 1_000;
// A failed fetch (both in-page and SW) is cached too — otherwise every
// popup open re-injects a script into the tab for a manifest that just
// 403'd. Shorter than CACHE_TTL_MS so a transient failure (page not ready
// yet, blip in the CDN) still retries. A `null` from the app probe
// (couldn't answer) uses the same TTL — only an actual answer (including
// an empty array — "answered, no variants") earns the long one.
//
// This was 5s back when resolution ran on the popup's render path, chosen
// so a transient failure retried almost immediately. It no longer needs to
// be that eager: retries now happen in the background (see
// `resolveVariantsForTab` in the service worker) where they cost the user
// nothing, while a TTL that short guaranteed a full re-pay of the whole
// tier chain on nearly every popup open.
const NEGATIVE_CACHE_TTL_MS = 30_000;
// A media playlist can be megabytes of segment lines; fetching it is
// already sunk once we're here, but parsing it for nothing is not. Master
// playlists are small (a handful of lines per rendition) even with many
// qualities, so anything past this is almost certainly a media playlist.
const MAX_MANIFEST_LENGTH_CHARS = 256_000;
// Wall-clock ceiling for one `loadVariants` call, covering ALL tiers
// combined. Nothing user-facing awaits this any more (the popup renders
// from `peekVariants` and gets a broadcast when resolution lands), but a
// bound is still what stops a wedged tab from pinning a `loadVariants`
// call — and its cache slot — open forever.
//
// It splits into two halves. The two cheap fetch tiers race, so their
// worst case is the slower one alone: FETCH_TIMEOUT_MS +
// EXEC_IPC_OVERHEAD_MS (the in-page tier). Whatever's left —
// FETCH_TIMEOUT_MS — is the probe's slice. Before the tiers raced, the
// pathological "everything is hanging" case burned the entire budget on
// the fetches and starved the probe to zero; now the probe always gets a
// real slice to work with.
// One more `FETCH_TIMEOUT_MS` on top of the two halves above, for the
// duration probe (see `probeDuration`). It runs after the master has
// parsed, so it cannot overlap with the tiers that fetched the master.
const TOTAL_BUDGET_MS =
  FETCH_TIMEOUT_MS + EXEC_IPC_OVERHEAD_MS + FETCH_TIMEOUT_MS + FETCH_TIMEOUT_MS;

/**
 * What resolution learned about one manifest URL.
 *
 * `variants` is empty for a media playlist — a stream with no alternate
 * renditions to choose between. `durationSecs` is still populated in that
 * case, which is the point of this being a record rather than a bare
 * variant list: a plain stream row has no variant to hang a duration on,
 * but the user still wants to know how long the video runs before
 * starting it.
 */
export interface ManifestInfo {
  readonly variants: readonly MediaVariant[];
  /** Length of the media in seconds, or null when it is not known —
   *  a live stream, or a manifest no tier could fetch. */
  readonly durationSecs: number | null;
}

/** The value cached for a manifest nothing could be learned about. */
const UNKNOWN: ManifestInfo = { variants: [], durationSecs: null };

interface CacheEntry {
  readonly info: ManifestInfo;
  readonly expiresAt: number;
  /**
   * True when NOTHING could answer — every tier failed or timed out.
   *
   * Deliberately distinct from a real answer that happens to hold no
   * variants. The two look identical in `info`, but they must not behave
   * identically: a real answer is final, while a failure is worth trying
   * again the moment a user actually looks.
   *
   * This matters because the warm-up runs at the worst possible time.
   * `onStreamDetected` fires from a `webRequest` listener the instant the
   * manifest response starts, when the page is still committing and the
   * tab may not accept an injected script yet. A failure captured at that
   * moment used to be cached like any other answer, so a popup opened
   * seconds later rendered a plain row and never retried — the manifest
   * was perfectly fetchable by then.
   */
  readonly failed: boolean;
}

const cache = new Map<string, CacheEntry>();

// Calls that have started but not yet cached a result, keyed by manifest
// URL. Without this, two overlapping `loadVariants` for the same manifest
// both miss the cache and both run the whole tier chain — including the
// subprocess-spawning app probe. That overlap is routine now: the sniffer
// warms the cache the moment a manifest is seen (`onStreamDetected` in the
// service worker) and the popup's background resolve can start while that
// warm-up is still in flight.
const inFlight = new Map<string, Promise<ManifestInfo>>();

/**
 * Last-resort variant source, injected by the caller rather than imported —
 * this module stays decoupled from the native-messaging bridge (which
 * lives in the service worker that imports `loadVariants`, so importing
 * bridge/wire types back here would be circular). The implementation
 * spawns a yt-dlp subprocess on the native side, so it's slow relative to
 * the two fetch tiers; `loadVariants` only reaches for it once those have
 * both failed. Returns already-parsed variants (the native side owns the
 * parsing), or `null` if it can't help — bridge down, host error,
 * unsupported URL. `null` is distinct from an empty array: `null` means
 * "couldn't answer" (negative-cached, retried soon), an empty array means
 * "answered, no variants" (cached like any other success).
 */
export type ProbeViaApp = (
  manifestUrl: string,
) => Promise<readonly MediaVariant[] | null>;

/** True if the playlist body advertises alternate renditions. */
export function isMasterPlaylist(text: string): boolean {
  return text.includes(STREAM_INF);
}

/**
 * Parse a master playlist into its selectable qualities, best (tallest)
 * first. `baseUrl` resolves relative variant URIs. A body with no
 * `#EXT-X-STREAM-INF` (i.e. a media playlist) yields an empty array.
 */
export function parseMasterPlaylist(text: string, baseUrl: string): MediaVariant[] {
  const lines = text.split(/\r?\n/);
  const variants: MediaVariant[] = [];

  for (let i = 0; i < lines.length; i += 1) {
    const line = (lines[i] ?? "").trim();
    if (!line.startsWith(STREAM_INF)) continue;

    const attrs = line.slice(STREAM_INF.length);
    // Match attributes directly rather than comma-splitting — quoted
    // values (e.g. CODECS="avc1.4d401f,mp4a.40.2") contain commas.
    const res = /RESOLUTION=(\d+)x(\d+)/i.exec(attrs);
    // AVERAGE-BANDWIDTH is the rendition's mean bit rate; BANDWIDTH is
    // its peak. Prefer the mean: this number drives the size estimate,
    // and a peak-driven estimate overstates a variable-bitrate encode by
    // a wide margin. The `(?<!AVERAGE-)` guard stops the BANDWIDTH
    // pattern from matching the tail of AVERAGE-BANDWIDTH.
    const avgBw = /AVERAGE-BANDWIDTH=(\d+)/i.exec(attrs);
    const bw = /(?<!AVERAGE-)BANDWIDTH=(\d+)/i.exec(attrs);
    const codecs = /CODECS="([^"]*)"/i.exec(attrs);
    const frameRateMatch = /FRAME-RATE=([\d.]+)/i.exec(attrs);
    const height = res ? Number.parseInt(res[2]!, 10) : null;
    const resolution = res ? `${res[1]}x${res[2]}` : null;
    const bandwidth = avgBw
      ? Number.parseInt(avgBw[1]!, 10)
      : bw
        ? Number.parseInt(bw[1]!, 10)
        : null;
    const { video: videoCodec, audio: audioCodec } = splitCodecs(codecs?.[1] ?? null);
    const parsedRate = frameRateMatch ? Number.parseFloat(frameRateMatch[1]!) : NaN;
    const frameRate = Number.isFinite(parsedRate) && parsedRate > 0 ? parsedRate : null;

    // The variant URI is the next non-blank, non-comment line.
    let uri: string | null = null;
    for (let j = i + 1; j < lines.length; j += 1) {
      const next = (lines[j] ?? "").trim();
      if (next.length === 0 || next.startsWith("#")) continue;
      uri = next;
      i = j;
      break;
    }
    if (!uri) continue;

    let url: string;
    try {
      url = new URL(uri, baseUrl).href;
    } catch {
      continue; // malformed URI — skip this rendition, keep the rest.
    }

    variants.push({
      url,
      height,
      resolution,
      bandwidth,
      label: labelFor(height, bandwidth, resolution),
      videoCodec,
      audioCodec,
      frameRate,
      // A master playlist states neither. `withDuration` fills both in
      // once the duration probe has run.
      durationSecs: null,
      estimatedBytes: null,
    });
  }

  const seen = new Set<string>();
  return variants
    .filter((v) => (seen.has(v.url) ? false : (seen.add(v.url), true)))
    .sort((a, b) => (b.height ?? 0) - (a.height ?? 0) || (b.bandwidth ?? 0) - (a.bandwidth ?? 0));
}

/**
 * Total length in seconds of a *media* playlist, by adding up its
 * `#EXTINF` segment durations.
 *
 * Returns null when the playlist has not ended. A live stream has no
 * total length, and its segment list is a rolling window, so the sum of
 * what the manifest lists right now is not the duration of anything.
 * Also returns null for a playlist with no segments, and for a master
 * playlist (which has no `#EXTINF` lines at all).
 *
 * The `#EXTINF` value is a decimal number, then an optional comma and an
 * optional title: `#EXTINF:9.009,` or `#EXTINF:10,Segment 3`.
 */
export function parseMediaPlaylistDuration(text: string): number | null {
  if (!text.includes(ENDLIST) && !text.includes(VOD_TYPE)) return null;

  let total = 0;
  let segments = 0;
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line.startsWith(EXTINF)) continue;
    const value = line.slice(EXTINF.length).split(",")[0]?.trim() ?? "";
    const secs = Number.parseFloat(value);
    // A malformed `#EXTINF` skips that segment rather than failing the
    // whole playlist: one bad line should not cost the other 900.
    if (!Number.isFinite(secs) || secs < 0) continue;
    total += secs;
    segments += 1;
  }
  return segments > 0 && total > 0 ? total : null;
}

/**
 * Transfer size implied by holding `bandwidth` bits per second for
 * `durationSecs`.
 *
 * An estimate, and the popup says so. HLS states a bit rate and never a
 * size, and a real encode varies around its advertised rate. Mirrors
 * `estimate_bytes` in `crates/core/src/ytdlp/wire.rs` so the two
 * discovery paths produce the same number for the same stream.
 */
function estimateBytes(
  bandwidth: number | null,
  durationSecs: number | null,
): number | null {
  if (!bandwidth || bandwidth <= 0) return null;
  if (!durationSecs || durationSecs <= 0) return null;
  return Math.round((bandwidth * durationSecs) / 8);
}

/** Copy of `variant` carrying `durationSecs` and the size that follows
 *  from it. A null duration leaves both fields null. */
function withDuration(variant: MediaVariant, durationSecs: number | null): MediaVariant {
  return {
    ...variant,
    durationSecs,
    estimatedBytes: estimateBytes(variant.bandwidth, durationSecs),
  };
}

/**
 * Learn a master playlist's duration by fetching exactly ONE of its
 * renditions and summing that media playlist's segments.
 *
 * One fetch, not one per rendition: every rendition of an adaptive
 * stream is the same content at a different bit rate, so they all report
 * the same duration. Fetching six playlists to read the same number six
 * times would add five requests per captured stream for nothing.
 *
 * The rendition picked is the lowest-bandwidth one. That is a tie-break
 * and not an optimization — the playlists are the same length in lines
 * whatever the bit rate — but it does ask the CDN for its cheapest
 * object, and it makes the choice deterministic for the tests.
 *
 * Returns null on any failure. A missing duration costs the row two
 * chips. It must never cost the row itself, so no error escapes here.
 */
async function probeDuration(
  variants: readonly MediaVariant[],
  tabId: number | null,
  deadline: number,
): Promise<number | null> {
  const cheapest = variants.reduce<MediaVariant | null>((best, v) => {
    if (!best) return v;
    return (v.bandwidth ?? Number.MAX_SAFE_INTEGER) <
      (best.bandwidth ?? Number.MAX_SAFE_INTEGER)
      ? v
      : best;
  }, null);
  if (!cheapest) return null;

  try {
    const body = await fetchManifest(cheapest.url, tabId, deadline);
    return body == null ? null : parseMediaPlaylistDuration(body);
  } catch {
    return null;
  }
}

/**
 * Fetch `manifestUrl` and, if it's a master playlist, return its qualities.
 * The two cheap fetch tiers — in-page and a direct service-worker fetch —
 * race, and only if neither produced a body does the expensive native-app
 * probe run (when `probeViaApp` is supplied). The whole call is bounded by
 * {@link TOTAL_BUDGET_MS} — see the tier budgeting in {@link fetchManifest}
 * and {@link fetchAppProbe}. `probeViaApp` omitted behaves exactly as the
 * two-tier fetch chain did before it existed.
 *
 * Concurrent calls for the same URL share one run; use {@link peekManifest}
 * for a non-blocking read of what's already resolved.
 *
 * `variants` comes back empty for media playlists, for an app probe that
 * answered "no variants", and (once every available tier has failed or
 * timed out) as the fallback — callers treat "no variants" as "plain
 * stream" either way. `durationSecs` is populated whenever some tier
 * could read it, INCLUDING for a media playlist with no variants at all.
 *
 * Cached for {@link CACHE_TTL_MS} on an actual answer, {@link
 * NEGATIVE_CACHE_TTL_MS} when nothing could answer at all.
 *
 * `retryFailed` re-runs the tier chain even when a cached entry exists,
 * as long as that entry is a recorded FAILURE rather than a real answer.
 * The popup passes it; the sniffer's warm-up does not. A real answer is
 * never re-fetched either way, so this cannot turn into hammering.
 */
export async function loadVariants(
  manifestUrl: string,
  tabId: number | null,
  probeViaApp?: ProbeViaApp,
  opts?: { readonly retryFailed?: boolean },
): Promise<ManifestInfo> {
  const hit = liveEntry(manifestUrl);
  if (hit && !(opts?.retryFailed && hit.failed)) return hit.info;

  const existing = inFlight.get(manifestUrl);
  if (existing) return existing;

  // `finally` runs before the derived promise settles, so the entry is
  // always gone by the time any awaiter observes the result — a later
  // call either hits the now-populated cache or starts a fresh run.
  const run = resolveVariants(manifestUrl, tabId, probeViaApp).finally(() => {
    inFlight.delete(manifestUrl);
  });
  inFlight.set(manifestUrl, run);
  return run;
}

/**
 * What is already known about `manifestUrl`, or `undefined` when nothing
 * has been resolved (or the entry has expired). Pure cache read — never
 * fetches, never spawns a probe, never awaits.
 *
 * `undefined` ("not resolved yet") is deliberately distinct from an empty
 * `variants` ("resolved: not a master playlist"): the popup's fast path
 * renders the former as a plain row that may still regroup into quality
 * rows, and the latter as a plain row that is final.
 */
export function peekManifest(manifestUrl: string): ManifestInfo | undefined {
  return liveEntry(manifestUrl)?.info;
}

/**
 * True when `manifestUrl` holds a cached entry that is a real ANSWER, not
 * a recorded failure.
 *
 * The service worker uses this to decide whether a tab still has work to
 * do. Asking "is anything cached" instead treats a failed warm-up as
 * finished work and skips the retry the user is waiting on.
 */
export function isResolved(manifestUrl: string): boolean {
  const hit = liveEntry(manifestUrl);
  return hit != null && !hit.failed;
}

/** The unexpired cache entry for `manifestUrl`, or undefined. */
function liveEntry(manifestUrl: string): CacheEntry | undefined {
  const hit = cache.get(manifestUrl);
  return hit && hit.expiresAt > Date.now() ? hit : undefined;
}

async function resolveVariants(
  manifestUrl: string,
  tabId: number | null,
  probeViaApp?: ProbeViaApp,
): Promise<ManifestInfo> {
  const deadline = Date.now() + TOTAL_BUDGET_MS;

  const text = await fetchManifest(manifestUrl, tabId, deadline);
  if (text != null) {
    // An oversized body is treated as "not a master" rather than an error
    // — it's almost certainly a media playlist, and the popup already
    // renders "no variants" as a plain stream row either way.
    const isMaster = text.length <= MAX_MANIFEST_LENGTH_CHARS && isMasterPlaylist(text);
    let info: ManifestInfo;
    if (isMaster) {
      const variants = parseMasterPlaylist(text, manifestUrl);
      // One extra fetch, and only when there is something to attach the
      // answer to: a master with no parsable renditions gains nothing
      // from knowing how long it runs.
      const durationSecs =
        variants.length > 0 ? await probeDuration(variants, tabId, deadline) : null;
      info = {
        variants: variants.map((v) => withDuration(v, durationSecs)),
        durationSecs,
      };
    } else {
      // A media playlist. Its duration is free — the body in hand IS the
      // segment list, so no probe fetch is needed. This is the case the
      // popup used to render as a bare URL with no facts at all.
      info = { variants: [], durationSecs: parseMediaPlaylistDuration(text) };
    }
    cache.set(manifestUrl, { info, expiresAt: Date.now() + CACHE_TTL_MS, failed: false });
    log.debug(
      "hls resolved:",
      isMaster ? `master, ${info.variants.length} variant(s)` : "media playlist",
      `duration=${info.durationSecs ?? "unknown"}`,
      manifestUrl,
    );
    return info;
  }

  // Both fetch tiers failed (or the budget ran out before they could
  // finish) — the expensive last resort, only if the caller supplied one
  // and there's budget left for it.
  const probed = probeViaApp
    ? await fetchAppProbe(probeViaApp, manifestUrl, deadline - Date.now())
    : null;
  if (probed != null) {
    // yt-dlp reports the duration on every format it returns, so no
    // probe fetch is needed on this path either. Read it off the first
    // format that states one.
    const info: ManifestInfo = {
      variants: probed,
      durationSecs: probed.find((v) => v.durationSecs != null)?.durationSecs ?? null,
    };
    cache.set(manifestUrl, { info, expiresAt: Date.now() + CACHE_TTL_MS, failed: false });
    return info;
  }

  // Negative-cache the failure — see NEGATIVE_CACHE_TTL_MS. The next popup
  // open past that window retries (the page may not have been ready, a
  // Referer-gated request transiently 403'd, or the bridge was briefly
  // down for the probe).
  log.debug("hls resolve FAILED — every tier came up empty:", manifestUrl);
  cache.set(manifestUrl, {
    info: UNKNOWN,
    expiresAt: Date.now() + NEGATIVE_CACHE_TTL_MS,
    failed: true,
  });
  return UNKNOWN;
}

/**
 * Fetch the manifest body, preferring the *page* context. A media CDN
 * behind Referer/hotlink protection (the common case) rejects a bare
 * service-worker fetch with 403 — the SW can't set a `Referer` (a
 * forbidden fetch header) and carries no page origin. Running the fetch
 * inside the tab via `chrome.scripting.executeScript` inherits the page's
 * Referer, cookies, and origin (so CORS matches what hls.js already
 * negotiated), which is what actually returns the master playlist. Falls
 * back to a direct SW fetch when there's no tab or the in-page fetch fails
 * (same-origin manifests, or a page that navigated away).
 *
 * The two tiers run *concurrently* and the first body wins. They used to
 * run in sequence, which meant a hanging in-page tier cost its full leash
 * before the SW tier even started — and the two together could eat the
 * whole `deadline`, leaving the app probe nothing. Racing costs one extra
 * (usually 403) request per cache miss and preserves the tier preference
 * for free: on the hotlink-protected CDNs this ordering exists for, the SW
 * tier resolves `null`, so it can never beat a real in-page body.
 *
 * Both tiers are capped at `FETCH_TIMEOUT_MS`, clamped to whatever's left
 * of `deadline` — started with little budget left they get a short leash
 * rather than the full allowance, and are skipped outright once the
 * deadline has already passed.
 */
async function fetchManifest(
  url: string,
  tabId: number | null,
  deadline: number,
): Promise<string | null> {
  const budget = Math.min(FETCH_TIMEOUT_MS, deadline - Date.now());
  if (budget <= 0) return null;

  const tiers: Promise<string | null>[] = [fetchInServiceWorker(url, budget)];
  if (tabId != null) tiers.unshift(fetchInPage(tabId, url, budget));
  return raceForBody(tiers);
}

/**
 * Resolve to the first tier that produces a body, or `null` once every
 * tier has come up empty. Neither tier rejects (both swallow their own
 * errors), but a rejection is folded into `null` rather than escaping to
 * the caller.
 */
function raceForBody(tiers: readonly Promise<string | null>[]): Promise<string | null> {
  if (tiers.length === 0) return Promise.resolve(null);
  return new Promise((resolve) => {
    let pending = tiers.length;
    const settle = (body: string | null): void => {
      if (body != null) {
        resolve(body);
        return;
      }
      pending -= 1;
      if (pending === 0) resolve(null);
    };
    for (const tier of tiers) {
      tier.then(settle, () => settle(null));
    }
  });
}

async function fetchInPage(tabId: number, url: string, budgetMs: number): Promise<string | null> {
  try {
    // The injected function is serialised via `Function.prototype.toString`
    // and re-parsed in the page's isolated world — it cannot close over
    // any variable from this module's scope. Everything it needs must
    // travel through `args`.
    const execution = chrome.scripting.executeScript({
      target: { tabId },
      // Returns a reason string on failure rather than a bare null. The
      // injected world is the one place we cannot observe directly, and
      // "the in-page tier returned nothing" was impossible to tell apart
      // from "the CDN said 403" without it.
      func: async (
        manifestUrl: string,
        timeoutMs: number,
      ): Promise<{ body: string | null; reason: string }> => {
        const attempt = async (
          credentials: RequestCredentials,
        ): Promise<{ body: string | null; reason: string }> => {
          const res = await fetch(manifestUrl, {
            credentials,
            signal: AbortSignal.timeout(timeoutMs),
          });
          if (!res.ok) return { body: null, reason: `http ${res.status} (${credentials})` };
          return { body: await res.text(), reason: `ok (${credentials})` };
        };
        // Cookies first: a session-gated CDN needs them, and it answers
        // with a specific `Access-Control-Allow-Origin`, so CORS allows
        // the credentialed request.
        try {
          return await attempt("include");
        } catch (includeErr) {
          // The other, more common shape: the CDN answers
          // `Access-Control-Allow-Origin: *` and no
          // `Access-Control-Allow-Credentials`. CORS rejects a wildcard
          // origin for a credentialed request, so the fetch dies inside
          // the browser before a request is ever sent — the failure looks
          // like a bare `TypeError: Failed to fetch`, not an HTTP status.
          //
          // A wildcard origin is exactly what lets hls.js read the same
          // manifest, because a media element fetches it without
          // credentials. Retrying without them is what makes this tier
          // work on the CDNs it was written for. Nothing is lost: a
          // manifest that truly needs cookies already succeeded above.
          try {
            return await attempt("omit");
          } catch (omitErr) {
            return {
              body: null,
              reason: `include: ${includeErr}; omit: ${omitErr}`,
            };
          }
        }
      },
      args: [url, budgetMs],
    });
    // `executeScript`'s own promise is a second failure mode beyond the
    // injected fetch's abort signal — it can hang on a suspended/
    // backgrounded tab regardless of what the injected code does.
    const [injection] = await withTimeout(execution, budgetMs + EXEC_IPC_OVERHEAD_MS);
    // Chrome awaits the injected async function and lands its resolved
    // value in `result`; a thrown/unhandled rejection instead populates
    // `injection.error` and leaves `result` undefined. Our injected
    // function never throws (it catches internally), but guard the shape
    // anyway rather than trust it.
    const out = injection?.result as { body?: unknown; reason?: unknown } | undefined;
    const body = typeof out?.body === "string" ? out.body : null;
    if (body == null) {
      log.debug("hls in-page fetch failed:", String(out?.reason ?? "no result"), url);
    }
    return body;
  } catch (err) {
    // Restricted page (chrome://), no host access, tab gone, or the
    // outer timeout guard above fired.
    log.debug("hls in-page tier unavailable:", err, url);
    return null;
  }
}

/**
 * Run the app probe with its own slice of the overall budget — unlike the
 * fetch tiers, the caller supplies this function, so it carries no timeout
 * of its own to lean on. Treats a timeout exactly like the prober itself
 * returning `null`: "couldn't answer in time" either way.
 */
async function fetchAppProbe(
  probeViaApp: ProbeViaApp,
  manifestUrl: string,
  budgetMs: number,
): Promise<readonly MediaVariant[] | null> {
  if (budgetMs <= 0) return null;
  try {
    return await withTimeout(probeViaApp(manifestUrl), budgetMs);
  } catch {
    return null;
  }
}

/** Rejects after `ms` if `promise` hasn't settled — a backstop for
 * promises (like `executeScript`) that can hang independently of any
 * timeout their own internals carry. */
function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("timed out")), ms);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (err: unknown) => {
        clearTimeout(timer);
        reject(err instanceof Error ? err : new Error(String(err)));
      },
    );
  });
}

async function fetchInServiceWorker(url: string, budgetMs: number): Promise<string | null> {
  try {
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), budgetMs);
    try {
      const res = await fetch(url, { credentials: "include", signal: ctrl.signal });
      if (!res.ok) {
        log.debug("hls SW fetch failed: http", res.status, url);
        return null;
      }
      return await res.text();
    } finally {
      clearTimeout(timer);
    }
  } catch (err) {
    log.debug("hls SW fetch failed:", err, url);
    return null;
  }
}

/** Human-facing pick shown on a quality row. Exported so the service
 * worker can label variants that came from the app probe: the wire
 * `MediaFormat` deliberately carries no `label`, so both discovery paths
 * format through this one function rather than drifting apart. */
export function labelFor(
  height: number | null,
  bandwidth: number | null,
  resolution: string | null,
): string {
  if (height && height > 0) return `${height}p`;
  if (resolution) return resolution;
  if (bandwidth && bandwidth > 0) return `${Math.round(bandwidth / 1000)} kbps`;
  return "auto";
}
