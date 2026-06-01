// Re-export the wire contract generated from `crates/core/src/wire.rs` and
// hand-author the extension-only `Settings` shape. Settings never crosses
// the bridge, so it stays here rather than in `wire.d.ts`.

export type {
  DownloadJob,
  ExtensionSettings,
  HandoffDecision,
  HandoffMode,
  HostRule,
  MediaKind,
  MediaStream,
  RequestHeader,
  RuleMetric,
  SettingsPatch,
  StatusEntry,
  Inbound,
  Outbound,
} from "./wire";

// Runtime constants — kept in sync with `wire.d.ts` by hand. The generated
// file is types-only, so the `export const` lines there don't survive
// type-erasure. The ts-rs export test re-emits the `.d.ts` on every CI
// run; if these drift the values flagged here will start failing the
// extension-side smoke tests.
//
// Source of truth: `crates/core/src/wire.rs::{HOST_NAME,
// ALLOWED_DEV_EXTENSION_ID}`.
export const HOST_NAME = "com.unduhin.host";
export const ALLOWED_DEV_EXTENSION_ID = "blbgjagjodpiiclpecohlfhebgddkejn";

/**
 * Bridge-status broadcast emitted by the service worker so popup / options
 * can update their connection-status indicator in real time. Sent via
 * `chrome.runtime.sendMessage({ kind: "bridge-status", status })`; the
 * receivers must wrap their `addListener` in a try/catch — the message
 * silently fails if no popup is open, and that's fine.
 */
export type BridgeStatus = "connected" | "reconnecting" | "disconnected";

export interface BridgeStatusMessage {
  readonly kind: "bridge-status";
  readonly status: BridgeStatus;
}

/**
 * Popup → service worker request, served by `service-worker.ts`. The popup
 * sends it on open to pull a single coherent snapshot of everything it
 * needs to render; subsequent changes arrive via `BridgeStatusMessage`.
 */
export interface PopupSnapshotRequest {
  readonly kind: "popup-snapshot";
  /**
   * Optional override — when omitted the SW queries the active tab in the
   * current window. The popup sets this explicitly so a slow query
   * doesn't show streams for the wrong tab.
   */
  readonly tabId?: number;
}

/** A media stream surfaced to the popup. Mirrors the wire `MediaStream` but
 * with `tabId` as a JS `number` (the popup never re-serialises it). */
export interface PopupMediaStream {
  readonly kind: "hls" | "dash";
  readonly manifestUrl: string;
  readonly pageUrl: string | null;
  readonly tabId: number;
  readonly suggestedFilename: string | null;
}

/** A recent download/downloadMedia job remembered for the popup. The ring
 * buffer lives in `chrome.storage.session` under `recentJobs` and is FIFO-
 * capped at 5 entries. `id` is the bigint download id from the host's ack,
 * stringified — bigint doesn't survive `chrome.storage` round-trips. */
export interface PopupRecentJob {
  readonly id: string;
  readonly filename: string;
  readonly status: string;
  readonly at: number;
}

/**
 * Popup → service worker request to send a `downloadMedia` for a stream
 * shown in the "Media on this page" card. The SW enriches the stream
 * (cookies / headers / UA) before handing it to the bridge.
 */
export interface PopupDownloadMediaRequest {
  readonly kind: "popup-download-media";
  readonly tabId: number;
  readonly manifestUrl: string;
}

export interface PopupDownloadMediaResponse {
  readonly ok: boolean;
  readonly error?: string;
}

/**
 * Popup → service worker request to ask the host for fresh statuses on
 * the recent-jobs buffer. The SW issues a `Status` over the bridge and
 * patches `chrome.storage.session.recentJobs` in place. The popup then
 * re-reads from storage.
 */
export interface PopupRefreshStatusRequest {
  readonly kind: "popup-refresh-status";
}

export interface PopupSnapshotResponse {
  readonly bridgeStatus: BridgeStatus;
  readonly streams: readonly PopupMediaStream[];
  readonly recentJobs: readonly PopupRecentJob[];
}

/**
 * Persistent extension settings. Lives in `chrome.storage.sync` keyed by
 * `"settings"`. `DEFAULT_SETTINGS` in `shared/settings.ts` is the
 * write-time source of truth; consumers read through the change-aware
 * reader so settings hot-apply without an extension reload.
 *
 * The shape grew over time: the core started with `nativeHostName` and
 * `verboseLogging`, then the interception and media fields were wired
 * in. The five flat fields the Tauri Settings → Browser panel binds to
 * (`mode`, `installContextMenu`, `hideShelf`, `forwardCookies`,
 * `fileTypes`) came later, each with its own consumer — `intercept-rules`
 * for `mode`, `context-menu` for `installContextMenu`,
 * `download-interceptor` for `hideShelf`, `cookie-forwarder` for
 * `forwardCookies`, and the pill grid for `fileTypes`. The structured
 * `blockedHosts` / `alwaysInterceptHosts` shape was the most recent
 * upgrade.
 */
/**
 * Persisted host-rule entry. `blockedHosts` / `alwaysInterceptHosts`
 * were upgraded from a flat `string[]` to a structured list with
 * `addedAt` so the panel's "added Mar 12" line is real. The legacy
 * shape migrates on first read (see `mergeWithDefaults`).
 */
export interface HostRuleEntry {
  readonly pattern: string;
  /** Unix-epoch milliseconds. `0` = migrated from the legacy shape. */
  readonly addedAt: number;
}

export interface Settings {
  readonly enabled: boolean;
  readonly nativeHostName: string;
  readonly minSizeMb: number;
  readonly extensionAllowlist: readonly string[];
  readonly extensionBlocklist: readonly string[];
  readonly blockedHosts: readonly HostRuleEntry[];
  readonly alwaysInterceptHosts: readonly HostRuleEntry[];
  readonly detectHls: boolean;
  readonly detectDash: boolean;
  readonly verboseLogging: boolean;
  readonly mode: "catch-all" | "ask-first" | "rules-only" | "passthrough";
  readonly installContextMenu: boolean;
  readonly hideShelf: boolean;
  readonly forwardCookies: boolean;
  readonly fileTypes: readonly string[];
}
