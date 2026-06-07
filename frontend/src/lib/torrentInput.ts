// Pure-function detection + parsing for torrent inputs.
//
// Kept in `lib/` (no Vue / no Tauri imports) so the rules are
// unit-testable without mounting the AddUrlDialog or mocking `invoke`.
//
// Two surfaces consume this:
//   - AddUrlDialog: detect a `magnet:` paste / `.torrent` drop and branch
//     to the torrent path (probe metadata, show a file picker).
//   - useClipboardCapture: recognize a `magnet:` URI alongside http(s).
//
// The detection mirrors the backend's notion of a `TorrentSource`
// (crates/core/src/download.rs): a magnet URI, a `.torrent` file path, or
// a bare BitTorrent v1 infohash. We only ever produce the magnet / file
// variants from the frontend — a bare infohash isn't something the user
// pastes into the Add dialog, but we still expose the hash extractor so
// the magnet de-dup key (design §5.7) can be computed without a metadata
// fetch.

import type { TorrentSource } from "@/types/tauri-bindings";

/** A 40-char hex (BTv1) or 32-char base32 (also BTv1, older clients)
 *  infohash, as it appears in a magnet `xt=urn:btih:` parameter. */
const BTIH_HEX = /^[0-9a-f]{40}$/i;
const BTIH_BASE32 = /^[a-z2-7]{32}$/i;

/**
 * True when `raw` looks like a magnet URI. Case-insensitive on the
 * scheme (browsers lowercase it, but a hand-typed `MAGNET:` is legal).
 * Intentionally loose — the backend re-validates; this only decides
 * which add path the dialog takes.
 */
export function isMagnetUri(raw: string): boolean {
  return /^magnet:\?/i.test(raw.trim());
}

/**
 * True when `path` points at a `.torrent` file. Used for the drag-and-drop
 * path where the OS hands us a filesystem path (or a `file:` URL). The
 * extension check is case-insensitive and tolerates a trailing query/hash
 * the way a `file:` URL might carry one.
 */
export function isTorrentFile(path: string): boolean {
  const trimmed = path.trim().split(/[?#]/, 1)[0];
  return /\.torrent$/i.test(trimmed);
}

/**
 * Pull the lowercase hex infohash out of a magnet URI's `xt=urn:btih:`
 * parameter — the stable de-dup key (design §5.7). Returns `null` when the
 * URI has no `btih` topic or the hash is malformed. A base32 hash is
 * returned lowercased but NOT converted to hex (the backend owns the
 * canonical form once metadata resolves); it is still a usable session key.
 */
export function infoHashFromMagnet(raw: string): string | null {
  if (!isMagnetUri(raw)) return null;
  let params: URLSearchParams;
  try {
    // `magnet:?xt=…` — strip the scheme and parse the query.
    params = new URLSearchParams(raw.trim().slice("magnet:".length).replace(/^\?/, ""));
  } catch {
    return null;
  }
  // A magnet can carry multiple `xt` topics; the BitTorrent one is
  // `urn:btih:`. Scan all of them rather than assuming the first.
  for (const xt of params.getAll("xt")) {
    const m = /^urn:btih:(.+)$/i.exec(xt.trim());
    if (!m) continue;
    const hash = m[1].trim();
    if (BTIH_HEX.test(hash)) return hash.toLowerCase();
    if (BTIH_BASE32.test(hash)) return hash.toLowerCase();
  }
  return null;
}

/**
 * Pull the `dn=` (display name) parameter from a magnet URI, percent- and
 * plus-decoded. Returns `null` when absent or empty. Used as the
 * provisional name shown in the dialog before metadata resolves — the
 * backend does the same on insert (`provisional_torrent_name`).
 */
export function displayNameFromMagnet(raw: string): string | null {
  if (!isMagnetUri(raw)) return null;
  let params: URLSearchParams;
  try {
    params = new URLSearchParams(raw.trim().slice("magnet:".length).replace(/^\?/, ""));
  } catch {
    return null;
  }
  const dn = params.get("dn");
  if (!dn) return null;
  const trimmed = dn.trim();
  return trimmed.length > 0 ? trimmed : null;
}

/**
 * Classify a raw Add-dialog input as a torrent source, or `null` when it
 * isn't one (the dialog then falls through to the media/HTTP path).
 *
 * - `magnet:?…`                 → `{ kind: "magnet", uri }`
 * - a path/URL ending `.torrent`→ `{ kind: "file", path }`
 *
 * We never synthesize the `info_hash` variant here — that's a
 * resume/dedup detail the backend owns.
 */
/**
 * Translate the add-time file picker's selection into the
 * `TorrentMeta.selected_files` value: `null` when every file is selected
 * (the librqbit `only_files` default = download all), otherwise the chosen
 * file indices in ascending order. `totalFiles` is the torrent's file count;
 * `selected` is the set of selected indices.
 *
 * Returning `null` for "all selected" keeps the persisted blob compact and
 * lets the backend treat it as the no-filter case, exactly as the design
 * specifies (`selected_files: None` = all → librqbit `only_files`).
 */
export function selectedFileIndices(
  selected: ReadonlySet<number>,
  totalFiles: number,
): number[] | null {
  if (totalFiles > 0 && selected.size === totalFiles) return null;
  return [...selected].sort((a, b) => a - b);
}

export function detectTorrentSource(raw: string): TorrentSource | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  if (isMagnetUri(trimmed)) {
    return { kind: "magnet", uri: trimmed };
  }
  if (isTorrentFile(trimmed)) {
    // A `file:` URL carries the path in `pathname` (percent-encoded);
    // decode it so the backend gets a real filesystem path. A plain OS
    // path is passed through untouched.
    if (/^file:/i.test(trimmed)) {
      try {
        const u = new URL(trimmed);
        return { kind: "file", path: decodeURIComponent(u.pathname.replace(/^\/([A-Za-z]:)/, "$1")) };
      } catch {
        // Fall through to the raw path.
      }
    }
    return { kind: "file", path: trimmed };
  }
  return null;
}
