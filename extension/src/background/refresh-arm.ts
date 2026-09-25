/**
 * Armed link refreshes.
 *
 * When a download dies because its link expired, the app asks the user to
 * re-click the link on the site. The capture that follows looks exactly like
 * any other download, so something has to say "this one replaces row 42".
 *
 * That link cannot come from the page: `download-interceptor.ts` sends
 * `tabId: null, pageUrl: null` for intercepted downloads, so no row knows what
 * page produced it. Instead the app arms an entry here, out of band, carrying
 * the file name and byte size it expects. Matching on those two means this
 * works for rows created long before the feature existed — no backfill.
 *
 * Everything here is in-memory module state in an MV3 service worker, so
 * Chrome can evict it at any time. That is acceptable: losing an arm degrades
 * to "the capture becomes a normal new download", which is exactly what
 * happened before this feature. The app's dialog also times out on its own.
 */

const log = {
  info: (...a: unknown[]) => console.info("[refresh-arm]", ...a),
};

/** Most armed refreshes held at once. A user refreshing more than a handful of
 *  downloads at the same moment is not a real workflow, and an unbounded map in
 *  a service worker is a leak. Oldest expiry is evicted first. */
const MAX_ENTRIES = 5;

/** Hard cap on how long an entry lives, regardless of what the app asked for.
 *  The app sets its own expiry; this only stops a bad or hostile value from
 *  pinning an entry forever. */
const MAX_TTL_MS = 10 * 60 * 1000;

export interface ArmedRefresh {
  readonly downloadId: number;
  /** File name the dead row carries. Compared case-insensitively against the
   *  captured item's base name. */
  readonly filename: string | null;
  /** Expected size in bytes, or null when the row never learned one. */
  readonly sizeBytes: number | null;
  /** Origin of the dead URL. A capture must share a *site* with this or
   *  `referrerOrigin` (see `match`). Sites, not origins, because a CDN
   *  legitimately moves hosts. */
  readonly origin: string | null;
  /** Origin of the page the dead row was first captured from, when known. */
  readonly referrerOrigin: string | null;
  /** Epoch milliseconds. */
  readonly expiresAt: number;
}

export interface RefreshArmTable {
  arm(entry: ArmedRefresh): void;
  /** Best match for a capture, or null. Does NOT remove it — the caller
   *  removes only after the send succeeds, so a failed send can be retried.
   *  `from` is the capture's URL and its referring page: one of them must
   *  share a site with the arm, or any page could trigger a same-named
   *  download while an arm is live and have its file folded into the row. */
  match(
    filename: string,
    sizeBytes: number | null,
    from: readonly (string | null | undefined)[],
  ): ArmedRefresh | null;
  remove(downloadId: number): void;
  /** True when at least one entry is live. The interceptor checks this to
   *  decide whether an armed capture should override a passthrough rule. */
  hasAny(): boolean;
  /** Test seam. */
  clear(): void;
}

/** Second-level labels under which registrations sit one level deeper
 *  (`example.co.uk`, `example.com.au`). A heuristic stand-in for the Public
 *  Suffix List, which an extension cannot ship cheaply; it only has to tell
 *  "same site" from "different site" for a refresh, and it errs towards
 *  "different", which degrades to a normal new download. */
const SECOND_LEVEL_LABELS = new Set([
  "ac", "co", "com", "edu", "gob", "go", "gov", "ltd", "mil", "ne", "net", "nic", "or",
  "org", "plc", "sch",
]);

/** The registrable domain ("site") of a URL or origin, or null. IP
 *  addresses and single-label hosts are their own site. */
export function siteOf(raw: string | null | undefined): string | null {
  if (!raw) return null;
  let host: string;
  try {
    host = new URL(raw).hostname.toLowerCase().replace(/\.$/, "");
  } catch {
    return null;
  }
  if (!host) return null;
  if (host.startsWith("[") || /^\d+(\.\d+){3}$/.test(host)) return host;
  const labels = host.split(".");
  if (labels.length <= 2) return host;
  const tld = labels[labels.length - 1]!;
  const sld = labels[labels.length - 2]!;
  const keep = tld.length === 2 && SECOND_LEVEL_LABELS.has(sld) ? 3 : 2;
  return labels.slice(-keep).join(".");
}

/** Strip any directory part Chrome may have put in `DownloadItem.filename`,
 *  which is a full path on disk, and lowercase for comparison. */
export function baseName(path: string): string {
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return (cut >= 0 ? path.slice(cut + 1) : path).toLowerCase();
}

export function createRefreshArmTable(now: () => number = Date.now): RefreshArmTable {
  const entries = new Map<number, ArmedRefresh>();

  function sweep(): void {
    const t = now();
    for (const [id, e] of entries) {
      if (e.expiresAt <= t) entries.delete(id);
    }
  }

  return {
    arm(entry: ArmedRefresh): void {
      const capped = Math.min(entry.expiresAt, now() + MAX_TTL_MS);
      sweep();
      // Re-arming the same row replaces its entry rather than stacking.
      entries.delete(entry.downloadId);
      while (entries.size >= MAX_ENTRIES) {
        // Evict whatever expires soonest — it is the closest to useless.
        let oldest: number | null = null;
        let oldestAt = Infinity;
        for (const [id, e] of entries) {
          if (e.expiresAt < oldestAt) {
            oldestAt = e.expiresAt;
            oldest = id;
          }
        }
        if (oldest === null) break;
        entries.delete(oldest);
      }
      entries.set(entry.downloadId, { ...entry, expiresAt: capped });
      log.info(`armed ${entry.downloadId} (${entry.filename ?? "any name"})`);
    },

    match(
      filename: string,
      sizeBytes: number | null,
      from: readonly (string | null | undefined)[],
    ): ArmedRefresh | null {
      sweep();
      if (entries.size === 0) return null;
      const candidate = baseName(filename);
      const captureSites = new Set(from.map(siteOf).filter((s): s is string => s !== null));

      // Prefer a name+size match over a name-only match: if the user armed
      // two refreshes for files that share a name, the size disambiguates.
      let nameOnly: ArmedRefresh | null = null;
      for (const e of entries.values()) {
        if (e.filename === null) continue;
        if (baseName(e.filename) !== candidate) continue;
        const armSites = [siteOf(e.origin), siteOf(e.referrerOrigin)].filter(
          (s): s is string => s !== null,
        );
        if (armSites.length > 0 && !armSites.some((s) => captureSites.has(s))) {
          log.info(`not refreshing ${e.downloadId}: ${filename} came from another site`);
          continue;
        }
        if (e.sizeBytes !== null && sizeBytes !== null) {
          if (e.sizeBytes === sizeBytes) return e;
          // A same-named file of a different size is a different file. Do not
          // fall back to a name-only match on it.
          continue;
        }
        nameOnly ??= e;
      }
      return nameOnly;
    },

    remove(downloadId: number): void {
      entries.delete(downloadId);
    },

    hasAny(): boolean {
      sweep();
      return entries.size > 0;
    },

    clear(): void {
      entries.clear();
    },
  };
}
