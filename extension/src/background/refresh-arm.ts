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
  /** Origin of the dead URL. Recorded for the app's confirmation prompt; not
   *  used for matching, because a CDN legitimately moves hosts. */
  readonly origin: string | null;
  /** Epoch milliseconds. */
  readonly expiresAt: number;
}

export interface RefreshArmTable {
  arm(entry: ArmedRefresh): void;
  /** Best match for a capture, or null. Does NOT remove it — the caller
   *  removes only after the send succeeds, so a failed send can be retried. */
  match(filename: string, sizeBytes: number | null): ArmedRefresh | null;
  remove(downloadId: number): void;
  /** True when at least one entry is live. The interceptor checks this to
   *  decide whether an armed capture should override a passthrough rule. */
  hasAny(): boolean;
  /** Test seam. */
  clear(): void;
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

    match(filename: string, sizeBytes: number | null): ArmedRefresh | null {
      sweep();
      if (entries.size === 0) return null;
      const candidate = baseName(filename);

      // Prefer a name+size match over a name-only match: if the user armed
      // two refreshes for files that share a name, the size disambiguates.
      let nameOnly: ArmedRefresh | null = null;
      for (const e of entries.values()) {
        if (e.filename === null) continue;
        if (baseName(e.filename) !== candidate) continue;
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
