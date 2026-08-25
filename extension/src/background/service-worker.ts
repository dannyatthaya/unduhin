// Service-worker entry. Wires every background module the extension owns:
//
//   - header capture
//   - cookie forwarder
//   - native bridge
//   - download interceptor
//   - media sniffer
//   - context menu
//   - popup snapshot + download-media + recent-jobs ring buffer
//   - options page reads/writes settings directly via
//     chrome.storage.sync — no message handler needed here.

import { HOST_NAME } from "../shared/types.js";
import type {
  ExtensionSettings,
  Inbound,
  MediaStream,
  MediaStreamsMessage,
  Outbound,
  PopupDownloadMediaResponse,
  PopupMediaStream,
  PopupSnapshotResponse,
} from "../shared/types.js";
import { log } from "../shared/log.js";
import {
  applyServerSettings,
  createSettingsReader,
  SETTINGS_KEY,
  toSettingsPatch,
} from "../shared/settings.js";
import { compareVersions } from "../shared/version.js";
import { installHeaderCapture } from "./header-capture.js";
import { buildCookieHeader } from "./cookie-forwarder.js";
import { createRefreshArmTable } from "./refresh-arm.js";
import { createNativeBridge } from "./native-bridge.js";
import type { NativeBridge } from "./native-bridge.js";
import { installDownloadInterceptor } from "./download-interceptor.js";
import { installMediaSniffer } from "./media-sniffer.js";
import {
  labelFor,
  loadVariants,
  peekManifest,
  type ManifestInfo,
} from "./hls-master.js";
import type { ProbeViaApp } from "./hls-master.js";
import { assembleStreams, sameStreams } from "./stream-view.js";
import { installContextMenu } from "./context-menu.js";
import { mergeStatus, readRecentJobs, recordAck } from "./recent-jobs.js";
import { pruneTo, snapshotForWire } from "./rule-metrics.js";

/** Placeholder for a manifest nothing has resolved yet. Renders as a
 *  plain row with no size and no duration. */
const NOTHING_KNOWN: ManifestInfo = { variants: [], durationSecs: null };

// Hot-applied settings reader. Consumers call `.current()` at the moment
// they need the value so options-page edits reach the next decision
// without needing an extension reload.
const settings = createSettingsReader();

// Host-name provider used by the bridge: read fresh from settings every
// `connectNative` so a user changing the host name in options re-binds
// the next attempt.
async function readHostName(): Promise<string> {
  await settings.ready;
  return settings.current().nativeHostName || HOST_NAME;
}

const headerCache = installHeaderCapture();

/** Rows the app asked us to fold the next matching capture into. See
 *  `refresh-arm.ts` for why the match is on file name and size. */
const refreshArms = createRefreshArmTable();

/**
 * Answer an `Outbound::RefreshCredentials` with a freshly read cookie header.
 *
 * This is the silent tier of the link refresh: it fixes a cookie-gated CDN,
 * where the URL never changed and only the session expired. `chrome.cookies`
 * is live regardless of what tabs are open, so nothing is asked of the user.
 *
 * A signed URL whose token expired is NOT fixable here — the app will fail
 * again and fall back to the "Refresh link" button.
 *
 * Failures are swallowed to a log: the app times its own attempt out, and a
 * thrown error in an unsolicited handler would take down the port.
 */
async function handleRefreshCredentials(
  token: string,
  downloadId: number,
  url: string,
): Promise<void> {
  let cookieHeader = "";
  try {
    cookieHeader = await buildCookieHeader(url);
  } catch (err) {
    log.warn("refreshCredentials: buildCookieHeader failed", err);
  }
  // Replay whatever headers we still hold for this exact URL. The cache is
  // short-lived (90 s), so this is usually empty by the time a download has
  // failed — the cookies above are the part that matters.
  const cached = headerCache.getHeadersFor(url) ?? [];
  const requestHeaders = cached
    .filter((h) => typeof h.name === "string" && h.name.length > 0)
    .map((h) => ({ name: h.name, value: typeof h.value === "string" ? h.value : "" }));

  const msg: Inbound = {
    type: "credentialsRefreshed",
    token,
    downloadId,
    cookieHeader: cookieHeader.length > 0 ? cookieHeader : null,
    userAgent: navigator.userAgent || null,
    requestHeaders,
  };
  try {
    await bridge.send(msg);
  } catch (err) {
    log.warn("refreshCredentials: reply failed", err);
  }
}

// `ask-first` no longer round-trips a capture/passthrough decision through
// the service worker. The interceptor sends the job to the app as an
// `askHandoff`; the app shows its full config dialog and starts the download
// itself via `start_handoff_download`. There is nothing for the SW to track
// and no `HandoffDecision` to resolve — cancelling the app dialog just aborts.

const rawBridge = createNativeBridge(
  readHostName,
  (msg: Outbound) => {
    // Unsolicited `settings` / `settingsChanged` from the Tauri
    // pipe server. Persist through `applyServerSettings`, which dedupes
    // against the current storage shape so the loop-back from a
    // SetSettings we just sent up is a no-op.
    if (msg.type === "settings" || msg.type === "settingsChanged") {
      const full = (msg as { full: ExtensionSettings }).full;
      void applyServerSettings(full);
      return;
    }
    if (msg.type === "extensionUpdated") {
      void handleExtensionUpdated(msg.version);
      return;
    }
    // The user clicked "Refresh link" in the app. Hold the entry so the next
    // matching capture folds into that row instead of adding a new one.
    if (msg.type === "armRefresh") {
      refreshArms.arm({
        downloadId: msg.downloadId,
        filename: msg.filename,
        sizeBytes: msg.sizeBytes,
        origin: msg.origin,
        expiresAt: msg.expiresAtMs,
      });
      return;
    }
    // Tier 0: the URL is still good, only the session died. Chrome's cookie
    // jar is always live, so this needs no page and no tab.
    if (msg.type === "refreshCredentials") {
      void handleRefreshCredentials(msg.token, msg.downloadId, msg.url);
      return;
    }
    // `handoffDecision` frames are vestigial — the app no longer drives the
    // ask-first download through the extension, so we just ignore them. They
    // stay routed here (not through the reply FIFO) via the bridge's
    // UNSOLICITED_TYPES so a stray frame can't hijack a pending ack.
  },
  // On every (re)connect, replay the current settings to the host. The
  // storage→bridge forward in `chrome.storage.onChanged` fails silently
  // while the host is down and nothing else replays it, so edits made
  // during an outage would never reach the host until the *next* edit.
  pushCurrentSettings,
);

// True once a reload is scheduled — the post-sync broadcast and the
// connection greeting can both arrive in one session; reload once.
let reloadScheduled = false;

/** Reload window: long enough for any in-flight pipe ack to resolve,
 *  short enough that the new version is live before the user's next
 *  download. */
const RELOAD_DELAY_MS = 2_000;

/** storage.local key remembering the disk version we last reloaded for.
 *  Persists across `chrome.runtime.reload()` (unlike `chrome.storage.session`
 *  and module state), which is what lets us detect a reload that didn't
 *  take effect and avoid looping. */
const RELOAD_MARKER_KEY = "extReloadAttemptedFor";

/** The app replaced the canonical extension folder on disk. We're an
 *  unpacked extension, so Chrome never auto-reloads us —
 *  `chrome.runtime.reload()` re-reads the folder and boots the new version.
 *
 *  Reload AT MOST ONCE per disk version. The pipe server re-announces the
 *  version on every (re)connect, so without a persisted guard a reload that
 *  doesn't raise `running` to `diskVersion` would loop forever. That happens
 *  whenever Chrome loaded the unpacked extension from a folder *other* than
 *  the canonical one the app updates (`%LOCALAPPDATA%\unduhin\extension`):
 *  the reload re-reads the stale folder and the version never moves. So if
 *  we already tried for this exact version and we're still older, stop and
 *  warn rather than thrash the browser.
 *
 *  Strictly-older check only: a dev running a newer local build gets greeted
 *  with the (older) bundled version on every reconnect and must never reload. */
async function handleExtensionUpdated(diskVersion: string): Promise<void> {
  const running = chrome.runtime.getManifest().version;

  if (compareVersions(diskVersion, running) <= 0) {
    log.debug(
      `extensionUpdated: disk ${diskVersion} not newer than running ${running} — ignoring`,
    );
    // We're current (or newer): drop any stale marker so the next genuine
    // upgrade can reload again.
    await setReloadMarker(null);
    return;
  }

  if (reloadScheduled) return;

  const attempted = await getReloadMarker();
  if (attempted === diskVersion) {
    log.warn(
      `extension is still ${running} after a reload for ${diskVersion} — the browser is ` +
        `loading a different folder than the app updates. Load-unpack the canonical ` +
        `extension at %LOCALAPPDATA%\\unduhin\\extension. Not reloading again to avoid a loop.`,
    );
    return;
  }

  reloadScheduled = true;
  // Persist the attempt BEFORE reloading so the post-reload session can see
  // it. If the reload works, the next greeting hits the "current" branch
  // above and clears the marker.
  await setReloadMarker(diskVersion);
  log.info(
    `extension updated on disk (${running} → ${diskVersion}) — reloading in ${RELOAD_DELAY_MS}ms`,
  );
  setTimeout(() => {
    chrome.runtime.reload();
  }, RELOAD_DELAY_MS);
}

function getReloadMarker(): Promise<string | null> {
  return new Promise((resolve) => {
    chrome.storage.local.get({ [RELOAD_MARKER_KEY]: null }, (items) => {
      const v = items[RELOAD_MARKER_KEY];
      resolve(typeof v === "string" ? v : null);
    });
  });
}

function setReloadMarker(version: string | null): Promise<void> {
  return new Promise((resolve) => {
    if (version === null) {
      chrome.storage.local.remove(RELOAD_MARKER_KEY, () => resolve());
    } else {
      chrome.storage.local.set({ [RELOAD_MARKER_KEY]: version }, () => resolve());
    }
  });
}

/** Push the current local settings to the host. Called on bridge
 *  (re)connect to deliver any edits made while it was disconnected. */
function pushCurrentSettings(): void {
  let patch: ReturnType<typeof toSettingsPatch>;
  try {
    patch = toSettingsPatch(settings.current());
  } catch (err) {
    log.debug("settings resync skipped (reader not ready):", err);
    return;
  }
  rawBridge
    .send({ type: "setSettings", patch })
    .catch((err) => log.debug("settings resync on connect failed:", err));
}

// All consumers downstream of this point talk to `bridge`, not `rawBridge`,
// so every download/downloadMedia ack lands in the recent-jobs buffer.
// `status` replies are merged in here too so a popup-driven refresh
// updates the buffer.
const bridge: NativeBridge = {
  async send(msg: Inbound): Promise<Outbound> {
    const reply = await rawBridge.send(msg);
    if (msg.type === "status" && reply.type === "status") {
      void mergeStatus(reply.downloads);
    }
    void recordAck(msg, reply);
    return reply;
  },
  isHealthy: () => rawBridge.isHealthy(),
  status: () => rawBridge.status(),
  shutdown: () => rawBridge.shutdown(),
};

const mediaSniffer = installMediaSniffer({
  headerCache,
  settings,
  // Resolve a master playlist's qualities the moment the manifest is
  // seen, rather than when the popup asks for them. Playback fetches its
  // manifests over the course of page load, so the work lands well before
  // the user reaches for the toolbar icon and is spread out instead of
  // arriving as one burst — which matters because the native probe tier
  // is dispatched strictly serially on the app side.
  //
  // Fire-and-forget: `loadVariants` caches its own failures and never
  // rejects in practice, and there is no UI waiting on this.
  onStreamDetected: (stream) => {
    if (stream.kind !== "hls") return;
    void loadVariants(
      stream.manifestUrl,
      stream.tabId,
      makeAppProber(stream.pageUrl),
    ).catch((err) => log.debug("variant warm-up failed:", err));
  },
});

installDownloadInterceptor({ headerCache, bridge, settings, refreshArms });
installContextMenu({ headerCache, bridge, settings });

// `chrome.runtime.sendMessage` excludes the sender from delivery, so the
// SW never receives its own `bridge-status` broadcasts — meaning the
// previous `lastBridgeStatus` cache here was stuck at its initial value
// forever. Read directly from `bridge.status()` on every snapshot; it's
// a closed-over closure read and free.

chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (!msg || typeof msg !== "object") return undefined;
  if ("kind" in msg && msg.kind === "popup-snapshot") {
    const override = (msg as { tabId?: number }).tabId;
    // A rejected `buildSnapshot` (e.g. `readRecentJobs` throwing) would
    // otherwise never call `sendResponse`, hanging the popup's message
    // channel. Fall back to a valid empty snapshot so the popup always
    // gets a reply.
    void buildSnapshot(override)
      .catch((err): PopupSnapshotResponse => {
        log.warn("popup-snapshot: buildSnapshot failed", err);
        return { bridgeStatus: "disconnected", streams: [], recentJobs: [] };
      })
      .then(sendResponse);
    return true;
  }
  if ("kind" in msg && msg.kind === "popup-download-media") {
    const req = msg as { tabId: number; manifestUrl: string; masterUrl?: string };
    void handleDownloadMedia(req.tabId, req.manifestUrl, req.masterUrl).then(
      sendResponse,
    );
    return true;
  }
  if ("kind" in msg && msg.kind === "popup-refresh-status") {
    void refreshStatusFromHost().then(() => sendResponse({ ok: true }));
    return true;
  }
  return undefined;
});

/** Last-resort variant source handed to `loadVariants`: ask the app to
 *  probe the manifest with yt-dlp. Only reached once the in-page fetch and
 *  the service-worker fetch have both failed — a restricted or closed tab,
 *  or a CDN that rejects both — because it spawns a subprocess on the
 *  native side and is slow relative to either fetch.
 *
 *  Returns `null` for anything short of a real answer (host down, error
 *  reply, unexpected frame). `loadVariants` treats `null` as "couldn't
 *  answer" and negative-caches it briefly, so a probe outage never
 *  masquerades as "this stream has no qualities" — an empty array would.
 *
 *  `referrer` is the page the manifest was sniffed on, and it is not
 *  optional in practice: the hotlink-protected CDNs this tier exists for
 *  reject a request carrying no Referer regardless of impersonation.
 *
 *  Labels are computed here rather than on the wire — `MediaFormat`
 *  carries no `label`, so both discovery paths format through the same
 *  `labelFor`. */
function makeAppProber(referrer: string | null): ProbeViaApp {
  return async (manifestUrl) => {
    if (!bridge.isHealthy()) return null;
    try {
      const reply = await bridge.send({
        type: "probeMedia",
        url: manifestUrl,
        referrer,
      });
      if (reply.type !== "mediaFormats") return null;
      return reply.formats.map((f) => ({
        url: f.url,
        height: f.height,
        resolution: f.resolution,
        bandwidth: f.bandwidth,
        label: labelFor(f.height, f.bandwidth, f.resolution),
        videoCodec: f.vcodec,
        audioCodec: f.acodec,
        frameRate: f.fps,
        durationSecs: f.durationSecs,
        // yt-dlp's own size, when it had one, is no more exact than the
        // manifest-derived estimate for HLS — it computes the same way.
        // The field is named `estimatedBytes` on both paths so the popup
        // cannot present one as exact and the other as a guess.
        estimatedBytes: f.filesizeBytes,
      }));
    } catch (err) {
      log.debug("probeMedia failed (expected when the host is down):", err);
      return null;
    }
  };
}

/**
 * The popup's on-open snapshot. Deliberately does no network work: media
 * rows are built from what the sniffer already has plus whatever variants
 * are already cached, so the list paints on the popup's first frame.
 *
 * Master playlists that haven't been resolved yet render as plain rows and
 * are upgraded to quality rows by the `media-streams` broadcast that
 * {@link resolveVariantsForTab} sends when resolution lands. In practice
 * the cache is usually already warm — the sniffer kicks resolution off the
 * moment it sees a manifest — and this reply is final.
 *
 * `readRecentJobs` is still awaited: it's a `chrome.storage.session` read,
 * not a network call.
 */
async function buildSnapshot(
  tabIdOverride: number | undefined,
): Promise<PopupSnapshotResponse> {
  const tabId = tabIdOverride ?? (await activeTabId());
  const streams = assembleCachedStreams(tabId);

  // Resolve anything still cold behind the reply. Not awaited — that's
  // the entire point.
  if (tabId != null) void resolveVariantsForTab(tabId, streams);

  const recentJobs = await readRecentJobs();
  return {
    bridgeStatus: bridge.status(),
    streams,
    recentJobs,
  };
}

/** Media rows for `tabId` from cache alone — no fetch, no probe, no await. */
function assembleCachedStreams(tabId: number | null): PopupMediaStream[] {
  const sniffed = tabId == null ? [] : mediaSniffer.getStreamsForTab(tabId);
  return assembleStreams(sniffed, tabId, (url) => peekManifest(url) ?? NOTHING_KNOWN);
}

/**
 * Resolve every HLS master playlist on `tabId` and broadcast the finished
 * list, so a popup that rendered plain rows can regroup them into quality
 * rows.
 *
 * Broadcast once, after everything settles, rather than per stream: each
 * message rebuilds the whole list in the popup, and several in a row read
 * as flicker. Skipped entirely when the result matches `alreadySent` —
 * the common warm-cache case, where the snapshot was already correct.
 */
async function resolveVariantsForTab(
  tabId: number,
  alreadySent: readonly PopupMediaStream[],
): Promise<void> {
  const sniffed = mediaSniffer.getStreamsForTab(tabId);
  const hls = sniffed.filter((s) => s.kind === "hls");
  if (hls.every((s) => peekManifest(s.manifestUrl) != null)) return;

  const resolved = new Map<string, ManifestInfo>();
  await Promise.all(
    hls.map(async (s) => {
      // Each tier is bounded and cached inside `loadVariants`; a non-master
      // (or a failed lookup) yields no variants and renders as a plain row.
      const info = await loadVariants(
        s.manifestUrl,
        tabId,
        makeAppProber(s.pageUrl),
      ).catch((err) => {
        log.debug("loadVariants failed:", err);
        return NOTHING_KNOWN;
      });
      resolved.set(s.manifestUrl, info);
    }),
  );

  // Re-read the sniffed list: playback may have surfaced more manifests
  // while we were resolving, and the tab may have navigated away.
  const streams = assembleStreams(
    mediaSniffer.getStreamsForTab(tabId),
    tabId,
    (url) => resolved.get(url) ?? peekManifest(url) ?? NOTHING_KNOWN,
  );
  if (sameStreams(streams, alreadySent)) return;

  const msg: MediaStreamsMessage = { kind: "media-streams", tabId, streams };
  // `sendMessage` rejects when no popup is listening — the normal case
  // once the user has closed it — and can throw synchronously during
  // browser shutdown.
  try {
    chrome.runtime.sendMessage(msg).catch(() => {});
  } catch {
    /* Chrome is going away; nothing to deliver to. */
  }
}

async function handleDownloadMedia(
  tabId: number,
  manifestUrl: string,
  masterUrl?: string,
): Promise<PopupDownloadMediaResponse> {
  const streams = mediaSniffer.getStreamsForTab(tabId);

  // A variant pick from a master playlist's quality rows: validate the
  // master is still sniffed and the chosen variant is same-origin, then
  // reuse the master's enriched context (cookies/UA/Referer/headers are
  // origin-scoped, so they apply to the variant too).
  const lookupUrl = masterUrl ?? manifestUrl;
  const target = streams.find((s) => s.manifestUrl === lookupUrl);
  if (!target) {
    return { ok: false, error: "stream no longer available" };
  }
  if (masterUrl && !sameOrigin(manifestUrl, masterUrl)) {
    return { ok: false, error: "invalid variant" };
  }
  if (!bridge.isHealthy()) {
    return { ok: false, error: "Unduhin is not running" };
  }

  let enriched: MediaStream;
  try {
    enriched = await mediaSniffer.enrich(target);
  } catch (err) {
    return {
      ok: false,
      error: err instanceof Error ? err.message : String(err),
    };
  }

  // Point the job at the chosen variant when one was picked.
  if (masterUrl) {
    enriched = { ...enriched, manifestUrl };
  }

  // Filename: the manifest tail is usually generic ("video", "playlist").
  // When it is, prefer the page's og:title, falling back to the tab title.
  if (isGenericName(enriched.suggestedFilename)) {
    const title = await captureTitle(tabId);
    if (title) enriched = { ...enriched, suggestedFilename: title };
  }

  try {
    const reply = await bridge.send({ type: "downloadMedia", stream: enriched });
    if (reply.type === "error") {
      return { ok: false, error: reply.message };
    }
    return { ok: true };
  } catch (err) {
    return {
      ok: false,
      error: err instanceof Error ? err.message : String(err),
    };
  }
}

/** Generic manifest basenames that carry no useful filename — the cue to
 * fall back to the page title. */
const GENERIC_NAMES = new Set([
  "video",
  "playlist",
  "media",
  "index",
  "master",
  "stream",
  "chunklist",
  "manifest",
]);

function isGenericName(name: string | null): boolean {
  if (!name) return true;
  return GENERIC_NAMES.has(name.trim().toLowerCase());
}

/** Read the page's og:title (most accurate), falling back to the tab
 * title. Returns a filesystem-safe string, or null if neither is usable
 * (e.g. a restricted page where scripting is blocked). */
async function captureTitle(tabId: number): Promise<string | null> {
  try {
    const [injection] = await chrome.scripting.executeScript({
      target: { tabId },
      func: () => {
        const el = document.querySelector(
          'meta[property="og:title"], meta[name="og:title"]',
        );
        return el?.getAttribute("content") ?? null;
      },
    });
    const og = injection?.result;
    if (typeof og === "string" && og.trim().length > 0) {
      return sanitizeName(og);
    }
  } catch {
    // chrome:// page, no host access, or tab gone — fall back to the title.
  }
  try {
    const tab = await chrome.tabs.get(tabId);
    if (tab.title && tab.title.trim().length > 0) return sanitizeName(tab.title);
  } catch {
    // tab gone — give up; the host keeps the manifest-derived name.
  }
  return null;
}

/** Collapse whitespace and strip characters illegal in filenames, capped
 * to a sane length. The host re-derives the extension. */
function sanitizeName(raw: string): string {
  const cleaned = raw
    .replace(/[\\/:*?"<>|]+/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 120);
  return cleaned.length > 0 ? cleaned : raw.trim().slice(0, 120);
}

function sameOrigin(a: string, b: string): boolean {
  try {
    return new URL(a).origin === new URL(b).origin;
  } catch {
    return false;
  }
}

async function refreshStatusFromHost(): Promise<void> {
  if (!bridge.isHealthy()) return;
  try {
    await bridge.send({ type: "status" });
  } catch (err) {
    log.warn("popup-refresh-status: bridge.send threw", err);
  }
}

async function activeTabId(): Promise<number | null> {
  return new Promise((resolve) => {
    chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
      const id = tabs[0]?.id;
      resolve(typeof id === "number" && id >= 0 ? id : null);
    });
  });
}

// Kick a `ping` whenever the SW boots — browser startup, extension
// install/update, or manual reload from chrome://extensions. Without this
// the bridge stays in `status: "disconnected"` until the 30s alarm tick
// or the next user-driven `send()`, and the interceptor's `isHealthy()`
// pre-check stops every download with a "not running" notification.
// Failures are silent; the reconnect loop picks up.
function kickBridge(reason: string): void {
  log.info(`service worker ${reason} — eager bridge ping`);
  bridge.send({ type: "ping" }).catch((err: Error) => {
    log.info(
      "eager ping failed (expected when host is not running):",
      err.message,
    );
  });
}

chrome.runtime.onInstalled.addListener((details) => {
  log.info("service worker installed:", details.reason);
  kickBridge(`installed (${details.reason})`);
});

chrome.runtime.onStartup.addListener(() => kickBridge("startup"));

// Forward every local settings edit to the Tauri pipe server so
// the Settings → Browser panel stays live without polling. The
// outbound `setSettings` is also broadcast back to us as a
// `settingsChanged`; `applyServerSettings` dedupes that echo against
// the current storage value.
chrome.storage.onChanged.addListener((changes, area) => {
  if (area !== "sync") return;
  const entry = changes[SETTINGS_KEY];
  if (!entry) return;
  const next = entry.newValue;
  if (!next || typeof next !== "object") return;
  // The storage value already went through `mergeWithDefaults` on
  // every write site, so it's structurally complete. Cast it through
  // a known-good patch builder.
  // We dynamically import the merged Settings shape via toSettingsPatch
  // — the storage value matches `Settings` by construction.
  const patch = toSettingsPatch(next as Parameters<typeof toSettingsPatch>[0]);
  rawBridge
    .send({ type: "setSettings", patch })
    .catch((err) =>
      log.debug("settings push to host failed (expected when host is down):", err),
    );
});

// Also kick on top-level evaluation. SW re-executes from the top on
// every wake (idle resume, install, update, manual reload), so this
// covers the cases `onInstalled` / `onStartup` miss — most importantly
// idle-resume, where neither lifecycle event fires.
kickBridge("boot");

// Periodic rule-metrics push. The alarm fires every 6 s
// (`periodInMinutes: 0.1`); the handler snapshots
// `chrome.storage.local.ruleMetrics` and forwards it as
// `Inbound::RuleMetrics`. Best-effort — a missed tick (host down, SW
// suspended) is fine because the snapshot is full each time, not a
// delta.
const RULE_METRICS_ALARM = "rule-metrics-push";
chrome.alarms.create(RULE_METRICS_ALARM, { periodInMinutes: 0.1 });
chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name !== RULE_METRICS_ALARM) return;
  void pushRuleMetrics();
});

async function pushRuleMetrics(): Promise<void> {
  if (!rawBridge.isHealthy()) return;
  try {
    const metrics = await snapshotForWire();
    if (metrics.length === 0) return;
    await rawBridge.send({ type: "ruleMetrics", metrics });
  } catch (err) {
    log.debug("rule-metrics push failed (expected when host is down):", err);
  }
}

// Prune metrics for rules the user has deleted. Fires on every
// settings change; cheap because it only touches
// chrome.storage.local.
chrome.storage.onChanged.addListener((changes, area) => {
  if (area !== "sync") return;
  const entry = changes[SETTINGS_KEY];
  if (!entry) return;
  const next = entry.newValue as
    | { blockedHosts?: { pattern: string }[]; alwaysInterceptHosts?: { pattern: string }[] }
    | undefined;
  if (!next) return;
  const active = new Set<string>();
  for (const r of next.blockedHosts ?? []) {
    if (typeof r.pattern === "string") active.add(r.pattern);
  }
  for (const r of next.alwaysInterceptHosts ?? []) {
    if (typeof r.pattern === "string") active.add(r.pattern);
  }
  void pruneTo(active);
});

// Re-export so esbuild can't tree-shake the wiring side-effects.
export { headerCache, bridge, mediaSniffer, settings };
