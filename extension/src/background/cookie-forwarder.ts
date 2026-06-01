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

import { log } from "../shared/log.js";

interface SortableCookie extends chrome.cookies.Cookie {
  // Some Chromium builds surface a `creationTime` epoch ms; the type
  // hasn't been pulled into `@types/chrome` yet. We read it via a
  // string-indexed access so the TS strict-mode flags don't complain.
  readonly [extra: string]: unknown;
}

export async function buildCookieHeader(url: string): Promise<string> {
  let cookies: chrome.cookies.Cookie[];
  try {
    cookies = await chrome.cookies.getAll({ url });
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
