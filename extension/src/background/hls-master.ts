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
const FETCH_TIMEOUT_MS = 2_500;

interface CacheEntry {
  readonly variants: readonly MediaVariant[];
  readonly expiresAt: number;
}

const cache = new Map<string, CacheEntry>();

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
 * Returns an empty array for media playlists, network/timeout failures, or
 * non-OK responses — callers treat "no variants" as "plain stream". Cached
 * for {@link CACHE_TTL_MS} keyed by URL.
 */
export async function loadVariants(manifestUrl: string): Promise<readonly MediaVariant[]> {
  const now = Date.now();
  const hit = cache.get(manifestUrl);
  if (hit && hit.expiresAt > now) return hit.variants;

  let variants: readonly MediaVariant[] = [];
  try {
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), FETCH_TIMEOUT_MS);
    try {
      const res = await fetch(manifestUrl, { credentials: "include", signal: ctrl.signal });
      if (res.ok) {
        const text = await res.text();
        if (isMasterPlaylist(text)) variants = parseMasterPlaylist(text, manifestUrl);
      }
    } finally {
      clearTimeout(timer);
    }
  } catch {
    // Network error, timeout, or CORS — treat as a plain (non-master) stream.
  }

  cache.set(manifestUrl, { variants, expiresAt: now + CACHE_TTL_MS });
  return variants;
}

function labelFor(
  height: number | null,
  bandwidth: number | null,
  resolution: string | null,
): string {
  if (height && height > 0) return `${height}p`;
  if (resolution) return resolution;
  if (bandwidth && bandwidth > 0) return `${Math.round(bandwidth / 1000)} kbps`;
  return "auto";
}
