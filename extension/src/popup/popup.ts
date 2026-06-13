// Popup entry. Lifecycle:
//
//   1. On open, message the SW for a snapshot (bridge status, streams for
//      the active tab, recent jobs).
//   2. Subscribe to `bridge-status` broadcasts so the header dot updates
//      live without polling.
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
  PopupDownloadMediaRequest,
  PopupDownloadMediaResponse,
  PopupMediaStream,
  PopupRecentJob,
  PopupRefreshStatusRequest,
  PopupSnapshotRequest,
  PopupSnapshotResponse,
} from "../shared/types.js";
import { RECENT_JOBS_KEY } from "../background/recent-jobs.js";

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
  const snapshot = await requestSnapshot(currentTabId);
  renderBridgeStatus(snapshot.bridgeStatus);
  renderMedia(snapshot.streams);
  renderRecent(snapshot.recentJobs);
  installLiveSubscriptions();
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
    els.mediaList.appendChild(buildMediaRow(stream));
  }
}

function buildMediaRow(stream: PopupMediaStream): HTMLLIElement {
  const li = document.createElement("li");
  li.className = "media-list__item";

  const main = document.createElement("div");
  main.className = "media-list__main";

  const name = document.createElement("span");
  name.className = "media-list__name";
  name.textContent =
    stream.suggestedFilename && stream.suggestedFilename.length > 0
      ? stream.suggestedFilename
      : stream.manifestUrl;
  name.title = stream.manifestUrl;

  const meta = document.createElement("span");
  meta.className = "media-list__meta";
  meta.textContent = stream.kind;

  main.append(name, meta);

  const button = document.createElement("button");
  button.type = "button";
  button.className = "button";
  button.textContent = "Download";
  button.addEventListener("click", () => {
    void requestDownloadMedia(stream, button);
  });

  li.append(main, button);
  return li;
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
  stream: PopupMediaStream,
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
    manifestUrl: stream.manifestUrl,
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
