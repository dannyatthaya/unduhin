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

import type { ManifestInfo } from "./hls-master.js";
import type { MediaStream, PopupMediaStream } from "../shared/types.js";

/** What a stream with no resolved manifest info renders as. */
const NOTHING_KNOWN: ManifestInfo = { variants: [], durationSecs: null };

/**
 * Build the popup's media list from the streams the sniffer caught on a
 * tab.
 *
 * `infoFor` supplies each HLS manifest's already-known facts —
 * `peekManifest` for the synchronous fast path, the resolved results for
 * the background path. Empty variants mean "no qualities to show",
 * whether that is because the manifest is not a master playlist or simply
 * because nothing has resolved yet. Either way the stream renders as a
 * single plain row — but that row still shows a duration when one is
 * known, which is what a media playlist (no variants, real duration)
 * contributes.
 *
 * `tabId` is the fallback for a sniffed stream that carries none of its
 * own.
 */
export function assembleStreams(
  sniffed: readonly MediaStream[],
  tabId: number | null,
  infoFor: (manifestUrl: string) => ManifestInfo,
): PopupMediaStream[] {
  const parsed = sniffed.map((s) => {
    const info = s.kind === "hls" ? infoFor(s.manifestUrl) : NOTHING_KNOWN;
    return { s, variants: info.variants, durationSecs: info.durationSecs };
  });

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
      // Only meaningful on a plain row: a quality row reads the duration
      // off its own variant, which carries the matching size estimate.
      ...(p.durationSecs != null ? { durationSecs: p.durationSecs } : {}),
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
    // Duration is compared, not just the variant URLs. A media playlist
    // resolves to the same zero variants it started with and differs
    // only by having learned a duration — suppressing that broadcast
    // would leave the row permanently blank.
    if (left.durationSecs !== right.durationSecs) return false;
    const lv = left.variants ?? [];
    const rv = right.variants ?? [];
    return lv.length === rv.length && lv.every((v, j) => v.url === rv[j]!.url);
  });
}
