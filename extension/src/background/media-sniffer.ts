// Media stream sniffer.
//
// Detect HLS (`.m3u8` URL or `application/vnd.apple.mpegurl` content-type)
// and DASH (`.mpd` URL or `application/dash+xml`) as the browser sees
// them. Stash per `tabId`; clear on top-frame navigation or tab close;
// surface the count on the toolbar badge so the user knows there's
// something to grab.
//
// The map is session-only — the SW can be paused at any time and lose
// its memory; that's fine, the badge will repopulate as soon as
// playback fetches its next manifest.

import { log } from "../shared/log.js";
import type { MediaKind, MediaStream, RequestHeader } from "../shared/types.js";
import { buildCookieHeader } from "./cookie-forwarder.js";
import type { HeaderCache } from "./header-capture.js";
import type { SettingsReader } from "../shared/settings.js";

const HLS_CONTENT_TYPE_FRAGMENTS = [
  "application/vnd.apple.mpegurl",
  "application/x-mpegurl",
  "audio/mpegurl",
  "vnd.apple.mpegurl",
];
const DASH_CONTENT_TYPE_FRAGMENTS = ["application/dash+xml", "dash+xml"];

const BADGE_COLOR = "#2563eb"; // matches the brand blue used in `frontend/src/style.css`.

export interface MediaSnifferDeps {
  readonly headerCache: HeaderCache;
  readonly settings: SettingsReader;
  /**
   * Called once per newly-detected stream — never for a re-fetch of a
   * manifest already on the tab's list. Lets the service worker start
   * resolving a master playlist's qualities while the page is still
   * loading, so the popup has them cached before it's ever opened.
   *
   * Synchronous and must not throw: this runs inside a `webRequest`
   * listener, so anything expensive belongs behind a fire-and-forget
   * promise rather than in the callback body.
   */
  readonly onStreamDetected?: (stream: MediaStream) => void;
}

export interface MediaSniffer {
  getStreamsForTab(tabId: number): readonly MediaStream[];
  /**
   * Fill in cookies + UA + cached headers for an already-detected stream
   * just before it's handed to the bridge. Lets us defer the (async)
   * cookie call until the user clicks "Download" in the popup, rather
   * than running it on every sniffed manifest.
   */
  enrich(stream: MediaStream): Promise<MediaStream>;
  dispose(): void;
}

export function installMediaSniffer(deps: MediaSnifferDeps): MediaSniffer {
  const byTab = new Map<number, MediaStream[]>();

  const onResponse = (details: chrome.webRequest.WebResponseCacheDetails): void => {
    if (details.tabId < 0) return;
    const settings = deps.settings.current();
    const kind = classify(details, settings);
    if (!kind) return;

    const list = byTab.get(details.tabId) ?? [];
    if (list.some((s) => s.manifestUrl === details.url)) {
      return; // dedupe — re-fetches of the same manifest are common.
    }
    const stream: MediaStream = {
      kind,
      manifestUrl: details.url,
      pageUrl: details.initiator ?? null,
      tabId: details.tabId,
      suggestedFilename: deriveFilename(details.url, kind),
      referrer: null,
      userAgent: null,
      cookieHeader: null,
      requestHeaders: [],
    };
    list.push(stream);
    byTab.set(details.tabId, list);
    setBadge(details.tabId, list.length);
    log.debug("media-sniffer:", kind, details.url);
    // Last, and guarded: a throwing consumer must not cost us the badge
    // update or the stream we just recorded.
    try {
      deps.onStreamDetected?.(stream);
    } catch (err) {
      log.warn("onStreamDetected threw", err);
    }
  };

  const onTabRemoved = (tabId: number): void => {
    if (byTab.delete(tabId)) {
      log.debug("media-sniffer: cleared closed tab", tabId);
    }
  };

  const onCommitted = (
    details: chrome.webNavigation.WebNavigationTransitionCallbackDetails,
  ): void => {
    if (details.frameId !== 0) return; // only top-frame navigation resets the streams.
    // A reload re-fetches the same page's manifests, so the streams are
    // still valid — clearing here just makes the badge flicker to 0 until
    // playback re-detects them. Only clear on a genuine navigation away.
    if (details.transitionType === "reload") return;
    if (byTab.delete(details.tabId)) {
      setBadge(details.tabId, 0);
      log.debug("media-sniffer: cleared on top-frame navigation", details.tabId);
    }
  };

  chrome.webRequest.onResponseStarted.addListener(
    onResponse,
    { urls: ["<all_urls>"] },
    ["responseHeaders"],
  );
  chrome.tabs.onRemoved.addListener(onTabRemoved);
  chrome.webNavigation.onCommitted.addListener(onCommitted);

  log.info("media-sniffer installed");

  return {
    getStreamsForTab(tabId) {
      return byTab.get(tabId) ?? [];
    },
    async enrich(stream) {
      const cookieHeader = await buildCookieHeader(stream.manifestUrl).catch(() => "");
      const cached = deps.headerCache.getHeadersFor(stream.manifestUrl) ?? [];
      const requestHeaders: RequestHeader[] = cached
        .filter((h) => typeof h.name === "string" && h.name.length > 0)
        .map((h) => ({
          name: h.name,
          value: typeof h.value === "string" ? h.value : "",
        }));
      // Forward the Referer. The native side treats `referrer` as a
      // dedicated field and drops any `Referer` left in `requestHeaders`
      // (see `is_prepended_header` in `wire.rs`), so a Referer that only
      // lived in the captured headers would otherwise be lost for media
      // captures. Pull it out of the observed request headers, falling
      // back to the page origin (`pageUrl`/initiator) when the browser
      // sent none.
      const capturedReferer = requestHeaders.find(
        (h) => h.name.toLowerCase() === "referer",
      )?.value;
      const referrer =
        capturedReferer && capturedReferer.length > 0 ? capturedReferer : stream.pageUrl;
      return {
        ...stream,
        referrer,
        cookieHeader: cookieHeader.length > 0 ? cookieHeader : null,
        userAgent:
          typeof navigator !== "undefined" && typeof navigator.userAgent === "string"
            ? navigator.userAgent
            : null,
        requestHeaders,
      };
    },
    dispose() {
      chrome.webRequest.onResponseStarted.removeListener(onResponse);
      chrome.tabs.onRemoved.removeListener(onTabRemoved);
      chrome.webNavigation.onCommitted.removeListener(onCommitted);
      byTab.clear();
    },
  };
}

function classify(
  details: chrome.webRequest.WebResponseCacheDetails,
  settings: { detectHls: boolean; detectDash: boolean },
): MediaKind | null {
  const lowerUrl = details.url.toLowerCase();
  const contentType =
    findHeader(details.responseHeaders, "content-type")?.toLowerCase() ?? "";

  if (settings.detectHls) {
    if (lowerUrl.includes(".m3u8")) return "hls";
    if (HLS_CONTENT_TYPE_FRAGMENTS.some((f) => contentType.includes(f))) return "hls";
  }
  if (settings.detectDash) {
    if (lowerUrl.includes(".mpd")) return "dash";
    if (DASH_CONTENT_TYPE_FRAGMENTS.some((f) => contentType.includes(f))) return "dash";
  }
  return null;
}

function findHeader(
  headers: chrome.webRequest.HttpHeader[] | undefined,
  name: string,
): string | undefined {
  if (!headers) return undefined;
  const target = name.toLowerCase();
  for (const h of headers) {
    if (h.name && h.name.toLowerCase() === target) {
      return typeof h.value === "string" ? h.value : undefined;
    }
  }
  return undefined;
}

/**
 * Manifest basenames that identify nothing. Nearly every adaptive stream
 * on the internet names its media playlists one of these, so two
 * renditions of one video arrive with the same name.
 */
const GENERIC_BASENAMES = new Set([
  "video",
  "index",
  "playlist",
  "master",
  "media",
  "stream",
  "chunklist",
  "manifest",
  "audio",
]);

/**
 * A path segment that reads as a quality label — `720p`, `1080P`,
 * `1280x720`.
 *
 * Deliberately narrow. The folder above a manifest is only worth
 * borrowing a name from when it actually describes the rendition; the
 * common alternative is an opaque id (`/db3324d5-6caa-.../playlist.m3u8`),
 * and a UUID on a row is worse than the generic word it replaced.
 */
const QUALITY_SEGMENT = /^(\d{2,4}p|\d{2,5}x\d{2,5})$/i;

export function deriveFilename(url: string, kind: MediaKind): string | null {
  try {
    const u = new URL(url);
    const segments = u.pathname.split("/").filter(Boolean);
    const tail = segments.pop();
    if (!tail) return null;
    const decoded = decodeURIComponent(tail);
    // Strip the manifest extension so yt-dlp / the engine pick something
    // sensible. Keep the basename so the user recognises it.
    const base = decoded.replace(/\.(m3u8|mpd)(\?.*)?$/i, "");
    if (base.length === 0) return kind;
    // `…/720p/video.m3u8` and `…/480p/video.m3u8` both reduce to "video",
    // so a master whose renditions could not be resolved renders as two
    // identical rows. Take the name from the folder when the folder is a
    // quality label, which is the case that produces the collision.
    if (GENERIC_BASENAMES.has(base.toLowerCase())) {
      const parent = segments.pop();
      if (parent) {
        const decodedParent = decodeURIComponent(parent);
        if (QUALITY_SEGMENT.test(decodedParent)) return decodedParent;
      }
    }
    return base;
  } catch {
    return null;
  }
}

function setBadge(tabId: number, count: number): void {
  const text = count > 0 ? String(count) : "";
  try {
    chrome.action.setBadgeText({ tabId, text });
    if (count > 0) {
      chrome.action.setBadgeBackgroundColor({ tabId, color: BADGE_COLOR });
    }
  } catch (err) {
    // `chrome.action` requires the `action` block in manifest. If the
    // manifest lost that field the call throws synchronously — log and
    // continue (the sniffer is still useful even without a badge).
    log.warn("setBadgeText failed", err);
  }
}
