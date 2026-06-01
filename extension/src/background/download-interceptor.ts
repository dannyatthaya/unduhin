// Download interceptor.
//
// `chrome.downloads.onDeterminingFilename` fires after the browser has
// decided to start a download. We don't (and can't) cancel it before it
// starts via this hook — the browser begins the download and we cancel
// it immediately after. The user-visible result is a brief blip on the
// shelf followed by the row disappearing (we chain `erase` for that).
//
// The decision is split in two: `shouldIntercept` in `intercept-rules.ts`
// is pure and exhaustively tested; this module wires it to the chrome
// APIs and the bridge.

import { log } from "../shared/log.js";
import type { DownloadJob, HandoffDecision, RequestHeader } from "../shared/types.js";
import { buildCookieHeader } from "./cookie-forwarder.js";
import type { HeaderCache } from "./header-capture.js";
import type { NativeBridge } from "./native-bridge.js";
import { shouldIntercept } from "./intercept-rules.js";
import { recordRuleHit } from "./rule-metrics.js";
import type { SettingsReader } from "../shared/settings.js";

/**
 * Round-trip an `ask-first` prompt. Implementation lives in
 * `service-worker.ts` because it needs the unsolicited frame router
 * already wired up there. Returns the user's choice or
 * `"passthrough"` if no response arrives within the timeout — we
 * default to the safer option (leave the download in the browser)
 * when the Tauri side never replies.
 */
export type AskHandoffFn = (id: string, job: DownloadJob) => Promise<HandoffDecision>;

export interface InterceptorDeps {
  readonly headerCache: HeaderCache;
  readonly bridge: NativeBridge;
  readonly settings: SettingsReader;
  readonly askHandoff: AskHandoffFn;
}

/**
 * One-shot session flag so we only notify "Unduhin not running" once per
 * service-worker session. `chrome.storage.session` survives transient SW
 * suspends but resets on browser restart — that's the granularity we
 * want.
 */
const NOT_RUNNING_FLAG = "unduhinNotRunningNotified";
const NOT_RUNNING_NOTIFICATION = "unduhin-not-running";

/**
 * 1×1 transparent PNG. `chrome.notifications.create` requires a non-empty
 * `iconUrl`; the real icon set lands later. This keeps the notification
 * surface functional today without shipping placeholder PNG binaries.
 */
const NOTIFICATION_ICON =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNgAAIAAAUAAarVyFEAAAAASUVORK5CYII=";

/**
 * Some hosts (pixeldrain.com, Google Drive interstitials) fire
 * `onDeterminingFilename` more than once for the same logical click —
 * an initial HTML pre-roll plus the eventual CDN URL share the same
 * `DownloadItem.id`. We must only run the cancel/forward/erase pipeline
 * once per id; the second fire's `cancel` would otherwise reject with
 * "Download must be in progress" once Chrome has already torn down the
 * record. The set is short-lived (cleared after the handler finishes),
 * so SW eviction can't leak it.
 */
const handlingIds = new Set<number>();

export function installDownloadInterceptor(deps: InterceptorDeps): void {
  chrome.downloads.onDeterminingFilename.addListener((item) => {
    if (handlingIds.has(item.id)) {
      log.debug("dedup: skipping duplicate onDeterminingFilename for", item.id);
      return;
    }
    handlingIds.add(item.id);
    // We never call `suggest()` — accepting the browser's default filename
    // is fine because we immediately cancel + erase. The handler runs
    // async; chrome.downloads doesn't wait for it.
    void handleItem(item, deps).finally(() => handlingIds.delete(item.id));
  });
  log.info("download-interceptor installed");
}

async function handleItem(
  item: chrome.downloads.DownloadItem,
  deps: InterceptorDeps,
): Promise<void> {
  const settings = deps.settings.current();
  const url = pickUrl(item);
  const filename = pickFilename(item);

  const decision = shouldIntercept({
    url,
    filename,
    size: item.totalBytes,
    settings,
  });
  // Record a hit for any rule that decided the outcome. Buffered;
  // the alarm tick pushes the snapshot to Tauri every 6 s.
  if (decision.kind !== "ask" && decision.matchedPattern) {
    recordRuleHit(decision.matchedPattern);
  }
  if (decision.kind === "passthrough") {
    log.debug("passthrough:", decision.reason, "→", url);
    return;
  }

  // The bridge-unreachable path: do *not* cancel. The user's download
  // proceeds in the browser, and we toast once per session so they know
  // why we didn't grab it.
  if (!deps.bridge.isHealthy()) {
    log.info("bridge unhealthy — leaving download to the browser:", url);
    await notifyNotRunningOnce();
    return;
  }

  // ask-first: build the job up front (we may need to send it twice
  // — once for the prompt, once for the actual download) and let the
  // Tauri prompt resolve before we touch chrome.downloads.
  const referrer = item.referrer && item.referrer.length > 0 ? item.referrer : null;
  const job = await buildJob({
    item,
    url,
    filename,
    referrer,
    cached: deps.headerCache.getHeadersFor(url) ?? [],
    forwardCookies: settings.forwardCookies,
  });

  if (decision.kind === "ask") {
    const id = newAskId();
    const choice = await deps.askHandoff(id, job);
    if (choice !== "capture") {
      log.debug("ask-first: user declined → passthrough", url);
      return;
    }
  }

  // The browser download has been running this whole time — in ask-first
  // mode for as long as the (up to 60s) prompt was open. If it already
  // finished, `chrome.downloads.cancel` is a no-op (the file stays) and
  // downloading again in Unduhin would leave two copies. Accept the
  // browser's copy and stop. (Bug: ask-first can produce a duplicate file.)
  if (await isDownloadComplete(item.id)) {
    log.debug("browser download already finished → keeping it, not re-downloading", url);
    return;
  }

  // Hand the job to the native host *before* cancelling the browser
  // download. If the host is unreachable (crash / pipe drop during the
  // build/prompt window), the browser download simply proceeds as a
  // fallback and we notify once — previously we cancelled first and a
  // failed send lost the download entirely. (Bug: interrupted handoff
  // loses the download.)
  try {
    await deps.bridge.send({ type: "download", job });
  } catch (err) {
    log.warn("bridge.send(download) failed; leaving download to the browser", err);
    await notifyNotRunningOnce();
    return;
  }

  // The host accepted the job — now cancel the browser's copy.
  try {
    await chrome.downloads.cancel(item.id);
  } catch (err) {
    // Chrome's promise-form `cancel` also sets `chrome.runtime.lastError`
    // as a side effect; reading it here clears the slot so the runtime
    // doesn't log "Unchecked runtime.lastError" on the next tick. A cancel
    // failure here (e.g. the item completed in the race) is benign: Unduhin
    // owns the job and at worst there's a harmless duplicate.
    void chrome.runtime.lastError;
    log.warn("downloads.cancel failed after handoff", err);
  }

  // Respect `hideShelf`. When false, leave the cancelled row on
  // the shelf so the user has a visible breadcrumb to revisit.
  if (settings.hideShelf) {
    try {
      await chrome.downloads.erase({ id: item.id });
    } catch {
      void chrome.runtime.lastError;
    }
  }
}

/** Whether the browser download `id` has already finished. Used to avoid
 * re-downloading a file the browser already saved (cancelling a completed
 * download is a no-op, so without this check Unduhin would make a second
 * copy). Returns false on any lookup error. */
async function isDownloadComplete(id: number): Promise<boolean> {
  try {
    const [found] = await chrome.downloads.search({ id });
    return found?.state === "complete";
  } catch {
    void chrome.runtime.lastError;
    return false;
  }
}

interface BuildJobInput {
  readonly item: chrome.downloads.DownloadItem;
  readonly url: string;
  readonly filename: string;
  readonly referrer: string | null;
  readonly cached: readonly chrome.webRequest.HttpHeader[];
  readonly forwardCookies: boolean;
}

async function buildJob(input: BuildJobInput): Promise<DownloadJob> {
  let cookieHeader = "";
  if (input.forwardCookies) {
    try {
      cookieHeader = await buildCookieHeader(input.url);
    } catch (err) {
      log.warn("buildCookieHeader failed", err);
    }
  }
  const requestHeaders: RequestHeader[] = input.cached
    .filter((h) => typeof h.name === "string" && h.name.length > 0)
    .map((h) => ({ name: h.name, value: typeof h.value === "string" ? h.value : "" }));
  return {
    finalUrl: input.url,
    originalUrl: input.item.url || input.url,
    referrer: input.referrer,
    filename: input.filename.length > 0 ? input.filename : null,
    mime: input.item.mime && input.item.mime.length > 0 ? input.item.mime : null,
    size: input.item.totalBytes > 0 ? input.item.totalBytes : null,
    cookieHeader: cookieHeader.length > 0 ? cookieHeader : null,
    userAgent: readUserAgent(),
    requestHeaders,
    tabId: null,
    pageUrl: null,
  };
}

function newAskId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `ask-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

function pickUrl(item: chrome.downloads.DownloadItem): string {
  if (item.finalUrl && item.finalUrl.length > 0) return item.finalUrl;
  return item.url ?? "";
}

function pickFilename(item: chrome.downloads.DownloadItem): string {
  if (item.filename && item.filename.length > 0) {
    return item.filename.split(/[\\/]/).pop() ?? item.filename;
  }
  try {
    const tail = new URL(pickUrl(item)).pathname.split("/").filter(Boolean).pop();
    return tail ? decodeURIComponent(tail) : "";
  } catch {
    return "";
  }
}

function readUserAgent(): string | null {
  // `navigator` is available in service workers; guard defensively for
  // a test/headless harness.
  if (typeof navigator !== "undefined" && typeof navigator.userAgent === "string") {
    return navigator.userAgent;
  }
  return null;
}

async function notifyNotRunningOnce(): Promise<void> {
  // `chrome.storage.session` lifetime is "the current browser process",
  // which matches what we want: re-notify on browser restart but stay
  // quiet across SW sleeps.
  const seen = await new Promise<boolean>((resolve) => {
    chrome.storage.session.get({ [NOT_RUNNING_FLAG]: false }, (items) => {
      resolve(items[NOT_RUNNING_FLAG] === true);
    });
  });
  if (seen) return;
  await new Promise<void>((resolve) => {
    chrome.storage.session.set({ [NOT_RUNNING_FLAG]: true }, () => resolve());
  });

  try {
    chrome.notifications.create(NOT_RUNNING_NOTIFICATION, {
      type: "basic",
      title: "Unduhin is not running",
      message: "Start the Unduhin app to capture browser downloads.",
      iconUrl: NOTIFICATION_ICON,
      priority: 1,
    });
  } catch (err) {
    log.warn("notifications.create failed", err);
  }
}

export { shouldIntercept } from "./intercept-rules.js";
