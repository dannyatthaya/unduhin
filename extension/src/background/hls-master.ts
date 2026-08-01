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

import type { MediaVariant } from "../shared/types.js";

const STREAM_INF = "#EXT-X-STREAM-INF:";
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
// A failed fetch (both in-page and SW) is cached briefly too — otherwise
// every popup open re-injects a script into the tab for a manifest that
// just 403'd. Much shorter than CACHE_TTL_MS so a transient failure (page
// not ready yet, blip in the CDN) still retries soon. A `null` from the app
// probe (couldn't answer) uses the same short TTL — only an actual answer
// (including an empty array — "answered, no variants") earns the long one.
const NEGATIVE_CACHE_TTL_MS = 5_000;
// A media playlist can be megabytes of segment lines; fetching it is
// already sunk once we're here, but parsing it for nothing is not. Master
// playlists are small (a handful of lines per rendition) even with many
// qualities, so anything past this is almost certainly a media playlist.
const MAX_MANIFEST_LENGTH_CHARS = 256_000;
// Wall-clock ceiling for one `loadVariants` call, covering ALL tiers
// combined. `buildSnapshot` in the service worker awaits every sniffed
// manifest's `loadVariants` via `Promise.all` before the popup renders
// anything, so an unbounded chain directly stalls the UI. Set equal to the
// worst case the two cheap fetch tiers could already take on their own
// (in-page's FETCH_TIMEOUT_MS + EXEC_IPC_OVERHEAD_MS, plus SW's
// FETCH_TIMEOUT_MS) — the wait users already tolerate today, before the app
// probe existed. If both fetch tiers time out (the pathological case —
// everything's hanging), the budget is exhausted and the probe is skipped
// entirely rather than adding a yt-dlp subprocess spawn on top of an
// already-maxed-out wait. In the far more common failure mode — a fast
// 403 rather than a hang — the fetch tiers return in well under their
// caps, leaving real budget for the probe to actually help.
const TOTAL_BUDGET_MS = FETCH_TIMEOUT_MS + EXEC_IPC_OVERHEAD_MS + FETCH_TIMEOUT_MS;

interface CacheEntry {
  readonly variants: readonly MediaVariant[];
  readonly expiresAt: number;
}

const cache = new Map<string, CacheEntry>();

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
    const bw = /BANDWIDTH=(\d+)/i.exec(attrs);
    const height = res ? Number.parseInt(res[2]!, 10) : null;
    const resolution = res ? `${res[1]}x${res[2]}` : null;
    const bandwidth = bw ? Number.parseInt(bw[1]!, 10) : null;

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
    });
  }

  const seen = new Set<string>();
  return variants
    .filter((v) => (seen.has(v.url) ? false : (seen.add(v.url), true)))
    .sort((a, b) => (b.height ?? 0) - (a.height ?? 0) || (b.bandwidth ?? 0) - (a.bandwidth ?? 0));
}

/**
 * Fetch `manifestUrl` and, if it's a master playlist, return its qualities.
 * Resolution order — cheapest first — is: in-page fetch, then a direct
 * service-worker fetch, then (if `probeViaApp` is supplied) the native-app
 * probe as a last resort. Each tier only runs if the ones before it
 * couldn't answer, and the whole call is bounded by {@link TOTAL_BUDGET_MS}
 * — see the tier budgeting in {@link fetchManifest} and {@link fetchAppProbe}.
 * `probeViaApp` omitted behaves exactly as the two-tier fetch chain did
 * before it existed.
 *
 * Returns an empty array for media playlists, an app probe that answered
 * "no variants", or (once every available tier has failed/timed out) as
 * the fallback — callers treat "no variants" as "plain stream" either way.
 * Cached for {@link CACHE_TTL_MS} on an actual answer, {@link
 * NEGATIVE_CACHE_TTL_MS} when nothing could answer at all.
 */
export async function loadVariants(
  manifestUrl: string,
  tabId: number | null,
  probeViaApp?: ProbeViaApp,
): Promise<readonly MediaVariant[]> {
  const now = Date.now();
  const hit = cache.get(manifestUrl);
  if (hit && hit.expiresAt > now) return hit.variants;

  const deadline = now + TOTAL_BUDGET_MS;

  const text = await fetchManifest(manifestUrl, tabId, deadline);
  if (text != null) {
    // An oversized body is treated as "not a master" rather than an error
    // — it's almost certainly a media playlist, and the popup already
    // renders "no variants" as a plain stream row either way.
    const variants =
      text.length <= MAX_MANIFEST_LENGTH_CHARS && isMasterPlaylist(text)
        ? parseMasterPlaylist(text, manifestUrl)
        : [];
    cache.set(manifestUrl, { variants, expiresAt: Date.now() + CACHE_TTL_MS });
    return variants;
  }

  // Both fetch tiers failed (or the budget ran out before they could
  // finish) — the expensive last resort, only if the caller supplied one
  // and there's budget left for it.
  const probed = probeViaApp
    ? await fetchAppProbe(probeViaApp, manifestUrl, deadline - Date.now())
    : null;
  if (probed != null) {
    cache.set(manifestUrl, { variants: probed, expiresAt: Date.now() + CACHE_TTL_MS });
    return probed;
  }

  // Negative-cache the failure — see NEGATIVE_CACHE_TTL_MS. The next popup
  // open past that window retries (the page may not have been ready, a
  // Referer-gated request transiently 403'd, or the bridge was briefly
  // down for the probe).
  cache.set(manifestUrl, { variants: [], expiresAt: Date.now() + NEGATIVE_CACHE_TTL_MS });
  return [];
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
 * Each tier is capped at `FETCH_TIMEOUT_MS`, further clamped to whatever's
 * left of `deadline` — a tier started with little budget left gets a short
 * leash rather than its usual full allowance, and a tier gets skipped
 * outright once the deadline has already passed.
 */
async function fetchManifest(
  url: string,
  tabId: number | null,
  deadline: number,
): Promise<string | null> {
  if (tabId != null) {
    const budget = Math.min(FETCH_TIMEOUT_MS, deadline - Date.now());
    if (budget > 0) {
      const inPage = await fetchInPage(tabId, url, budget);
      if (inPage != null) return inPage;
    }
  }

  const budget = Math.min(FETCH_TIMEOUT_MS, deadline - Date.now());
  if (budget <= 0) return null;
  return fetchInServiceWorker(url, budget);
}

async function fetchInPage(tabId: number, url: string, budgetMs: number): Promise<string | null> {
  try {
    // The injected function is serialised via `Function.prototype.toString`
    // and re-parsed in the page's isolated world — it cannot close over
    // any variable from this module's scope. Everything it needs must
    // travel through `args`.
    const execution = chrome.scripting.executeScript({
      target: { tabId },
      func: async (manifestUrl: string, timeoutMs: number): Promise<string | null> => {
        try {
          const res = await fetch(manifestUrl, {
            credentials: "include",
            signal: AbortSignal.timeout(timeoutMs),
          });
          return res.ok ? await res.text() : null;
        } catch {
          return null;
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
    const text = injection?.result;
    return typeof text === "string" ? text : null;
  } catch {
    // Restricted page (chrome://), no host access, tab gone, or the
    // outer timeout guard above fired.
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
      return res.ok ? await res.text() : null;
    } finally {
      clearTimeout(timer);
    }
  } catch {
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
