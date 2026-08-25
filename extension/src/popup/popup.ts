// Popup entry. Lifecycle:
//
//   1. On open, message the SW for a snapshot (bridge status, streams for
//      the active tab, recent jobs). The snapshot never waits on the
//      network, so the list paints on the first frame.
//   2. Subscribe — before that round-trip, so nothing is missed — to
//      `bridge-status` broadcasts so the header dot updates live without
//      polling, and to `media-streams` so a stream whose qualities
//      weren't cached yet regroups into quality rows once the SW has
//      resolved them.
//   3. Listen for `chrome.storage.session.onChanged` on the recent-jobs
//      key so newly-completed downloads animate in even while the popup
//      is open.
//   4. Click handlers:
//      - Per-stream "Download" → send `popup-download-media` to the SW.
//      - "Refresh" → send `popup-refresh-status`; the SW asks the host
//        and patches `chrome.storage.session.recentJobs`, which our
//        storage listener picks up.
//      - "Options" → `chrome.runtime.openOptionsPage()`.
//
// No state lives in this module beyond what's needed for the current view.
// The popup teardown is implicit — closing the popup kills the document.

import type {
  BridgeStatus,
  BridgeStatusMessage,
  MediaStreamsMessage,
  MediaVariant,
  PopupDownloadMediaRequest,
  PopupDownloadMediaResponse,
  PopupMediaStream,
  PopupRecentJob,
  PopupRefreshStatusRequest,
  PopupSnapshotRequest,
  PopupSnapshotResponse,
} from "../shared/types.js";
import { RECENT_JOBS_KEY } from "../background/recent-jobs.js";
import {
  codecName,
  formatBitrate,
  formatBytes,
  formatDuration,
  formatFrameRate,
} from "../shared/format.js";

const STATUS_LABEL: Record<BridgeStatus, string> = {
  connected: "Connected to Unduhin",
  reconnecting: "Reconnecting…",
  disconnected: "Unduhin is not running",
};

const els = {
  status: document.querySelector<HTMLElement>(".bridge-status")!,
  statusLabel: document.querySelector<HTMLSpanElement>("#status-label")!,
  mediaList: document.querySelector<HTMLUListElement>("#media-list")!,
  mediaEmpty: document.querySelector<HTMLParagraphElement>("#media-empty")!,
  recentList: document.querySelector<HTMLUListElement>("#recent-list")!,
  recentEmpty: document.querySelector<HTMLParagraphElement>("#recent-empty")!,
  refreshButton: document.querySelector<HTMLButtonElement>("#refresh-button")!,
  optionsLink: document.querySelector<HTMLButtonElement>("#options-link")!,
  version: document.querySelector<HTMLSpanElement>("#ext-version")!,
};

let currentTabId: number | null = null;
let toastTimer: ReturnType<typeof setTimeout> | null = null;
/** Set once a `media-streams` broadcast has painted the media list, so the
 *  initial snapshot can't overwrite it with an older view. */
let hasLiveStreams = false;

void boot();

async function boot(): Promise<void> {
  els.version.textContent = `v${chrome.runtime.getManifest().version}`;
  els.optionsLink.addEventListener("click", () => {
    chrome.runtime.openOptionsPage();
  });
  els.refreshButton.addEventListener("click", () => {
    void refreshStatus();
  });
  currentTabId = await activeTabId();
  // Subscribe *before* the snapshot round-trip: the SW starts resolving
  // qualities as soon as it has served the snapshot, and a `media-streams`
  // broadcast that lands while we're still awaiting would otherwise be
  // dropped with no second chance. `currentTabId` is already set, so the
  // tab guard in the handler is valid from the first message.
  installLiveSubscriptions();
  const snapshot = await requestSnapshot(currentTabId);
  renderBridgeStatus(snapshot.bridgeStatus);
  // Unless a broadcast beat the reply here — resolution can finish inside
  // the round-trip when the cache was already warm — in which case the
  // snapshot is the older of the two and must not paint over it.
  if (!hasLiveStreams) renderMedia(snapshot.streams);
  renderRecent(snapshot.recentJobs);
  // Kick a status refresh on open so any pre-existing recent-job rows
  // reflect the latest host-side state, not a stale snapshot.
  void refreshStatus({ silent: true });
}

function installLiveSubscriptions(): void {
  chrome.runtime.onMessage.addListener((msg) => {
    if (!msg || typeof msg !== "object") return;
    if ("kind" in msg && msg.kind === "bridge-status") {
      renderBridgeStatus((msg as BridgeStatusMessage).status);
    }
    if ("kind" in msg && msg.kind === "media-streams") {
      const update = msg as MediaStreamsMessage;
      // Resolution finishing for some other tab is not a reason to
      // repaint what the user is looking at.
      if (update.tabId !== currentTabId) return;
      hasLiveStreams = true;
      renderMedia(update.streams);
    }
  });
  chrome.storage.onChanged.addListener((changes, area) => {
    if (area !== "session") return;
    const entry = changes[RECENT_JOBS_KEY];
    if (!entry) return;
    const next = Array.isArray(entry.newValue)
      ? (entry.newValue as PopupRecentJob[])
      : [];
    renderRecent(next);
  });
}

function renderBridgeStatus(status: BridgeStatus): void {
  els.status.dataset.status = status;
  els.statusLabel.textContent = STATUS_LABEL[status];
}

function renderMedia(streams: readonly PopupMediaStream[]): void {
  els.mediaList.replaceChildren();
  if (streams.length === 0) {
    els.mediaList.hidden = true;
    els.mediaEmpty.hidden = false;
    return;
  }
  els.mediaList.hidden = false;
  els.mediaEmpty.hidden = true;
  for (const stream of streams) {
    const variants = stream.variants ?? [];
    if (variants.length > 0) {
      // Master playlist → one row per quality, under a group label.
      els.mediaList.appendChild(buildGroupHeader(stream));
      for (const v of variants) {
        els.mediaList.appendChild(
          buildDownloadRow({
            name: v.label,
            titleAttr: v.url,
            chips: variantChips(v, stream),
            size: v.estimatedBytes,
            durationSecs: v.durationSecs,
            req: { manifestUrl: v.url, masterUrl: stream.manifestUrl },
          }),
        );
      }
    } else {
      const name =
        stream.suggestedFilename && stream.suggestedFilename.length > 0
          ? stream.suggestedFilename
          : stream.manifestUrl;
      els.mediaList.appendChild(
        buildDownloadRow({
          name,
          titleAttr: stream.manifestUrl,
          chips: [stream.kind.toUpperCase()],
          // A plain stream has no bit rate to estimate a size from, so
          // only the duration is ever known here.
          size: null,
          durationSecs: stream.durationSecs ?? null,
          req: { manifestUrl: stream.manifestUrl },
        }),
      );
    }
  }
}

/**
 * The facts shown under one quality's name, in order of how often the
 * user needs them: pixel size, then frame rate, then bit rate, then the
 * two codecs.
 *
 * Each fact is its own chip. There is no separator glyph between them —
 * the gap does that job, and a chip that is not known is simply absent
 * rather than a dash the user has to read past.
 */
function variantChips(
  variant: MediaVariant,
  stream: PopupMediaStream,
): string[] {
  const chips = [
    variant.resolution ?? stream.kind.toUpperCase(),
    formatFrameRate(variant.frameRate),
    formatBitrate(variant.bandwidth),
    codecName(variant.videoCodec),
    codecName(variant.audioCodec),
  ];
  return chips.filter((c): c is string => c != null && c.length > 0);
}

function buildGroupHeader(stream: PopupMediaStream): HTMLLIElement {
  const li = document.createElement("li");
  li.className = "media-list__group";
  const label =
    stream.suggestedFilename && stream.suggestedFilename.length > 0
      ? stream.suggestedFilename
      : "Adaptive stream";
  li.append(
    textSpan("media-list__group-name", label),
    textSpan("media-list__group-kind", stream.kind.toUpperCase()),
  );
  li.title = stream.manifestUrl;
  return li;
}

interface RowSpec {
  readonly name: string;
  readonly titleAttr: string;
  readonly chips: readonly string[];
  /** Estimated bytes, or null when no estimate is possible. */
  readonly size: number | null;
  readonly durationSecs: number | null;
  readonly req: { manifestUrl: string; masterUrl?: string };
}

function buildDownloadRow(spec: RowSpec): HTMLLIElement {
  const li = document.createElement("li");
  li.className = "media-list__item";

  const main = document.createElement("div");
  main.className = "media-list__main";

  const nameEl = textSpan("media-list__name", spec.name);
  nameEl.title = spec.titleAttr;

  const metaEl = document.createElement("span");
  metaEl.className = "media-list__meta";
  for (const chip of spec.chips) {
    metaEl.appendChild(textSpan("media-list__chip", chip));
  }

  main.append(nameEl, metaEl);

  // Size and length sit in their own column, right-aligned, because they
  // are the two numbers the user compares between rows. Lining them up
  // makes "which of these is the small one" a glance instead of a read.
  const figures = document.createElement("div");
  figures.className = "media-list__figures";
  const size = formatBytes(spec.size);
  if (size) {
    const sizeEl = textSpan("media-list__size", `~${size}`);
    // The tilde is easy to miss, so say it in full for anyone hovering
    // or using a screen reader.
    sizeEl.title = "Estimated from the stream bit rate. The real size can differ.";
    figures.appendChild(sizeEl);
  }
  const duration = formatDuration(spec.durationSecs);
  if (duration) figures.appendChild(textSpan("media-list__duration", duration));

  const button = document.createElement("button");
  button.type = "button";
  button.className = "button";
  button.textContent = "Download";
  button.addEventListener("click", () => {
    void requestDownloadMedia(spec.req, button);
  });

  li.append(main, figures, button);
  return li;
}

function textSpan(className: string, text: string): HTMLSpanElement {
  const el = document.createElement("span");
  el.className = className;
  el.textContent = text;
  return el;
}

function renderRecent(jobs: readonly PopupRecentJob[]): void {
  els.recentList.replaceChildren();
  if (jobs.length === 0) {
    els.recentList.hidden = true;
    els.recentEmpty.hidden = false;
    return;
  }
  els.recentList.hidden = false;
  els.recentEmpty.hidden = true;
  for (const job of jobs) {
    els.recentList.appendChild(buildRecentRow(job));
  }
}

function buildRecentRow(job: PopupRecentJob): HTMLLIElement {
  const li = document.createElement("li");
  li.className = "recent-list__item";

  const main = document.createElement("div");
  main.className = "recent-list__main";

  const name = document.createElement("span");
  name.className = "recent-list__name";
  name.textContent = job.filename;
  name.title = job.filename;

  const meta = document.createElement("span");
  meta.className = "recent-list__meta";

  const status = document.createElement("span");
  status.className = "recent-list__status";
  status.dataset.status = job.status.toLowerCase();
  status.textContent = job.status;

  const timestamp = document.createElement("span");
  timestamp.textContent = formatRelative(job.at);

  meta.append(status, timestamp);
  main.append(name, meta);
  li.append(main);
  return li;
}

function formatRelative(at: number): string {
  const seconds = Math.max(0, Math.floor((Date.now() - at) / 1000));
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ago`;
}

async function activeTabId(): Promise<number | null> {
  return new Promise((resolve) => {
    chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
      const id = tabs[0]?.id;
      resolve(typeof id === "number" && id >= 0 ? id : null);
    });
  });
}

async function requestSnapshot(
  tabId: number | null,
): Promise<PopupSnapshotResponse> {
  const req: PopupSnapshotRequest = {
    kind: "popup-snapshot",
    ...(tabId == null ? {} : { tabId }),
  };
  try {
    const reply = (await chrome.runtime.sendMessage(req)) as
      | PopupSnapshotResponse
      | undefined;
    if (reply && typeof reply === "object") return reply;
  } catch {
    // The SW may have just woken — retry once. Failure here is benign;
    // we fall through to the empty snapshot below.
  }
  return {
    bridgeStatus: "disconnected",
    streams: [],
    recentJobs: [],
  };
}

async function requestDownloadMedia(
  opts: { manifestUrl: string; masterUrl?: string },
  button: HTMLButtonElement,
): Promise<void> {
  if (currentTabId == null) {
    showToast("No active tab", "error");
    return;
  }
  button.disabled = true;
  const req: PopupDownloadMediaRequest = {
    kind: "popup-download-media",
    tabId: currentTabId,
    manifestUrl: opts.manifestUrl,
    ...(opts.masterUrl ? { masterUrl: opts.masterUrl } : {}),
  };
  try {
    const reply = (await chrome.runtime.sendMessage(
      req,
    )) as PopupDownloadMediaResponse;
    if (reply?.ok) {
      showToast("Sent to Unduhin");
    } else {
      showToast(reply?.error ?? "Send failed", "error");
    }
  } catch (err) {
    showToast(err instanceof Error ? err.message : String(err), "error");
  } finally {
    button.disabled = false;
  }
}

async function refreshStatus({ silent = false } = {}): Promise<void> {
  const req: PopupRefreshStatusRequest = { kind: "popup-refresh-status" };
  try {
    await chrome.runtime.sendMessage(req);
    if (!silent) showToast("Refreshed");
  } catch (err) {
    if (!silent) {
      showToast(err instanceof Error ? err.message : String(err), "error");
    }
  }
}

function showToast(message: string, tone: "info" | "error" = "info"): void {
  let toast = document.querySelector<HTMLDivElement>(".toast");
  if (!toast) {
    toast = document.createElement("div");
    toast.className = "toast";
    document.body.appendChild(toast);
  }
  toast.textContent = message;
  toast.dataset.tone = tone;
  toast.classList.add("is-visible");
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    toast?.classList.remove("is-visible");
  }, 1800);
}
