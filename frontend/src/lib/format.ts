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
