// Pure-function URL matcher for the clipboard watcher.
//
// Kept in `lib/` (no Vue / no Tauri imports) so the matching rules
// are unit-testable in isolation without mocking the clipboard plugin
// or the downloads store.
//
// The matcher answers a single question: "does this clipboard payload
// look like a download URL the user would want Unduhin to catch?"
//
// The matcher is intentionally conservative — false positives feel
// noisier than false negatives because each match surfaces a toast.

/**
 * Result of inspecting a clipboard payload. `null` means "no match —
 * stay silent". A non-null result carries the canonical URL and the
 * extension that was matched (the latter is only useful for telemetry
 * / debugging in the dev console).
 */
export interface ClipboardMatch {
  url: string;
  ext: string;
}

/**
 * Inspect a clipboard payload and return a [`ClipboardMatch`] iff the
 * payload looks like an HTTP(S) URL whose final path segment ends in
 * an allowlisted file extension.
 *
 * Rules:
 * - Must parse as a `URL` with scheme `http:` or `https:`.
 * - Path tail must contain a `.<ext>` suffix; `<ext>` is lowercased.
 * - `<ext>` must appear in the supplied allowlist (case-insensitive).
 *   An empty allowlist means *nothing matches* — different from
 *   `shouldIntercept`'s "empty = unrestricted" because for the
 *   clipboard surface the user has to opt in to *something* for the
 *   prompt to feel non-spammy.
 * - HTML pages, directory paths, and query-only URLs return `null`.
 */
export function matchClipboardPayload(
  raw: string,
  fileTypes: readonly string[],
): ClipboardMatch | null {
  if (!raw) return null;
  const text = raw.trim();
  if (text.length === 0 || text.length > 4096) return null;
  // Reject anything that visibly spans multiple lines; copying a paragraph
  // shouldn't trigger a capture even if some token in it parses as a URL.
  if (/[\n\r\t]/.test(text)) return null;

  let parsed: URL;
  try {
    parsed = new URL(text);
  } catch {
    return null;
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    return null;
  }

  const ext = extractExtension(parsed.pathname);
  if (!ext) return null;

  const allow = new Set(
    fileTypes.map((x) => x.trim().toLowerCase().replace(/^\./, "")),
  );
  if (!allow.has(ext)) return null;

  return { url: text, ext };
}

/**
 * Pull the lowercase trailing extension from a URL pathname. Returns
 * `null` for pathnames without a `.<ext>` tail or when the extension is
 * suspicious (too long, contains non-alnum). The same `[a-z0-9]{1,8}`
 * shape `shouldIntercept` uses extension-side so the two surfaces don't
 * disagree.
 */
function extractExtension(pathname: string): string | null {
  // Strip trailing slashes — a directory URL has no extension.
  const tail = pathname.replace(/\/+$/, "");
  if (!tail) return null;
  const lastSegment = tail.slice(tail.lastIndexOf("/") + 1);
  const dot = lastSegment.lastIndexOf(".");
  if (dot <= 0) return null;
  const ext = lastSegment.slice(dot + 1).toLowerCase();
  if (!/^[a-z0-9]{1,8}$/.test(ext)) return null;
  return ext;
}
