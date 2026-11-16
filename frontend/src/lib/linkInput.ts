// What the text in New download's Link field is. Pure, so the dialog's
// "that isn't a link" state is unit-testable.

import { isMagnetUri, isTorrentFile } from "@/lib/torrentInput";

export type LinkKind =
  /** Nothing typed yet. */
  | "empty"
  /** An http(s) link the engine (or yt-dlp) can fetch. */
  | "http"
  /** A magnet URI or a path to a `.torrent` file. */
  | "torrent"
  /** Text that is none of the above. */
  | "invalid";

export function classifyLink(raw: string): LinkKind {
  const text = raw.trim();
  if (!text) return "empty";
  if (isMagnetUri(text) || isTorrentFile(text)) return "torrent";
  // A link has no spaces; prose and half-typed hosts do.
  if (/\s/.test(text)) return "invalid";
  try {
    const url = new URL(text);
    const isWeb = url.protocol === "http:" || url.protocol === "https:";
    return isWeb && url.hostname !== "" ? "http" : "invalid";
  } catch {
    return "invalid";
  }
}

/** The last path segment of a link, decoded, as a stand-in file name. */
export function fileNameFromLink(raw: string): string {
  try {
    const segments = new URL(raw.trim()).pathname.split("/").filter(Boolean);
    const tail = segments[segments.length - 1] ?? "";
    return decodeURIComponent(tail);
  } catch {
    return "";
  }
}

export interface NameParts {
  stem: string;
  /** Includes the dot (`.mp4`); empty when the name has no extension. */
  extension: string;
}

/**
 * Split a file name at its last dot. A leading dot is part of the stem
 * (`.gitignore` has no extension), and so is a trailing one.
 */
export function splitFileName(name: string): NameParts {
  const dot = name.lastIndexOf(".");
  if (dot <= 0 || dot === name.length - 1) return { stem: name, extension: "" };
  return { stem: name.slice(0, dot), extension: name.slice(dot) };
}

/** Lower-case extension without the dot, or "" when there is none. */
export function extensionOf(name: string): string {
  return splitFileName(name).extension.slice(1).toLowerCase();
}
