// Decides whether text pasted onto the downloads page is a link worth
// opening the New download dialog for. Pure, so it is unit-testable.

import { isMagnetUri } from "@/lib/torrentInput";

/**
 * Returns the trimmed link when `raw` is a single http(s) URL or a magnet
 * URI, otherwise null. Anything with whitespace inside is prose, not a link.
 */
export function downloadLinkFromPaste(raw: string): string | null {
  const text = raw.trim();
  if (!text || /\s/.test(text)) return null;
  if (isMagnetUri(text)) return text;
  try {
    const url = new URL(text);
    return url.protocol === "http:" || url.protocol === "https:" ? text : null;
  } catch {
    return null;
  }
}
