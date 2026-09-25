// Right-click context menu.
//
// Three deterministic menu items, rebuilt by way of `removeAll` so the
// IDs are clean even if the user installed a previous version with
// different titles.
//
// `chrome.contextMenus.create` is NOT idempotent against a known id:
// creating an id that already exists fails with "Cannot create item with
// duplicate id". That, plus the fact that `removeAll` and `create` are
// both callback-async, is why every reconcile goes through the queue in
// `applyMenuToggle` rather than being called directly.

import { log } from "../shared/log.js";
import type { DownloadJob, RequestHeader } from "../shared/types.js";
import { buildCookieHeader } from "./cookie-forwarder.js";
import type { HeaderCache } from "./header-capture.js";
import type { NativeBridge } from "./native-bridge.js";
import type { SettingsReader } from "../shared/settings.js";

export const MENU_LINK = "unduhin-link";
export const MENU_IMAGE = "unduhin-image";
export const MENU_MEDIA = "unduhin-media";

const MENU_IDS = [MENU_LINK, MENU_IMAGE, MENU_MEDIA];

export interface ContextMenuDeps {
  readonly headerCache: HeaderCache;
  readonly bridge: NativeBridge;
  readonly settings: SettingsReader;
}

/** Promise wrapper so a reconcile can await the clear before creating. */
function removeAllMenus(): Promise<void> {
  return new Promise((resolve) => {
    chrome.contextMenus.removeAll(() => {
      void chrome.runtime.lastError;
      resolve();
    });
  });
}

/**
 * Create one item, treating a duplicate id as survivable.
 *
 * Reading `lastError` is what marks it handled; leaving it unread is what
 * turns it into an `Unchecked runtime.lastError` line in the user's
 * console. The queue below should make duplicates unreachable — this is
 * the belt to its braces.
 */
function createMenu(props: chrome.contextMenus.CreateProperties): Promise<void> {
  return new Promise((resolve) => {
    chrome.contextMenus.create(props, () => {
      const err = chrome.runtime.lastError;
      if (err) log.warn("context-menu: create failed", props.id, err.message);
      resolve();
    });
  });
}

async function buildMenus(): Promise<void> {
  await removeAllMenus();
  await createMenu({
    id: MENU_LINK,
    title: "Download link with Unduhin",
    contexts: ["link"],
  });
  await createMenu({
    id: MENU_IMAGE,
    title: "Download image with Unduhin",
    contexts: ["image"],
  });
  await createMenu({
    id: MENU_MEDIA,
    title: "Download with Unduhin",
    contexts: ["video", "audio"],
  });
}

function teardownMenus(): Promise<void> {
  return removeAllMenus();
}

/**
 * Serializes reconciles. Without it two overlapping calls interleave:
 * both issue `removeAll` before either has created anything, so the
 * second one clears nothing and its creates collide with the first's —
 * three duplicate-id errors, one per item.
 *
 * That is not hypothetical. On a fresh install `onInstalled` and the
 * `settings.ready` reconcile both fire, which is exactly the race. It
 * stays quiet on later service-worker wakes only because `onInstalled`
 * does not fire then.
 */
let reconcile: Promise<void> = Promise.resolve();

/**
 * Reconcile menu presence with the `installContextMenu` setting.
 * Idempotent — safe to call on every `chrome.storage.onChanged`.
 */
function applyMenuToggle(install: boolean): void {
  // The `catch` keeps the chain usable: a rejection left unhandled here
  // would poison every later reconcile, not just this one.
  reconcile = reconcile
    .then(() => (install ? buildMenus() : teardownMenus()))
    .catch((e: unknown) => {
      log.warn("context-menu: reconcile failed", e);
    });
}

export function installContextMenu(deps: ContextMenuDeps): void {
  // `onInstalled` covers install + update + chrome restart of the
  // unpacked extension; that's the right place to rebuild the menus.
  chrome.runtime.onInstalled.addListener(() => {
    applyMenuToggle(deps.settings.current().installContextMenu);
  });

  // Reconcile on first load (covers SW idle-resume — `onInstalled`
  // doesn't fire on wake).
  void deps.settings.ready.then(() => {
    applyMenuToggle(deps.settings.current().installContextMenu);
  });

  // Live-apply changes from either the Tauri panel or the
  // extension options page. The reader has already merged on changed
  // so `current()` reflects the new state.
  chrome.storage.onChanged.addListener((changes, area) => {
    if (area !== "sync") return;
    if (!("settings" in changes)) return;
    applyMenuToggle(deps.settings.current().installContextMenu);
  });

  chrome.contextMenus.onClicked.addListener((info, tab) => {
    if (typeof info.menuItemId === "string" && !MENU_IDS.includes(info.menuItemId)) {
      return;
    }
    void handleClick(info, tab, deps);
  });

  log.info("context-menu installed");
}

async function handleClick(
  info: chrome.contextMenus.OnClickData,
  tab: chrome.tabs.Tab | undefined,
  deps: ContextMenuDeps,
): Promise<void> {
  const url = pickUrlFromInfo(info);
  if (!url) {
    log.warn("context-menu: no URL on click info", info);
    return;
  }
  if (!/^https?:/i.test(url)) {
    log.info("context-menu: skipping non-http URL", url);
    return;
  }

  const referrer = info.pageUrl && info.pageUrl.length > 0 ? info.pageUrl : null;

  // Respect `forwardCookies`. When the user wants Unduhin to behave
  // as if it were a fresh browser session, drop cookies on the floor.
  const forwardCookies = deps.settings.current().forwardCookies;
  const cookieHeader = forwardCookies
    ? await buildCookieHeader(url, tab?.id).catch(() => "")
    : "";
  const cached = deps.headerCache.getHeadersFor(url) ?? [];
  const requestHeaders: RequestHeader[] = cached
    .filter((h) => typeof h.name === "string" && h.name.length > 0)
    .map((h) => ({ name: h.name, value: typeof h.value === "string" ? h.value : "" }));

  const job: DownloadJob = {
    finalUrl: url,
    originalUrl: url,
    referrer,
    filename: deriveFilename(url),
    mime: null,
    size: null,
    cookieHeader: cookieHeader.length > 0 ? cookieHeader : null,
    userAgent:
      typeof navigator !== "undefined" && typeof navigator.userAgent === "string"
        ? navigator.userAgent
        : null,
    requestHeaders,
    tabId: tab?.id != null && tab.id >= 0 ? tab.id : null,
    pageUrl: referrer,
  };

  try {
    await deps.bridge.send({ type: "download", job });
  } catch (err) {
    log.warn("context-menu bridge send failed", err);
  }
}

function pickUrlFromInfo(info: chrome.contextMenus.OnClickData): string | null {
  switch (info.menuItemId) {
    case MENU_LINK:
      return nonEmpty(info.linkUrl);
    case MENU_IMAGE:
      return nonEmpty(info.srcUrl);
    case MENU_MEDIA:
      return nonEmpty(info.srcUrl) ?? nonEmpty(info.linkUrl);
    default:
      return null;
  }
}

function nonEmpty(value: string | undefined): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function deriveFilename(url: string): string | null {
  try {
    const tail = new URL(url).pathname.split("/").filter(Boolean).pop();
    return tail ? decodeURIComponent(tail) : null;
  } catch {
    return null;
  }
}
