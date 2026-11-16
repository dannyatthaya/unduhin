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

/**
 * Whether a link points at this computer or the local network by address:
 * `localhost`, a loopback, private or link-local IPv4 address, or the IPv6
 * equivalents. New download does not check such a link on its own, so text
 * that merely lands in the Link field never sends a request to a device on
 * the user's network. A host *name* that resolves to a local address is not
 * caught here.
 */
export function isLocalNetworkLink(raw: string): boolean {
  let host: string;
  try {
    host = new URL(raw.trim()).hostname.toLowerCase();
  } catch {
    return false;
  }
  if (host === "localhost" || host.endsWith(".localhost")) return true;
  if (host.startsWith("[")) return isLocalIpv6(host.slice(1, -1));
  const octets = host.split(".").map(Number);
  const isIpv4 =
    octets.length === 4 && octets.every((n) => Number.isInteger(n) && n >= 0 && n <= 255);
  if (!isIpv4) return false;
  const [a, b] = octets;
  return (
    a === 0 ||
    a === 10 ||
    a === 127 ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    (a === 169 && b === 254)
  );
}

/** Loopback (`::1`), unique local (`fc00::/7`) or link-local (`fe80::/10`). */
function isLocalIpv6(address: string): boolean {
  return address === "::1" || /^f[cd]/.test(address) || /^fe[89ab]/.test(address);
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
