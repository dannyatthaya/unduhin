// Download interceptor.
//
// We intercept on `chrome.downloads.onCreated` — the EARLIEST download
// event. `onCreated` fires before `onDeterminingFilename`, which in turn
// fires before Chrome shows the "Save As" prompt; cancelling here (rather
// than at the later `onDeterminingFilename`, which does NOT reliably
// suppress that dialog) stops the browser before any dialog or bytes.
//
// Flow for a captured download: decide (`shouldIntercept`) → if the app is
// reachable, cancel the browser's copy immediately → hand the job to the
// native app. If the app is unreachable we leave the download to the
// browser as a fallback (and never cancel). For `ask-first` we cancel
// up front to kill the dialog, prompt, and re-issue the browser download
// if the user declines.
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
 * Guards against handling the same `DownloadItem.id` twice (defensive —
 * `onCreated` fires once per id, but a stray duplicate would otherwise
 * re-run the cancel/forward pipeline and the second `cancel` would reject
 * once Chrome has torn the record down). Short-lived: cleared after the
 * handler finishes, so SW eviction can't leak it.
 */
const handlingIds = new Set<number>();

/**
 * URLs we deliberately re-issued to the browser (after an `ask-first`
 * decline, or when a hand-off failed post-cancel). The resulting
 * `onCreated` for that URL is passed through once instead of being
 * re-intercepted — without this guard an `ask-first` decline would loop
 * forever (cancel → re-download → prompt → decline → …).
 */
const letThrough = new Set<string>();

export function installDownloadInterceptor(deps: InterceptorDeps): void {
  chrome.downloads.onCreated.addListener((item) => {
    // Only act on freshly-started downloads. `onCreated` can also fire for
    // history rows restored on startup (state "complete"/"interrupted") —
    // never cancel those.
    if (item.state !== "in_progress") return;
    if (handlingIds.has(item.id)) {
      log.debug("dedup: skipping duplicate onCreated for", item.id);
      return;
    }
    handlingIds.add(item.id);
    void handleItem(item, deps)
      .catch((err) => log.warn("handleItem failed", err))
      .finally(() => handlingIds.delete(item.id));
  });
  log.info("download-interceptor installed (onCreated)");
}

/** Cancel the browser's copy of `id`, then erase it from the shelf when
 * `hideShelf` is set. Both swallow errors (and clear `lastError`): a
 * cancel that loses a race is benign, and erase is cosmetic. */
async function cancelBrowserDownload(id: number, hideShelf: boolean): Promise<void> {
  try {
    await chrome.downloads.cancel(id);
  } catch (err) {
    void chrome.runtime.lastError;
    log.warn("downloads.cancel failed", err);
  }
  if (hideShelf) {
    try {
      await chrome.downloads.erase({ id });
    } catch {
      void chrome.runtime.lastError;
    }
  }
}

/** Re-issue a browser download we previously cancelled (ask-first decline,
 * or a hand-off that failed after we already cancelled). Marked in
 * `letThrough` so the resulting `onCreated` isn't re-intercepted. */
async function redownloadInBrowser(url: string): Promise<void> {
  if (url.length === 0) return;
  letThrough.add(url);
  try {
    await chrome.downloads.download({ url });
  } catch (err) {
    letThrough.delete(url);
    void chrome.runtime.lastError;
    log.warn("re-download fallback failed", err);
  }
}

async function handleItem(
  item: chrome.downloads.DownloadItem,
  deps: InterceptorDeps,
): Promise<void> {
  const settings = deps.settings.current();
  const url = pickUrl(item);
  const filename = pickFilename(item);

  // A download we re-issued ourselves (post-decline / post-failure
  // fallback): let the browser keep it exactly once.
  if (letThrough.delete(url)) {
    log.debug("passthrough: re-issued browser download", url);
    return;
  }

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

  const referrer = item.referrer && item.referrer.length > 0 ? item.referrer : null;

  // ask-first: cancel up front so no "Save As" dialog appears while the
  // prompt is open, then ask. On decline, hand the download back to the
  // browser. We build the job first because the prompt needs it.
  if (decision.kind === "ask") {
    const job = await buildJob({
      item,
      url,
      filename,
      referrer,
      cached: deps.headerCache.getHeadersFor(url) ?? [],
      forwardCookies: settings.forwardCookies,
    });
    await cancelBrowserDownload(item.id, settings.hideShelf);
    const choice = await deps.askHandoff(newAskId(), job);
    if (choice !== "capture") {
      log.debug("ask-first: user declined → re-download in browser", url);
      await redownloadInBrowser(url);
      return;
    }
    try {
      await deps.bridge.send({ type: "download", job });
    } catch (err) {
      log.warn("bridge.send(download) failed after cancel; re-downloading", err);
      await notifyNotRunningOnce();
      await redownloadInBrowser(url);
    }
    return;
  }

  // Immediate intercept: cancel the browser's copy NOW — at `onCreated`
  // this lands before Chrome reaches the "Save As" prompt, so no dialog
  // appears and nothing is written to the Downloads folder. Then hand the
  // job to the app. The app already proved reachable (`isHealthy`), so a
  // post-cancel send failure is rare; if it happens we re-download in the
  // browser rather than silently drop the file.
  await cancelBrowserDownload(item.id, settings.hideShelf);
  const job = await buildJob({
    item,
    url,
    filename,
    referrer,
    cached: deps.headerCache.getHeadersFor(url) ?? [],
    forwardCookies: settings.forwardCookies,
  });
  try {
    await deps.bridge.send({ type: "download", job });
  } catch (err) {
    log.warn("bridge.send(download) failed after cancel; re-downloading", err);
    await notifyNotRunningOnce();
    await redownloadInBrowser(url);
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
    originalUrl: stripFragment(input.item.url) || input.url,
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
  const raw = item.finalUrl && item.finalUrl.length > 0 ? item.finalUrl : (item.url ?? "");
  return stripFragment(raw);
}

/**
 * Drop the `#fragment`. Fragments are never sent over the wire, so a URL
 * like `https://host/id#filename.rar` is stored in the header cache (keyed
 * by the wire URL `https://host/id`) and requested by the engine without
 * it. Stripping here keeps the cache/cookie lookups and the URL we hand to
 * the engine aligned with that wire URL — otherwise the captured
 * `Sec-Fetch-*` / client-hint headers miss the cache and never reach the
 * engine. One-click hosts that pass the filename as a fragment relied on
 * this mismatch failing silently.
 */
function stripFragment(url: string): string {
  const hash = url.indexOf("#");
  return hash >= 0 ? url.slice(0, hash) : url;
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
