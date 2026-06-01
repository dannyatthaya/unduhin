// AUTO-GENERATED FROM crates/core/src/wire.rs — DO NOT EDIT BY HAND.
// Run `cargo test -p unduhin-core --features ts-rs-export export_wire_types`
// to regenerate. Shape changes that aren't matched by a Rust-side change
// will fail CI via `git diff --exit-code`.

export type RequestHeader = { name: string, value: string, };

export type MediaKind = "hls" | "dash";

export type MediaStream = { kind: MediaKind, manifestUrl: string, pageUrl: string | null, tabId: number | null, suggestedFilename: string | null, referrer: string | null, userAgent: string | null, cookieHeader: string | null, requestHeaders: Array<RequestHeader>, };

export type DownloadJob = { 
/**
 * URL after the browser followed redirects.
 */
finalUrl: string, 
/**
 * User-visible URL on the page that triggered the download.
 */
originalUrl: string, referrer: string | null, filename: string | null, mime: string | null, 
/**
 * File size in bytes. JSON-serialised as a `number` (not `bigint`)
 * — Chrome's native-messaging port serializes via JSON.stringify,
 * which throws on BigInt. JS's safe integer range (~9 PB) covers
 * every realistic file size.
 */
size: number | null, 
/**
 * Cookie value as Chrome would send it on a fresh GET to `final_url`.
 */
cookieHeader: string | null, userAgent: string | null, 
/**
 * Headers observed by the extension's webRequest cache for this URL.
 * Already filtered on the extension side; the native host still
 * re-applies its own drop-list defensively.
 */
requestHeaders: Array<RequestHeader>, tabId: number | null, pageUrl: string | null, };

export type StatusEntry = { id: number, url: string, filename: string, status: string, totalBytes: number | null, downloadedBytes: number, };

export type HandoffMode = "catch-all" | "ask-first" | "rules-only" | "passthrough";

export type HandoffDecision = "capture" | "passthrough";

export type HostRule = { pattern: string, 
/**
 * Milliseconds since the unix epoch. `0` marks rules migrated from
 * the legacy flat `string[]` shape — the UI renders that as
 * "added —" rather than guessing a date.
 */
addedAt: number, };

export type RuleMetric = { 
/**
 * The rule pattern. Matches the corresponding `HostRule.pattern`
 * in `blockedHosts` / `alwaysInterceptHosts`. The extension and
 * the panel join on this on the frontend.
 */
pattern: string, 
/**
 * Lifetime hit count for this pattern (since the user added it).
 * Resets to zero when the rule is deleted and recreated.
 */
matchCount: number, 
/**
 * Last hit timestamp, unix-epoch milliseconds. `None` if never
 * matched.
 */
lastMatchAt: number | null, };

export type ExtensionSettings = { enabled: boolean, nativeHostName: string, minSizeMb: number, extensionAllowlist: Array<string>, extensionBlocklist: Array<string>, blockedHosts: Array<HostRule>, alwaysInterceptHosts: Array<HostRule>, detectHls: boolean, detectDash: boolean, verboseLogging: boolean, mode: HandoffMode, installContextMenu: boolean, hideShelf: boolean, forwardCookies: boolean, fileTypes: Array<string>, };

export type SettingsPatch = { enabled: boolean | null, nativeHostName: string | null, minSizeMb: number | null, extensionAllowlist: Array<string> | null, extensionBlocklist: Array<string> | null, blockedHosts: Array<HostRule> | null, alwaysInterceptHosts: Array<HostRule> | null, detectHls: boolean | null, detectDash: boolean | null, verboseLogging: boolean | null, mode: HandoffMode | null, installContextMenu: boolean | null, hideShelf: boolean | null, forwardCookies: boolean | null, fileTypes: Array<string> | null, };

export type Inbound = { "type": "ping" } | { "type": "download", job: DownloadJob, } | { "type": "downloadMedia", stream: MediaStream, } | { "type": "status" } | { "type": "getSettings" } | { "type": "setSettings", patch: SettingsPatch, } | { "type": "askHandoff", 
/**
 * Extension-generated correlation token. Echoed back on the
 * matching `HandoffDecision`. UUID-ish in practice; opaque to
 * the native side.
 */
id: string, job: DownloadJob, } | { "type": "ruleMetrics", metrics: Array<RuleMetric>, };

export type Outbound = { "type": "pong" } | { "type": "ack", id: number, } | { "type": "status", downloads: Array<StatusEntry>, } | { "type": "error", message: string, } | { "type": "settings", full: ExtensionSettings, } | { "type": "settingsChanged", full: ExtensionSettings, } | { "type": "handoffDecision", id: string, decision: HandoffDecision, };

export const HOST_NAME = "com.unduhin.host" as const;
export const ALLOWED_DEV_EXTENSION_ID = "blbgjagjodpiiclpecohlfhebgddkejn" as const;
