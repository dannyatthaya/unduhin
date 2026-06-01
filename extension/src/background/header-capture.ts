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
]);

interface CacheEntry {
  readonly headers: readonly chrome.webRequest.HttpHeader[];
  readonly expiresAt: number;
}

export interface HeaderCache {
  /** Returns a copy of cached headers for `url` or `null` if absent/expired. */
  getHeadersFor(url: string): chrome.webRequest.HttpHeader[] | null;
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

  const listener = (
    details: chrome.webRequest.WebRequestHeadersDetails,
  ): void => {
    if (!details.requestHeaders) return;
    if (details.tabId < 0) return; // service worker / extension internal — skip.

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
    { urls: ["<all_urls>"] },
    ["requestHeaders", "extraHeaders"],
  );

  log.info("header-capture installed");

  return {
    getHeadersFor(url) {
      const entry = cache.get(url);
      if (!entry) return null;
      if (entry.expiresAt < Date.now()) {
        cache.delete(url);
        return null;
      }
      // Touch on read so the entry doesn't get evicted while it's still hot.
      cache.delete(url);
      cache.set(url, entry);
      // Return a shallow copy so callers can't mutate the cached slot.
      return entry.headers.map((h) => ({ ...h }));
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
    },
  };
}
