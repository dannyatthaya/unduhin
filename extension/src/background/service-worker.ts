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
import { createNativeBridge } from "./native-bridge.js";
import type { NativeBridge } from "./native-bridge.js";
import { installDownloadInterceptor } from "./download-interceptor.js";
import { installMediaSniffer } from "./media-sniffer.js";
import { loadVariants } from "./hls-master.js";
import { installContextMenu } from "./context-menu.js";
import { mergeStatus, readRecentJobs, recordAck } from "./recent-jobs.js";
import { pruneTo, snapshotForWire } from "./rule-metrics.js";

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
      handleExtensionUpdated(msg.version);
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

/** The app replaced the canonical extension folder on disk. We're an
 *  unpacked extension, so Chrome never auto-reloads us —
 *  `chrome.runtime.reload()` re-reads the folder and boots the new
 *  version. Strictly-older check only: a dev running a newer local build
 *  gets greeted with the (older) bundled version on every reconnect, and
 *  reloading then would loop forever without ever changing anything. */
function handleExtensionUpdated(diskVersion: string): void {
  const running = chrome.runtime.getManifest().version;
  if (compareVersions(diskVersion, running) <= 0) {
    log.debug(
      `extensionUpdated: disk ${diskVersion} not newer than running ${running} — ignoring`,
    );
    return;
  }
  if (reloadScheduled) return;
  reloadScheduled = true;
  log.info(
    `extension updated on disk (${running} → ${diskVersion}) — reloading in ${RELOAD_DELAY_MS}ms`,
  );
  setTimeout(() => {
    chrome.runtime.reload();
  }, RELOAD_DELAY_MS);
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

const mediaSniffer = installMediaSniffer({ headerCache, settings });

installDownloadInterceptor({ headerCache, bridge, settings });
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

async function buildSnapshot(
  tabIdOverride: number | undefined,
): Promise<PopupSnapshotResponse> {
  const tabId = tabIdOverride ?? (await activeTabId());
  const sniffed = tabId == null ? [] : mediaSniffer.getStreamsForTab(tabId);

  // Resolve HLS master playlists into their selectable qualities. Each
  // fetch is bounded + cached inside `loadVariants`; a non-master (or a
  // failed fetch) yields no variants and renders as a plain row.
  const parsed = await Promise.all(
    sniffed.map(async (s) => ({
      s,
      variants: s.kind === "hls" ? await loadVariants(s.manifestUrl) : [],
    })),
  );

  // A master's renditions are themselves media playlists the sniffer often
  // also caught (e.g. the one hls.js auto-selected). Collect every master's
  // variant URLs so we can drop those twin rows — the master's quality rows
  // already cover them.
  const variantUrls = new Set<string>();
  for (const p of parsed) {
    for (const v of p.variants) variantUrls.add(v.url);
  }

  const streams: PopupMediaStream[] = parsed
    .filter((p) => !(p.variants.length === 0 && variantUrls.has(p.s.manifestUrl)))
    .map((p) => ({
      kind: p.s.kind,
      manifestUrl: p.s.manifestUrl,
      pageUrl: p.s.pageUrl,
      tabId: Number(p.s.tabId ?? tabId),
      suggestedFilename: p.s.suggestedFilename,
      ...(p.variants.length > 0 ? { variants: p.variants } : {}),
    }));

  const recentJobs = await readRecentJobs();
  return {
    bridgeStatus: bridge.status(),
    streams,
    recentJobs,
  };
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
