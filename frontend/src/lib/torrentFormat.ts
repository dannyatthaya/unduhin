// Pure-function formatting + merge helpers for the torrent detail pane.
//
// Kept in `lib/` (no Vue / no Tauri imports) so the swarm-ratio formatting
// and the per-file progress merge are unit-testable without mounting
// `DetailTorrent.vue`. The view stays a thin presentational shell.
//
// Two surfaces consume this:
//   - DetailTorrent: the swarm stats strip (`formatRatio`) and the per-file
//     progress list (`fileProgressRows`).
//   - tests: pin the merge precedence + ratio edges.

import type { TorrentMeta, TorrentSource } from "@/types/tauri-bindings";
import type { TorrentFileLive } from "@/stores/downloads";

/**
 * Format a `ratio_milli` (upload/download ratio in thousandths, the
 * `SwarmStats.ratio_milli` wire shape) as a fixed two-decimal string:
 * `1500 → "1.50"`, `0 → "0.00"`. A negative or non-finite value clamps to
 * `"0.00"` so a malformed snapshot never renders `NaN`.
 */
export function formatRatio(milli: number | null | undefined): string {
  if (milli == null || !Number.isFinite(milli) || milli < 0) return "0.00";
  return (milli / 1000).toFixed(2);
}

/**
 * A one-line human summary of where a torrent came from, for the detail
 * pane's source row. Mirrors the `TorrentSource` discriminant.
 */
export function torrentSourceLabel(source: TorrentSource): string {
  switch (source.kind) {
    case "magnet":
      return source.uri;
    case "file":
      return source.path;
    case "info_hash":
      return source.hash;
  }
}

/** One row in the per-file progress list. Merges the persisted file list
 *  (path / length / selection) with the in-memory live byte counts. */
export interface TorrentFileRow {
  index: number;
  path: string;
  length: number;
  downloaded: number;
  pct: number;
  selected: boolean;
  done: boolean;
}

/**
 * Merge the persisted `TorrentMeta.files` (path, length, selection — survives
 * relaunch) with the in-memory `liveTorrentFiles` map (live byte counts that
 * are NOT persisted, exactly like `liveSegments`). Live bytes win for the
 * `downloaded`/`pct`/`done` columns; the persisted row supplies path, length,
 * and the user's file selection.
 *
 * Returns rows sorted by file index. When metadata hasn't resolved yet
 * (`files == null`), returns an empty list — the caller renders an
 * awaiting-metadata placeholder.
 *
 * A file with no live tick yet falls back to a 0-byte / 0% row (shape only),
 * so the list renders the full torrent before the first `torrent_file_progress`
 * event arrives.
 */
export function fileProgressRows(
  meta: TorrentMeta | null | undefined,
  live: ReadonlyMap<number, TorrentFileLive> | undefined,
): TorrentFileRow[] {
  const files = meta?.files;
  if (!files || files.length === 0) return [];
  return files
    .map((f) => {
      const l = live?.get(f.index);
      // Prefer the live `total` when present (it's the authoritative
      // librqbit length for that file); fall back to the persisted length.
      const total = l && l.total > 0 ? l.total : f.length;
      const downloaded = l ? l.downloaded : 0;
      const pct =
        total > 0 ? Math.min(100, Math.round((downloaded / total) * 100)) : 0;
      return {
        index: f.index,
        path: f.path,
        length: f.length,
        downloaded,
        pct,
        selected: f.selected,
        done: total > 0 && downloaded >= total,
      };
    })
    .sort((a, b) => a.index - b.index);
}
