// Download interceptor.
//
// We intercept at `chrome.downloads.onDeterminingFilename` and use it as a
// BARRIER. That event fires while Chrome is still deciding the target path —
// *before* the "Save As" prompt is shown and before any bytes are committed
// to the Downloads folder. Crucially, a listener that returns `true` tells
// Chrome "I'll suggest a filename asynchronously", so Chrome BLOCKS the
// download (and therefore the dialog) until we respond. For a captured
// download we never suggest a filename; we cancel the download out from
// under the determination instead. Because Chrome never reaches the prompt
// step, the dialog can NEVER appear for a captured download.
//
// This replaces the older `onCreated` + `cancel` approach. `onCreated` is a
// non-blocking notification: Chrome keeps advancing toward the dialog while
// the (possibly asleep) service worker wakes and runs `cancel`, so the
// cancel raced the dialog and frequently lost — the dialog flashed, and
// whether it did was timing-dependent ("sometimes blocks, sometimes
// doesn't"). The determining-filename barrier removes the race entirely:
// the dialog is held shut by the `return true`, and the cancel only has to
// clean up the held download, which it does off the dialog's critical path.
//
// Flow for a captured download: hold the dialog (`return true`) → cancel the
// browser's copy → hand the job to the native app. If the app is unreachable
// we never hold — the browser keeps the download as a fallback. For
// `ask-first` we still hold (so the browser dialog never appears) and hand the
// job to the app as an `askHandoff`; the app shows its own config dialog
// (category / location / segments) and starts the download itself. We never
// re-issue an `ask-first` download to the browser — cancelling the app dialog
// just aborts. The only mode that lets the browser keep a download is
// `passthrough` (extension off).
//
// The decision is split in two: `shouldIntercept` in `intercept-rules.ts`
// is pure and exhaustively tested; this module wires it to the chrome
// APIs and the bridge.

import { log } from "../shared/log.js";
import type { DownloadJob, RequestHeader } from "../shared/types.js";
import { buildCookieHeader } from "./cookie-forwarder.js";
import type { HeaderCache } from "./header-capture.js";
import type { NativeBridge } from "./native-bridge.js";
import { shouldIntercept, type ShouldInterceptDecision } from "./intercept-rules.js";
import { recordRuleHit } from "./rule-metrics.js";
import type { SettingsReader } from "../shared/settings.js";

export interface InterceptorDeps {
  readonly headerCache: HeaderCache;
  readonly bridge: NativeBridge;
  readonly settings: SettingsReader;
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
 * `onDeterminingFilename` normally fires once per id, but a stray duplicate
 * would otherwise re-run the cancel/forward pipeline and the second
 * `cancel` would reject once Chrome has torn the record down). While an id
 * is in this set we keep holding the dialog (`return true`) without
 * re-handing-off. Short-lived: cleared after the handler finishes, so SW
 * eviction can't leak it.
 */
const handlingIds = new Set<number>();

/**
 * URLs we deliberately re-issued to the browser (after an `ask-first`
 * decline, or when a hand-off failed post-cancel). The resulting
 * `onDeterminingFilename` for that URL is passed through once (not held,
 * not re-intercepted) — without this guard an `ask-first` decline would
 * loop forever (cancel → re-download → prompt → decline → …).
 */
const letThrough = new Set<string>();

export function installDownloadInterceptor(deps: InterceptorDeps): void {
  // The listener decides SYNCHRONOUSLY whether to hold the dialog
  // (`return true`) — every input it needs (`shouldIntercept`,
  // `bridge.isHealthy`, the `letThrough` set) is synchronous. The slow
  // work (cancel, cookie/header lookup, bridge round-trip, ask-first
  // prompt) runs afterwards in `handleCapture`, while the dialog stays
  // held. Returning `true` is the documented contract for "I will call
  // `suggest()` asynchronously"; for a captured download we deliberately
  // never call it and cancel instead.
  //
  // `onDeterminingFilename`'s callback is typed `=> void`, but the runtime
  // honours a `boolean` return (the async-suggest signal). TypeScript's
  // void-returning-callback rule lets us return `boolean | void` here.
  const onDetermining = (
    item: chrome.downloads.DownloadItem,
    _suggest: (suggestion?: chrome.downloads.DownloadFilenameSuggestion) => void,
  ): boolean | void => {
    // Determining-filename only fires for live downloads, never for
    // history rows restored on startup — but guard anyway.
    if (item.state !== "in_progress") return;

    const url = pickUrl(item);

    // A download we re-issued ourselves (post-decline / post-failure
    // fallback): let the browser keep it exactly once — including its
    // own "Save As" dialog if the user has that enabled. Do NOT hold.
    if (letThrough.delete(url)) {
      log.debug("passthrough: re-issued browser download", url);
      return;
    }

    // Already holding + handling this id (duplicate event): keep the
    // dialog held until the in-flight handler cancels the download.
    if (handlingIds.has(item.id)) {
      log.debug("dedup: still handling", item.id, "— keep holding");
      return true;
    }

    const settings = deps.settings.current();
    const filename = pickFilename(item);
    const decision = shouldIntercept({ url, filename, size: item.totalBytes, settings });
    // Record a hit for any rule that decided the outcome. Buffered;
    // the alarm tick pushes the snapshot to Tauri every 6 s.
    if (decision.kind !== "ask" && decision.matchedPattern) {
      recordRuleHit(decision.matchedPattern);
    }

    if (decision.kind === "passthrough") {
      log.debug("passthrough:", decision.reason, "→", url);
      return; // let Chrome name + save it normally (dialog included).
    }

    // Bridge unreachable: do NOT hold. The user's download proceeds in the
    // browser, and we toast once per session so they know why we let it go.
    if (!deps.bridge.isHealthy()) {
      log.info("bridge unhealthy — leaving download to the browser:", url);
      void notifyNotRunningOnce();
      return;
    }

    // CAPTURE or ASK. Hold the dialog (the `return true` below) and run the
    // cancel + hand-off off the dialog's critical path.
    handlingIds.add(item.id);
    void handleCapture(item, decision, url, filename, deps)
      .catch((err) => log.warn("handleCapture failed", err))
      .finally(() => handlingIds.delete(item.id));
    return true; // BARRIER — blocks the "Save As" dialog until we cancel.
  };

  // The callback is declared `=> void` upstream; the `boolean | void` return
  // is accepted by the void-returning-callback rule and honoured at runtime.
  chrome.downloads.onDeterminingFilename.addListener(onDetermining);
  log.info("download-interceptor installed (onDeterminingFilename barrier)");
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
 * `letThrough` so the resulting `onDeterminingFilename` isn't re-intercepted. */
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

/**
 * Runs while the dialog is held by the barrier (`return true`). Cancels the
 * browser's copy, then hands the job to the app. The dialog is already held
 * shut, so the `cancel` here only has to tear down the held download — it is
 * no longer racing the prompt.
 *
 * `intercept` (catch-all / rules-only): send the job straight to the app,
 * which auto-starts it.
 *
 * `ask`: send the job as an `askHandoff`. The app shows its full config
 * dialog (category / location / segments / …) and starts the download itself
 * (`start_handoff_download`) with the user's choices — so we send NO
 * `download` frame here. We never re-issue the download to the browser on a
 * decline: cancelling the app dialog simply aborts. The only mode that hands
 * a download to the browser is `passthrough` (extension off).
 *
 * The lone exception is a genuine send failure after we've already cancelled
 * (the app died mid-handoff). That's equivalent to "extension off", so we
 * re-download in the browser rather than silently drop the file. It's rare —
 * the bridge proved healthy moments earlier in the listener.
 */
async function handleCapture(
  item: chrome.downloads.DownloadItem,
  decision: ShouldInterceptDecision,
  url: string,
  filename: string,
  deps: InterceptorDeps,
): Promise<void> {
  const settings = deps.settings.current();
  const referrer = item.referrer && item.referrer.length > 0 ? item.referrer : null;

  // Tear down the held download. The dialog is already blocked by the
  // barrier, so this cancel only removes the item from the downloads tray —
  // it's off the dialog's critical path. We never call `suggest()`; the
  // cancel is how we resolve the held determination.
  await cancelBrowserDownload(item.id, settings.hideShelf);

  const job = await buildJob({
    item,
    url,
    filename,
    referrer,
    cached: deps.headerCache.getHeadersFor(url) ?? [],
    forwardCookies: settings.forwardCookies,
  });

  const message =
    decision.kind === "ask"
      ? ({ type: "askHandoff", id: newAskId(), job } as const)
      : ({ type: "download", job } as const);

  try {
    await deps.bridge.send(message);
  } catch (err) {
    log.warn("bridge.send failed after cancel; re-downloading", err);
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
