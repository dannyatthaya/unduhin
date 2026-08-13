// Sniffed streams → the shape the popup renders.
//
// Extracted from the service worker so both paths that produce a media
// list go through exactly one implementation: the snapshot reply (built
// synchronously from whatever variants are already cached) and the
// background broadcast that follows once resolution lands. When these two
// lived inline they could only be kept in step by hand, and a drift
// between them shows up as rows flickering in and out on the re-render.
//
// A separate module rather than an export from `service-worker.ts` —
// importing that file installs every background listener as a side effect,
// so it can't be pulled into a test.

import type { MediaStream, MediaVariant, PopupMediaStream } from "../shared/types.js";

/**
 * Build the popup's media list from the streams the sniffer caught on a
 * tab.
 *
 * `variantsFor` supplies each HLS manifest's already-known qualities —
 * `peekVariants` for the synchronous fast path, the resolved results for
 * the background path. An empty array means "no qualities to show",
 * whether that's because the manifest isn't a master playlist or simply
 * because nothing has resolved yet; either way the stream renders as a
 * single plain row.
 *
 * `tabId` is the fallback for a sniffed stream that carries none of its
 * own.
 */
export function assembleStreams(
  sniffed: readonly MediaStream[],
  tabId: number | null,
  variantsFor: (manifestUrl: string) => readonly MediaVariant[],
): PopupMediaStream[] {
  const parsed = sniffed.map((s) => ({
    s,
    variants: s.kind === "hls" ? variantsFor(s.manifestUrl) : [],
  }));

  // A master's renditions are themselves media playlists the sniffer often
  // also caught (e.g. the one hls.js auto-selected). Collect every master's
  // variant URLs so we can drop those twin rows — the master's quality rows
  // already cover them. A master whose variants aren't resolved yet
  // contributes nothing here, so its twin stays visible until the master
  // can actually replace it.
  const variantUrls = new Set<string>();
  for (const p of parsed) {
    for (const v of p.variants) variantUrls.add(v.url);
  }

  return parsed
    .filter((p) => !(p.variants.length === 0 && variantUrls.has(p.s.manifestUrl)))
    .map((p) => ({
      kind: p.s.kind,
      manifestUrl: p.s.manifestUrl,
      pageUrl: p.s.pageUrl,
      tabId: Number(p.s.tabId ?? tabId),
      suggestedFilename: p.s.suggestedFilename,
      ...(p.variants.length > 0 ? { variants: p.variants } : {}),
    }));
}

/**
 * Whether two assembled lists would render identically. Used to suppress
 * the background broadcast when resolution didn't actually change anything
 * the popup is showing — a no-op re-render still tears down and rebuilds
 * every row, which is visible as a flicker.
 */
export function sameStreams(
  a: readonly PopupMediaStream[],
  b: readonly PopupMediaStream[],
): boolean {
  if (a.length !== b.length) return false;
  return a.every((left, i) => {
    const right = b[i]!;
    if (left.manifestUrl !== right.manifestUrl) return false;
    const lv = left.variants ?? [];
    const rv = right.variants ?? [];
    return lv.length === rv.length && lv.every((v, j) => v.url === rv[j]!.url);
  });
}
