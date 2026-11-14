// Small formatting helpers shared by the UI. Pure functions — easy to
// unit-test if we ever need to.

const KB = 1024;
const MB = KB * 1024;
const GB = MB * 1024;
const TB = GB * 1024;

export function formatBytes(n: number | null | undefined, fractionDigits = 1): string {
  if (n == null || !Number.isFinite(n)) return "—";
  if (n < KB) return `${Math.round(n)} B`;
  if (n < MB) return `${(n / KB).toFixed(fractionDigits)} KB`;
  if (n < GB) return `${(n / MB).toFixed(fractionDigits)} MB`;
  if (n < TB) return `${(n / GB).toFixed(fractionDigits)} GB`;
  return `${(n / TB).toFixed(fractionDigits)} TB`;
}

export function formatSpeed(bps: number | null | undefined): string {
  if (bps == null || bps <= 0) return "—";
  return `${formatBytes(bps)}/s`;
}

export function formatEta(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds) || seconds <= 0) return "—";
  const s = Math.round(seconds);
  const hh = Math.floor(s / 3600);
  const mm = Math.floor((s % 3600) / 60);
  const ss = s % 60;
  const pad = (n: number) => n.toString().padStart(2, "0");
  return hh > 0 ? `${pad(hh)}:${pad(mm)}:${pad(ss)}` : `${pad(mm)}:${pad(ss)}`;
}

export function percent(done: number, total: number | null | undefined): number {
  if (total == null || total <= 0) return 0;
  return Math.min(100, Math.max(0, Math.round((done / total) * 100)));
}

export function shortenUrl(url: string, max = 32): string {
  try {
    const u = new URL(url);
    const tail = `${u.hostname}${u.pathname}`;
    if (tail.length <= max) return tail;
    return `${tail.slice(0, max - 1)}…`;
  } catch {
    return url.length > max ? `${url.slice(0, max - 1)}…` : url;
  }
}

// Cap a filename's length while keeping it recognizable. Direct-download
// hosts hand out absurdly long opaque slugs (100+ chars) that, untruncated,
// wrap the window title bar onto a second line. We elide the middle and
// keep the extension visible — `abc…very…long…xyz.mkv` reads better than a
// hard left-cut that loses the extension.
export function truncateFilename(name: string, max = 64): string {
  if (!name || name.length <= max) return name;
  const ellipsis = "…";
  const dot = name.lastIndexOf(".");
  // Treat a trailing chunk as an extension only when it's short and not the
  // whole string (so slugs containing dots aren't mistaken for one).
  const ext =
    dot > 0 && name.length - dot <= 6 ? name.slice(dot) : "";
  const budget = max - ext.length - ellipsis.length;
  if (budget <= 0) return `${name.slice(0, max - 1)}${ellipsis}`;
  const head = Math.ceil(budget / 2);
  const tail = budget - head;
  const stem = name.slice(0, dot > 0 && ext ? dot : name.length);
  const tailPart = tail > 0 ? stem.slice(stem.length - tail) : "";
  return `${stem.slice(0, head)}${ellipsis}${tailPart}${ext}`;
}

export function extOf(filename: string): string {
  const i = filename.lastIndexOf(".");
  if (i < 0 || i === filename.length - 1) return "";
  return filename.slice(i + 1).toUpperCase().slice(0, 4);
}

export function relativeTime(iso: string | null | undefined, nowMs = Date.now()): string {
  if (!iso) return "—";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "—";
  const delta = Math.max(0, nowMs - t);
  const s = Math.floor(delta / 1000);
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.floor(h / 24);
  return `${d}d ago`;
}

export function shortenPath(p: string, max = 36): string {
  if (!p) return "";
  if (p.length <= max) return p;
  const i = p.indexOf("\\", 3);
  if (i < 0) return `${p.slice(0, max - 1)}…`;
  const head = p.slice(0, i);
  const tail = p.slice(Math.max(i, p.length - (max - head.length - 4)));
  return `${head}\\…${tail}`;
}

/** Hostname of a URL, or "" when it has none (magnet links, bare paths). */
export function hostOf(url: string): string {
  try {
    return new URL(url).hostname;
  } catch {
    return "";
  }
}

/**
 * The folder a file sits in, cut to its last `segments` path parts —
 * `C:\Users\me\Downloads\Video\clip.mp4` reads as `Downloads\Video`.
 * Keeps whichever separator the path already uses.
 */
export function parentFolderLabel(path: string, segments = 2): string {
  if (!path) return "";
  const separator = path.includes("\\") ? "\\" : "/";
  const parts = path.split(/[\\/]+/).filter((part) => part !== "");
  const folders = parts.slice(0, -1);
  return folders.slice(-segments).join(separator);
}

export interface SpeedParts {
  value: string;
  /** Includes the leading space, e.g. " MB/s". Empty when idle. */
  unit: string;
}

/** `formatSpeed` split in two so the number and the unit can be styled apart. */
export function splitSpeed(bps: number | null | undefined): SpeedParts {
  const text = formatSpeed(bps);
  const space = text.indexOf(" ");
  if (space < 0) return { value: text, unit: "" };
  return { value: text.slice(0, space), unit: text.slice(space) };
}

/** A speed limit as a short label: whole numbers lose the ".0" (`10 MB/s`). */
export function formatSpeedLimit(bps: number): string {
  return formatSpeed(bps).replace(/\.0(?= )/, "");
}

export type DurationUnit = "s" | "min" | "h";

export interface ApproxDuration {
  n: number;
  unit: DurationUnit;
}

const SECONDS_PER_MINUTE = 60;
const SECONDS_PER_HOUR = 3600;

/** A duration rounded to the one unit a person would say it in. */
export function approxDuration(seconds: number): ApproxDuration {
  const s = Math.max(0, seconds);
  if (s < SECONDS_PER_MINUTE) return { n: Math.max(1, Math.round(s)), unit: "s" };
  if (s < SECONDS_PER_HOUR) return { n: Math.round(s / SECONDS_PER_MINUTE), unit: "min" };
  return { n: Math.round(s / SECONDS_PER_HOUR), unit: "h" };
}

function startOfLocalDay(ms: number): number {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** Whole local calendar days between `ms` and `nowMs` (0 = today, 1 = yesterday). */
export function calendarDaysAgo(ms: number, nowMs = Date.now()): number {
  const DAY_MS = 24 * 3600 * 1000;
  return Math.round((startOfLocalDay(nowMs) - startOfLocalDay(ms)) / DAY_MS);
}

/**
 * The "Added" column: a clock time for today, `yesterdayLabel` for
 * yesterday, otherwise a short date (with the year once it differs).
 */
export function formatAdded(
  iso: string | null | undefined,
  yesterdayLabel: string,
  locale?: string,
  nowMs = Date.now(),
): string {
  if (!iso) return "—";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "—";
  const daysAgo = calendarDaysAgo(t, nowMs);
  const d = new Date(t);
  if (daysAgo <= 0) {
    const pad = (n: number) => n.toString().padStart(2, "0");
    return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  }
  if (daysAgo === 1) return yesterdayLabel;
  const sameYear = d.getFullYear() === new Date(nowMs).getFullYear();
  return d.toLocaleDateString(locale, {
    day: "numeric",
    month: "short",
    ...(sameYear ? {} : { year: "numeric" }),
  });
}
