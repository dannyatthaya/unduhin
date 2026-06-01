// Recent-downloads ring buffer.
//
// Stores the last `RECENT_JOBS_CAP` jobs we've handed to the native host in
// `chrome.storage.session.recentJobs`. session storage lifetime is the
// browser process, which matches the popup's "feels recent" granularity
// without polluting `sync` storage with throw-away data.
//
// Two writers:
//   - `recordAck(msg, reply)` pushes a fresh entry whenever the host
//     acks a `download` / `downloadMedia` we sent. Status starts at
//     `"queued"`; the live status flows in via `mergeStatus`.
//   - `mergeStatus(entries)` patches the existing buffer with fresh
//     statuses pulled from an `Outbound::Status`. Matched by stringified
//     id — bigint doesn't survive a `chrome.storage` round-trip.

import { log } from "../shared/log.js";
import type {
  Inbound,
  Outbound,
  PopupRecentJob,
  StatusEntry,
} from "../shared/types.js";

export const RECENT_JOBS_KEY = "recentJobs";
export const RECENT_JOBS_CAP = 5;

export async function readRecentJobs(): Promise<PopupRecentJob[]> {
  return new Promise((resolve) => {
    chrome.storage.session.get({ [RECENT_JOBS_KEY]: [] }, (items) => {
      const raw = items[RECENT_JOBS_KEY];
      resolve(Array.isArray(raw) ? (raw as PopupRecentJob[]) : []);
    });
  });
}

async function writeRecentJobs(next: readonly PopupRecentJob[]): Promise<void> {
  return new Promise((resolve) => {
    chrome.storage.session.set({ [RECENT_JOBS_KEY]: next }, () => resolve());
  });
}

/** Record an ack reply against the request that produced it. No-op when
 * the message wasn't a download-shaped request, or the reply wasn't an
 * `ack` (errors and pongs are uninteresting here). */
export async function recordAck(msg: Inbound, reply: Outbound): Promise<void> {
  if (reply.type !== "ack") return;
  if (msg.type !== "download" && msg.type !== "downloadMedia") return;

  const filename =
    msg.type === "download"
      ? msg.job.filename ?? deriveLabel(msg.job.finalUrl)
      : msg.stream.suggestedFilename ?? deriveLabel(msg.stream.manifestUrl);

  const entry: PopupRecentJob = {
    id: String(reply.id),
    filename,
    status: "queued",
    at: Date.now(),
  };

  const existing = await readRecentJobs();
  // FIFO — newest first, capped. Drop any prior entry with the same id
  // so the popup doesn't show a duplicate after a retry.
  const next = [entry, ...existing.filter((e) => e.id !== entry.id)].slice(
    0,
    RECENT_JOBS_CAP,
  );
  await writeRecentJobs(next);
  log.debug("recent-jobs: pushed", entry);
}

/** Patch existing entries with statuses from a host `Status` reply. New
 * downloads visible to the host but not present in our buffer are not
 * added — the popup only cares about jobs *this browser* triggered. */
export async function mergeStatus(
  entries: readonly StatusEntry[],
): Promise<void> {
  const existing = await readRecentJobs();
  if (existing.length === 0) return;
  const byId = new Map(entries.map((e) => [String(e.id), e]));
  let changed = false;
  const next = existing.map((j) => {
    const live = byId.get(j.id);
    if (!live) return j;
    if (live.status === j.status) return j;
    changed = true;
    return { ...j, status: live.status };
  });
  if (!changed) return;
  await writeRecentJobs(next);
  log.debug("recent-jobs: merged status from host");
}

function deriveLabel(url: string): string {
  try {
    const u = new URL(url);
    const tail = u.pathname.split("/").filter(Boolean).pop();
    if (tail) return decodeURIComponent(tail);
    return u.host;
  } catch {
    return url;
  }
}
