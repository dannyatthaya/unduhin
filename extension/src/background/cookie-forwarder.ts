// Cookie forwarder.
//
// Builds the `Cookie:` header string the browser *would* send on a fresh
// GET to the given URL. The native app replays this verbatim so range
// requests on auth-gated CDNs (Drive, Mega, S3 signed URLs) keep working.
//
// We rely on `chrome.cookies.getAll({ url })` — Chrome already applies
// the relevant filters (SameSite, Secure, HttpOnly is allowed via the
// API, expiry). RFC 6265 §5.4 says cookies should be sent ordered by:
//   1. Longest path first.
//   2. Earliest creation time first (Chrome surfaces this via
//      `creationTime` but ts type doesn't always — we fall back to a
//      stable insertion order if it's missing).
//
// CHIPS / partitioned cookies (Chrome 113+): `getAll` returns the
// unpartitioned set by default. Cross-site embeds that depend on
// partitioned cookies may not have their auth forwarded correctly — a
// known limitation documented for users in the extension README.
//
// Cookie stores: `getAll` without a `storeId` reads the regular profile's
// store. A tab in an incognito window has its own store, so a lookup made
// on behalf of a tab passes that tab's store — otherwise an incognito tab
// would be served the regular session's cookies.

import { log } from "../shared/log.js";

interface SortableCookie extends chrome.cookies.Cookie {
  // Some Chromium builds surface a `creationTime` epoch ms; the type
  // hasn't been pulled into `@types/chrome` yet. We read it via a
  // string-indexed access so the TS strict-mode flags don't complain.
  readonly [extra: string]: unknown;
}

/** The id of the cookie store `tabId` belongs to, from a
 *  `chrome.cookies.getAllCookieStores()` listing. `undefined` when no store
 *  lists the tab — the caller then reads the default store, as before. */
export function storeIdForTab(
  stores: readonly Pick<chrome.cookies.CookieStore, "id" | "tabIds">[],
  tabId: number,
): string | undefined {
  return stores.find((s) => s.tabIds.includes(tabId))?.id;
}

/** Build the `Cookie` header for `url`. Pass the originating tab when there
 *  is one, so its own cookie store is read (see the note on cookie stores
 *  above). */
export async function buildCookieHeader(url: string, tabId?: number | null): Promise<string> {
  let cookies: chrome.cookies.Cookie[];
  try {
    let storeId: string | undefined;
    if (typeof tabId === "number" && tabId >= 0) {
      storeId = storeIdForTab(await chrome.cookies.getAllCookieStores(), tabId);
    }
    cookies = await chrome.cookies.getAll(storeId ? { url, storeId } : { url });
  } catch (err) {
    log.warn("cookies.getAll failed", url, err);
    return "";
  }
  if (cookies.length === 0) return "";

  const sorted = [...cookies] as SortableCookie[];
  sorted.sort((a, b) => {
    const pathDiff = (b.path ?? "/").length - (a.path ?? "/").length;
    if (pathDiff !== 0) return pathDiff;
    const aTime = typeof a["creationTime"] === "number" ? a["creationTime"] : 0;
    const bTime = typeof b["creationTime"] === "number" ? b["creationTime"] : 0;
    return aTime - bTime;
  });

  // `name=value` joined by `; `. Empty cookie names are illegal per RFC
  // 6265; skip them defensively rather than emit `=value`.
  const parts: string[] = [];
  for (const c of sorted) {
    if (!c.name) continue;
    parts.push(`${c.name}=${c.value}`);
  }
  return parts.join("; ");
}
