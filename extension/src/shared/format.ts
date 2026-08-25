// Display formatting for the popup's media rows.
//
// Pure functions, kept out of `popup.ts` so they can be tested without a
// DOM. Every one returns `null` — never a placeholder such as "—" — when
// it has nothing to show. The popup renders a row from a list of chips
// and drops the empty ones, so a null here means "omit this chip", which
// is what keeps a row with two known facts from printing three dashes
// next to them.
//
// Units match `frontend/src/lib/format.ts`: binary (1024-based) for
// bytes, decimal for bit rates. That split is not an inconsistency — it
// is what both the operating system's file browser and every stream
// manifest do, so the numbers agree with what the user sees elsewhere.

const KB = 1024;
const MB = KB * 1024;
const GB = MB * 1024;
const TB = GB * 1024;

/**
 * A byte count as a short human string, e.g. `2.7 GB`.
 *
 * The caller marks the value as an estimate, not this function — the
 * popup shows sizes it derived from a bit rate, and the desktop app may
 * one day show exact ones. Baking a "~" in here would make the second
 * case impossible without a second function.
 */
export function formatBytes(n: number | null | undefined): string | null {
  if (n == null || !Number.isFinite(n) || n <= 0) return null;
  if (n < KB) return `${Math.round(n)} B`;
  if (n < MB) return `${(n / KB).toFixed(0)} KB`;
  if (n < GB) return `${(n / MB).toFixed(0)} MB`;
  if (n < TB) return `${(n / GB).toFixed(1)} GB`;
  return `${(n / TB).toFixed(1)} TB`;
}

/**
 * A duration in seconds as `h:mm:ss`, or `m:ss` below one hour.
 *
 * Minutes are not zero-padded in the short form. `4:07` reads as four
 * minutes. `04:07` invites a read of four hours seven minutes.
 */
export function formatDuration(secs: number | null | undefined): string | null {
  if (secs == null || !Number.isFinite(secs) || secs <= 0) return null;
  const total = Math.round(secs);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  const pad = (n: number): string => n.toString().padStart(2, "0");
  return hours > 0
    ? `${hours}:${pad(minutes)}:${pad(seconds)}`
    : `${minutes}:${pad(seconds)}`;
}

/** A bit rate in bits per second as `6.0 Mbps` or `800 kbps`. */
export function formatBitrate(bitsPerSec: number | null | undefined): string | null {
  if (bitsPerSec == null || !Number.isFinite(bitsPerSec) || bitsPerSec <= 0) {
    return null;
  }
  if (bitsPerSec >= 1_000_000) return `${(bitsPerSec / 1_000_000).toFixed(1)} Mbps`;
  return `${Math.round(bitsPerSec / 1000)} kbps`;
}

/** A frame rate as `30 fps`. Fractional rates keep two decimals, so
 *  29.97 does not read as 30 next to a true 30. */
export function formatFrameRate(fps: number | null | undefined): string | null {
  if (fps == null || !Number.isFinite(fps) || fps <= 0) return null;
  const rounded = Math.round(fps);
  return Math.abs(fps - rounded) < 0.01 ? `${rounded} fps` : `${fps.toFixed(2)} fps`;
}

// Codec identifiers, mapped by prefix to the name the user knows. Both
// discovery paths produce the same identifiers — the manifest parser
// reads them out of an HLS `CODECS` attribute, and yt-dlp reports the
// same strings — so this one table serves both.
//
// Longest prefix wins, which is why the table is a list and not an
// object: `mp4a.40.5` (HE-AAC) must beat `mp4a` (AAC).
const CODEC_NAMES: readonly (readonly [string, string])[] = [
  // Video.
  ["avc1", "H.264"],
  ["avc3", "H.264"],
  ["h264", "H.264"],
  ["hvc1", "H.265"],
  ["hev1", "H.265"],
  ["h265", "H.265"],
  ["dvh1", "Dolby Vision"],
  ["dvhe", "Dolby Vision"],
  ["av01", "AV1"],
  ["vp09", "VP9"],
  ["vp9", "VP9"],
  ["vp08", "VP8"],
  ["vp8", "VP8"],
  // Audio.
  ["mp4a.40.5", "HE-AAC"],
  ["mp4a.40.29", "HE-AAC"],
  ["mp4a.40.2", "AAC"],
  ["mp4a", "AAC"],
  ["aac", "AAC"],
  ["ec-3", "E-AC-3"],
  ["eac3", "E-AC-3"],
  ["ac-3", "AC-3"],
  ["ac3", "AC-3"],
  ["opus", "Opus"],
  ["vorbis", "Vorbis"],
  ["flac", "FLAC"],
  ["mp3", "MP3"],
];

/**
 * The short name for one codec identifier, e.g. `avc1.640028` becomes
 * `H.264`.
 *
 * An unrecognized identifier comes back unchanged rather than as null: a
 * raw `xyz1.2` still tells the user more than a blank does, and a codec
 * this table does not know yet is a gap in the table, not a reason to
 * hide the fact. yt-dlp's `none` sentinel is already stripped on the
 * native side, and an empty string yields null.
 */
export function codecName(raw: string | null | undefined): string | null {
  const id = raw?.trim().toLowerCase();
  if (!id || id === "none") return null;
  let best: readonly [string, string] | null = null;
  for (const entry of CODEC_NAMES) {
    if (!id.startsWith(entry[0])) continue;
    if (!best || entry[0].length > best[0].length) best = entry;
  }
  return best ? best[1] : raw!.trim();
}

/**
 * Split an HLS `CODECS` attribute into a video identifier and an audio
 * one, e.g. `avc1.640028,mp4a.40.2`.
 *
 * The attribute is an unordered list, so each entry is classified by its
 * own prefix rather than by position. An entry matching neither list is
 * dropped: subtitle and closed-caption codecs appear here too, and they
 * are not what the row is describing.
 */
export function splitCodecs(attr: string | null | undefined): {
  video: string | null;
  audio: string | null;
} {
  const out: { video: string | null; audio: string | null } = {
    video: null,
    audio: null,
  };
  if (!attr) return out;
  for (const part of attr.split(",")) {
    const id = part.trim();
    if (id.length === 0) continue;
    const lower = id.toLowerCase();
    if (!out.video && VIDEO_PREFIXES.some((p) => lower.startsWith(p))) {
      out.video = id;
    } else if (!out.audio && AUDIO_PREFIXES.some((p) => lower.startsWith(p))) {
      out.audio = id;
    }
  }
  return out;
}

const VIDEO_PREFIXES = [
  "avc1",
  "avc3",
  "h264",
  "hvc1",
  "hev1",
  "h265",
  "dvh1",
  "dvhe",
  "av01",
  "vp0",
  "vp8",
  "vp9",
];
const AUDIO_PREFIXES = [
  "mp4a",
  "aac",
  "ac-3",
  "ac3",
  "ec-3",
  "eac3",
  "opus",
  "vorbis",
  "flac",
  "mp3",
];
