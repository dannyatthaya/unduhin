// Pure decision logic for the download interceptor. Lives in its own
// module so it can be unit-tested without a `chrome` global — every
// `chrome.*` call belongs in `download-interceptor.ts`.
//
// The rule order matches the user-facing settings page, with the
// `mode` gate placed *after* the global-enable check and scheme
// check so a `passthrough` mode skips host parsing entirely. Order:
//   1. Global enable gate.
//   2. Scheme — only http(s) is ever intercepted; everything else is a
//      passthrough (blob:, data:, file:, chrome-extension:, etc.).
//   3. Mode gate:
//        passthrough → always passthrough
//        rules-only  → require an alwaysInterceptHosts hit
//        ask-first   → defer to a user prompt (download-interceptor
//                      handles the round-trip; this function returns
//                      `{ kind: "ask" }` so it stays pure)
//        catch-all   → preserve the existing filter pipeline below
//   4. Blocked hosts — hard veto, beats every other rule.
//   5. Always-intercept hosts — bypass the size + extension filters.
//   6. Min size threshold (in MiB; unknown size is treated as "above
//      threshold" so we don't lose downloads where the server didn't
//      send Content-Length).
//   7. Extension allowlist — if non-empty, the file's extension must be
//      in it.
//   8. Extension blocklist — if the extension is in it, passthrough.
//   9. File-types allowlist — when non-empty, only listed file
//      extensions are intercepted. Mirrors the extension-allowlist
//      semantics but bound to the Tauri panel's pill grid.

import type { Settings } from "../shared/types.js";

export type ShouldInterceptDecision =
  | { readonly kind: "intercept"; readonly matchedPattern?: string }
  | {
      readonly kind: "passthrough";
      readonly reason: string;
      readonly matchedPattern?: string;
    }
  | { readonly kind: "ask" };

export interface InterceptInputs {
  readonly url: string;
  readonly filename: string;
  /** Chrome reports -1 (or 0 on some builds) when Content-Length was absent. */
  readonly size: number;
  readonly settings: Settings;
}

const NON_HTTP_SCHEME = /^(?:blob|data|file|chrome-extension|chrome|about|javascript|ws|wss|ftp):/i;

export function shouldIntercept(input: InterceptInputs): ShouldInterceptDecision {
  const { url, filename, size, settings } = input;

  if (!settings.enabled) return { kind: "passthrough", reason: "extension disabled" };

  if (NON_HTTP_SCHEME.test(url)) {
    return { kind: "passthrough", reason: "non-http scheme" };
  }

  let host: string;
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      return { kind: "passthrough", reason: "non-http scheme" };
    }
    host = parsed.hostname.toLowerCase();
  } catch {
    return { kind: "passthrough", reason: "url parse failed" };
  }

  const blockedHit = settings.blockedHosts.find((rule) =>
    hostMatches(host, rule.pattern),
  );
  if (blockedHit) {
    return {
      kind: "passthrough",
      reason: "blocked host",
      matchedPattern: blockedHit.pattern,
    };
  }

  const alwaysHit = settings.alwaysInterceptHosts.find((rule) =>
    hostMatches(host, rule.pattern),
  );
  const alwaysIntercept = alwaysHit !== undefined;

  // Mode gate. `passthrough` is short-circuited at the top of the
  // function; here we only need to handle `rules-only` and `ask-first`.
  // `catch-all` falls through to the existing filter pipeline.
  if (settings.mode === "passthrough") {
    return { kind: "passthrough", reason: "mode: passthrough" };
  }
  if (settings.mode === "rules-only" && !alwaysIntercept) {
    return { kind: "passthrough", reason: "mode: rules-only (no host match)" };
  }
  // ask-first defers to the user; the caller resolves the prompt.
  // Always-intercept hosts skip the prompt — they're a stronger user
  // signal than the mode default.
  if (settings.mode === "ask-first" && !alwaysIntercept) {
    return { kind: "ask" };
  }

  if (!alwaysIntercept) {
    const minBytes = Math.max(0, settings.minSizeMb) * 1024 * 1024;
    // Treat <=0 as "size unknown" — intercept anyway so we don't drop
    // every server that omits Content-Length.
    if (size > 0 && size < minBytes) {
      return { kind: "passthrough", reason: "below min size" };
    }

    const ext = filenameExt(filename);
    if (settings.extensionAllowlist.length > 0 && !settings.extensionAllowlist.includes(ext)) {
      return { kind: "passthrough", reason: "extension not in allowlist" };
    }
    if (settings.extensionBlocklist.includes(ext)) {
      return { kind: "passthrough", reason: "extension in blocklist" };
    }
    if (settings.fileTypes.length > 0 && !settings.fileTypes.includes(ext)) {
      return { kind: "passthrough", reason: "file type not in capture list" };
    }
  }

  return alwaysHit
    ? { kind: "intercept", matchedPattern: alwaysHit.pattern }
    : { kind: "intercept" };
}

function filenameExt(filename: string): string {
  if (!filename) return "";
  // Strip any path separators a server-suggested filename might smuggle in.
  const tail = filename.split(/[\\/]/).pop() ?? filename;
  const i = tail.lastIndexOf(".");
  if (i <= 0 || i === tail.length - 1) return "";
  return tail.slice(i + 1).toLowerCase();
}

/**
 * `pattern` matches `host` if it's:
 *   - an exact match (`example.com`), or
 *   - a wildcard match (`*.example.com` matches `cdn.example.com` and
 *     `example.com` itself).
 * Anything else (empty, leading/trailing dots) returns false.
 */
function hostMatches(host: string, pattern: string): boolean {
  const p = pattern.trim().toLowerCase();
  if (!p) return false;
  if (p.startsWith("*.")) {
    const tail = p.slice(2);
    if (!tail) return false;
    return host === tail || host.endsWith("." + tail);
  }
  return host === p;
}
