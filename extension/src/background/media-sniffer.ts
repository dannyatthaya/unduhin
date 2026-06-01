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
      return {
        ...stream,
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

function deriveFilename(url: string, kind: MediaKind): string | null {
  try {
    const u = new URL(url);
    const tail = u.pathname.split("/").filter(Boolean).pop();
    if (!tail) return null;
    const decoded = decodeURIComponent(tail);
    // Strip the manifest extension so yt-dlp / the engine pick something
    // sensible. Keep the basename so the user recognises it.
    const base = decoded.replace(/\.(m3u8|mpd)(\?.*)?$/i, "");
    return base.length > 0 ? base : kind;
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
