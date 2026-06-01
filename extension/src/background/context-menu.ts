// Right-click context menu.
//
// Three deterministic menu items so an `onInstalled` re-create doesn't
// duplicate them — `chrome.contextMenus.create` is idempotent against a
// known id, but we still call `removeAll` first so the IDs are
// guaranteed clean even if the user installed a previous version with
// different titles.

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

function buildMenus(): void {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: MENU_LINK,
      title: "Download link with Unduhin",
      contexts: ["link"],
    });
    chrome.contextMenus.create({
      id: MENU_IMAGE,
      title: "Download image with Unduhin",
      contexts: ["image"],
    });
    chrome.contextMenus.create({
      id: MENU_MEDIA,
      title: "Download with Unduhin",
      contexts: ["video", "audio"],
    });
  });
}

function teardownMenus(): void {
  chrome.contextMenus.removeAll(() => {
    void chrome.runtime.lastError;
  });
}

/**
 * Reconcile menu presence with the `installContextMenu` setting.
 * Idempotent — safe to call on every `chrome.storage.onChanged`.
 */
function applyMenuToggle(install: boolean): void {
  if (install) buildMenus();
  else teardownMenus();
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
  const cookieHeader = forwardCookies ? await buildCookieHeader(url).catch(() => "") : "";
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
