// Request-header capture.
//
// The download interceptor needs the same headers Chrome
// sent on the *original* request so the engine can replay range requests
// that look identical to the browser's. We listen on
// `webRequest.onBeforeSendHeaders`, snapshot the headers, and cache them
// for 90s keyed by URL.
//
// Persistence is intentionally session-only. The service worker may sleep
// at any time and lose the cache; that's fine — the cache is a hot-path
// optimisation, not authoritative state. Cookies are filtered out before
// storing (we never want them to bleed across calls; the cookie forwarder
// pulls them fresh per request).

import { CAPTURE_RESOURCE_TYPES } from "./request-types.js";
import { log } from "../shared/log.js";

const MAX_ENTRIES = 200;
const TTL_MS = 90_000;

/**
 * Headers we deliberately do NOT persist in the cache:
 *   - `Cookie`: replayed via `cookie-forwarder.ts` per URL on demand.
 *     Keeping it here would leak session state across unrelated URLs in
 *     the same cache.
 *   - `Host` / `Content-Length` / `Connection`: per-request transport
 *     mechanics. The native side fills these in itself.
 *   - `Proxy-*`: never appropriate to replay.
 *
 * The engine's `HEADER_DROP_LIST` (in `crates/engine/src/http.rs`) is the
 * defensive backstop — anything that sneaks past us here is dropped on
 * the Rust side before reqwest sees it.
 */
const STRIP = new Set([
  "cookie",
  "host",
  "content-length",
  "connection",
  "proxy-authorization",
  "proxy-authenticate",
  // Conditional / range headers describe the BROWSER's cache and its own
  // partial copy, not ours. Replayed by the engine they turn a download into
  // a `304 Not Modified` (no body) or pin a range to a validator we never saw.
  "if-none-match",
  "if-modified-since",
  "if-match",
  "if-unmodified-since",
  "if-range",
  "range",
]);

/**
 * How long we remember what KIND of request a tab made for a URL. Much
 * longer than the header TTL on purpose: "Save image as…" is routinely used
 * minutes after the page loaded the image, and the answer is only useful if
 * it is still here by then.
 */
const TYPE_TTL_MS = 30 * 60_000;
const MAX_TYPE_ENTRIES = 4000;

interface CacheEntry {
  readonly headers: readonly chrome.webRequest.HttpHeader[];
  readonly expiresAt: number;
}

interface TypeEntry {
  readonly type: chrome.webRequest.ResourceType;
  readonly expiresAt: number;
}

export interface HeaderCache {
  /** Returns a copy of cached headers for `url` or `null` if absent/expired. */
  getHeadersFor(url: string): chrome.webRequest.HttpHeader[] | null;
  /**
   * How a tab most recently requested `url` — `"image"` / `"media"` when the
   * page loaded it as a picture or a player source, `"main_frame"` when the
   * user navigated to it, and so on. `null` when no tab requested it within
   * the last {@link TYPE_TTL_MS}. The interceptor uses this to tell a link
   * click from the browser's own "Save image as…".
   */
  getResourceTypeFor(url: string): chrome.webRequest.ResourceType | null;
  /** Number of live (non-expired) entries — exported for tests / debugging. */
  size(): number;
  /** Tear down listeners. Currently only used for tests. */
  dispose(): void;
}

export function installHeaderCapture(): HeaderCache {
  // `Map` preserves insertion order, which gives us cheap LRU semantics:
  // on each write we delete-then-set so the most recently touched URL
  // ends up last. On overflow, we evict the head (least recently used).
  const cache = new Map<string, CacheEntry>();
  // Same LRU discipline as `cache`, but tracking only the resource type and
  // kept far longer — see `TYPE_TTL_MS`.
  const types = new Map<string, TypeEntry>();

  const listener = (
    details: chrome.webRequest.WebRequestHeadersDetails,
  ): void => {
    if (details.tabId < 0) return; // service worker / extension internal — skip.

    types.delete(details.url);
    types.set(details.url, { type: details.type, expiresAt: Date.now() + TYPE_TTL_MS });
    while (types.size > MAX_TYPE_ENTRIES) {
      const oldest = types.keys().next().value;
      if (oldest === undefined) break;
      types.delete(oldest);
    }

    if (!details.requestHeaders) return;

    const filtered: chrome.webRequest.HttpHeader[] = [];
    for (const h of details.requestHeaders) {
      if (!h.name) continue;
      if (STRIP.has(h.name.toLowerCase())) continue;
      filtered.push({ name: h.name, value: h.value, binaryValue: h.binaryValue });
    }
    if (filtered.length === 0) return;

    cache.delete(details.url);
    cache.set(details.url, {
      headers: filtered,
      expiresAt: Date.now() + TTL_MS,
    });

    while (cache.size > MAX_ENTRIES) {
      const oldest = cache.keys().next().value;
      if (oldest === undefined) break;
      cache.delete(oldest);
    }
  };

  // `extraHeaders` is required to see Cookie / Referer / Authorization on
  // MV3 — without it Chrome hides them from the listener.
  chrome.webRequest.onBeforeSendHeaders.addListener(
    listener,
    { urls: ["<all_urls>"], types: CAPTURE_RESOURCE_TYPES },
    ["requestHeaders", "extraHeaders"],
  );

  log.info("header-capture installed");

  return {
    getHeadersFor(url) {
      // Fragments never reach the wire, so the cache is keyed by the
      // fragment-less `details.url`. Strip defensively in case a caller
      // passes a `#fragment` URL (e.g. a one-click host that encodes the
      // filename there) so the lookup still hits.
      const hash = url.indexOf("#");
      const key = hash >= 0 ? url.slice(0, hash) : url;
      const entry = cache.get(key);
      if (!entry) return null;
      if (entry.expiresAt < Date.now()) {
        cache.delete(key);
        return null;
      }
      // Touch on read so the entry doesn't get evicted while it's still hot.
      cache.delete(key);
      cache.set(key, entry);
      // Return a shallow copy so callers can't mutate the cached slot.
      return entry.headers.map((h) => ({ ...h }));
    },
    getResourceTypeFor(url) {
      const hash = url.indexOf("#");
      const key = hash >= 0 ? url.slice(0, hash) : url;
      const entry = types.get(key);
      if (!entry) return null;
      if (entry.expiresAt < Date.now()) {
        types.delete(key);
        return null;
      }
      return entry.type;
    },
    size() {
      let n = 0;
      const now = Date.now();
      for (const entry of cache.values()) {
        if (entry.expiresAt >= now) n++;
      }
      return n;
    },
    dispose() {
      chrome.webRequest.onBeforeSendHeaders.removeListener(listener);
      cache.clear();
      types.clear();
    },
  };
}
