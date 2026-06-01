//! Wire contract shared by the browser extension, the native messaging
//! host, and the in-app pipe server.
//!
//! Nothing in this module talks to stdin/stdout or
//! to a named pipe — it only defines the JSON shapes both sides
//! exchange. The actual IPC plumbing lives in the native host,
//! the pipe server, and the extension bridge.
//!
//! The types are deliberately small and serde-driven. The same definitions
//! are exported to TypeScript via `ts-rs` under the `ts-rs-export` feature
//! flag (see [`tests/export_wire_types.rs`](../../tests/export_wire_types.rs))
//! so the extension and the frontend cannot drift from the Rust source of
//! truth.

use serde::{Deserialize, Serialize};

#[cfg(feature = "ts-rs-export")]
use ts_rs::TS;

pub mod framing;

/// Single captured HTTP request header. Stored as a tuple-of-strings rather
/// than a [`reqwest::header::HeaderMap`] because the extension can only
/// produce string pairs and the wire format must be self-describing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs-export", derive(TS))]
#[cfg_attr(feature = "ts-rs-export", ts(export, export_to = "wire.d.ts"))]
#[serde(rename_all = "camelCase")]
pub struct RequestHeader {
    pub name: String,
    pub value: String,
}

/// A direct-file download captured by the extension. Mirrors
/// `DownloadJob`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs-export", derive(TS))]
#[cfg_attr(feature = "ts-rs-export", ts(export, export_to = "wire.d.ts"))]
#[serde(rename_all = "camelCase")]
pub struct DownloadJob {
    /// URL after the browser followed redirects.
    pub final_url: String,
    /// User-visible URL on the page that triggered the download.
    pub original_url: String,
    pub referrer: Option<String>,
    pub filename: Option<String>,
    pub mime: Option<String>,
    /// File size in bytes. JSON-serialised as a `number` (not `bigint`)
    /// — Chrome's native-messaging port serializes via JSON.stringify,
    /// which throws on BigInt. JS's safe integer range (~9 PB) covers
    /// every realistic file size.
    #[cfg_attr(feature = "ts-rs-export", ts(type = "number | null"))]
    pub size: Option<u64>,
    /// Cookie value as Chrome would send it on a fresh GET to `final_url`.
    pub cookie_header: Option<String>,
    pub user_agent: Option<String>,
    /// Headers observed by the extension's webRequest cache for this URL.
    /// Already filtered on the extension side; the native host still
    /// re-applies its own drop-list defensively.
    pub request_headers: Vec<RequestHeader>,
    #[cfg_attr(feature = "ts-rs-export", ts(type = "number | null"))]
    pub tab_id: Option<i64>,
    pub page_url: Option<String>,
}

/// Container format of a sniffed streaming manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs-export", derive(TS))]
#[cfg_attr(feature = "ts-rs-export", ts(export, export_to = "wire.d.ts"))]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Hls,
    Dash,
}

/// A streaming media manifest sniffed by the extension. Routed to
/// `core::ytdlp` on the native side — yt-dlp handles segment assembly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs-export", derive(TS))]
#[cfg_attr(feature = "ts-rs-export", ts(export, export_to = "wire.d.ts"))]
#[serde(rename_all = "camelCase")]
pub struct MediaStream {
    pub kind: MediaKind,
    pub manifest_url: String,
    pub page_url: Option<String>,
    #[cfg_attr(feature = "ts-rs-export", ts(type = "number | null"))]
    pub tab_id: Option<i64>,
    pub suggested_filename: Option<String>,
    pub referrer: Option<String>,
    pub user_agent: Option<String>,
    pub cookie_header: Option<String>,
    pub request_headers: Vec<RequestHeader>,
}

/// One row in an [`Outbound::Status`] payload. Strictly a subset of
/// [`crate::DownloadRecord`] — the extension popup only needs enough to
/// render the "recent downloads" list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs-export", derive(TS))]
#[cfg_attr(feature = "ts-rs-export", ts(export, export_to = "wire.d.ts"))]
#[serde(rename_all = "camelCase")]
pub struct StatusEntry {
    #[cfg_attr(feature = "ts-rs-export", ts(type = "number"))]
    pub id: i64,
    pub url: String,
    pub filename: String,
    pub status: String,
    #[cfg_attr(feature = "ts-rs-export", ts(type = "number | null"))]
    pub total_bytes: Option<u64>,
    #[cfg_attr(feature = "ts-rs-export", ts(type = "number"))]
    pub downloaded_bytes: u64,
}

/// Extension handoff mode. Selected by the user in Settings → Browser
/// (Tier-3 mockup) and consulted on every browser download decision.
/// 9d adds the type; 9e wires the extension consumer surfaces.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs-export", derive(TS))]
#[cfg_attr(feature = "ts-rs-export", ts(export, export_to = "wire.d.ts"))]
#[serde(rename_all = "kebab-case")]
pub enum HandoffMode {
    /// Current behaviour — every captured download is captured.
    #[default]
    CatchAll,
    /// Surface a Tauri modal for every download (9e).
    AskFirst,
    /// Only capture when an `alwaysInterceptHosts` rule matches (9f).
    RulesOnly,
    /// Hand every download straight back to the browser shelf (9e).
    Passthrough,
}

/// A single host rule in either the `blockedHosts` or
/// `alwaysInterceptHosts` list. `addedAt` is the unix-epoch millisecond
/// stamp the user added the rule; `0` is a sentinel for "migrated from
/// the pre-9f flat `string[]` shape" — UIs render it as "added —".
///
/// Array order in the parent list **is** the priority order (front =
/// highest). Drag-reorder in the Tauri panel commits as a full
/// `SettingsPatch` with the new ordering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs-export", derive(TS))]
#[cfg_attr(feature = "ts-rs-export", ts(export, export_to = "wire.d.ts"))]
#[serde(rename_all = "camelCase")]
pub struct HostRule {
    pub pattern: String,
    /// Milliseconds since the unix epoch. `0` marks rules migrated from
    /// the legacy flat `string[]` shape — the UI renders that as
    /// "added —" rather than guessing a date.
    #[cfg_attr(feature = "ts-rs-export", ts(type = "number"))]
    pub added_at: i64,
}

/// Per-rule live metrics. Machine-local on the extension side
/// (`chrome.storage.local`) so the counter doesn't sync between
/// devices — totals are platform-specific and the user expects the
/// dashboard to reflect *this* machine. Pushed to Tauri every ~6 s by
/// the extension's `chrome.alarms` tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs-export", derive(TS))]
#[cfg_attr(feature = "ts-rs-export", ts(export, export_to = "wire.d.ts"))]
#[serde(rename_all = "camelCase")]
pub struct RuleMetric {
    /// The rule pattern. Matches the corresponding `HostRule.pattern`
    /// in `blockedHosts` / `alwaysInterceptHosts`. The extension and
    /// the panel join on this on the frontend.
    pub pattern: String,
    /// Lifetime hit count for this pattern (since the user added it).
    /// Resets to zero when the rule is deleted and recreated.
    #[cfg_attr(feature = "ts-rs-export", ts(type = "number"))]
    pub match_count: u64,
    /// Last hit timestamp, unix-epoch milliseconds. `None` if never
    /// matched.
    #[cfg_attr(feature = "ts-rs-export", ts(type = "number | null"))]
    pub last_match_at: Option<i64>,
}

/// Full extension settings snapshot. Mirrors `Settings` in
/// `extension/src/shared/types.ts`; the ts-rs export keeps the TS and
/// Rust shapes locked. Sent both directions over the pipe.
///
/// 9d ships the *flat* upgrades (`mode`, `installContextMenu`,
/// `hideShelf`, `forwardCookies`, `fileTypes`). 9f upgrades the host
/// rule lists from flat `Vec<String>` to structured `Vec<HostRule>` so
/// the mockup's per-rule "added Mar 12" line is real. The legacy
/// `string[]` shape is migrated on the extension side
/// (`mergeWithDefaults`) on first read after upgrade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs-export", derive(TS))]
#[cfg_attr(feature = "ts-rs-export", ts(export, export_to = "wire.d.ts"))]
#[serde(rename_all = "camelCase")]
pub struct ExtensionSettings {
    pub enabled: bool,
    pub native_host_name: String,
    #[cfg_attr(feature = "ts-rs-export", ts(type = "number"))]
    pub min_size_mb: u32,
    pub extension_allowlist: Vec<String>,
    pub extension_blocklist: Vec<String>,
    pub blocked_hosts: Vec<HostRule>,
    pub always_intercept_hosts: Vec<HostRule>,
    pub detect_hls: bool,
    pub detect_dash: bool,
    pub verbose_logging: bool,
    pub mode: HandoffMode,
    pub install_context_menu: bool,
    pub hide_shelf: bool,
    pub forward_cookies: bool,
    pub file_types: Vec<String>,
}

impl ExtensionSettings {
    /// The canonical default shape. Matches `DEFAULT_SETTINGS` in
    /// `extension/src/shared/settings.ts` byte-for-byte so a fresh
    /// extension install and a fresh Tauri cache agree without any
    /// round-trip.
    pub fn defaults() -> Self {
        Self {
            enabled: true,
            native_host_name: HOST_NAME.to_string(),
            min_size_mb: 1,
            extension_allowlist: Vec::new(),
            extension_blocklist: vec!["html".into(), "pdf".into(), "txt".into(), "json".into()],
            blocked_hosts: Vec::new(),
            always_intercept_hosts: Vec::new(),
            detect_hls: true,
            detect_dash: true,
            verbose_logging: false,
            mode: HandoffMode::CatchAll,
            install_context_menu: true,
            hide_shelf: true,
            forward_cookies: true,
            file_types: Vec::new(),
        }
    }

    /// Apply an in-place patch. Fields left as `None` on the patch are
    /// preserved. Used by the pipe server when the extension calls
    /// `SetSettings { patch }` so a one-key edit doesn't require the
    /// whole shape on the wire.
    pub fn apply(&mut self, patch: SettingsPatch) {
        let SettingsPatch {
            enabled,
            native_host_name,
            min_size_mb,
            extension_allowlist,
            extension_blocklist,
            blocked_hosts,
            always_intercept_hosts,
            detect_hls,
            detect_dash,
            verbose_logging,
            mode,
            install_context_menu,
            hide_shelf,
            forward_cookies,
            file_types,
        } = patch;
        if let Some(v) = enabled {
            self.enabled = v;
        }
        if let Some(v) = native_host_name {
            self.native_host_name = v;
        }
        if let Some(v) = min_size_mb {
            self.min_size_mb = v;
        }
        if let Some(v) = extension_allowlist {
            self.extension_allowlist = v;
        }
        if let Some(v) = extension_blocklist {
            self.extension_blocklist = v;
        }
        if let Some(v) = blocked_hosts {
            self.blocked_hosts = v;
        }
        if let Some(v) = always_intercept_hosts {
            self.always_intercept_hosts = v;
        }
        if let Some(v) = detect_hls {
            self.detect_hls = v;
        }
        if let Some(v) = detect_dash {
            self.detect_dash = v;
        }
        if let Some(v) = verbose_logging {
            self.verbose_logging = v;
        }
        if let Some(v) = mode {
            self.mode = v;
        }
        if let Some(v) = install_context_menu {
            self.install_context_menu = v;
        }
        if let Some(v) = hide_shelf {
            self.hide_shelf = v;
        }
        if let Some(v) = forward_cookies {
            self.forward_cookies = v;
        }
        if let Some(v) = file_types {
            self.file_types = v;
        }
    }
}

/// Sparse extension settings update — every field optional so the
/// Tauri panel can patch a single key without round-tripping the whole
/// shape. Wire-symmetric with [`ExtensionSettings::apply`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs-export", derive(TS))]
#[cfg_attr(feature = "ts-rs-export", ts(export, export_to = "wire.d.ts"))]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsPatch {
    pub enabled: Option<bool>,
    pub native_host_name: Option<String>,
    #[cfg_attr(feature = "ts-rs-export", ts(type = "number | null"))]
    pub min_size_mb: Option<u32>,
    pub extension_allowlist: Option<Vec<String>>,
    pub extension_blocklist: Option<Vec<String>>,
    pub blocked_hosts: Option<Vec<HostRule>>,
    pub always_intercept_hosts: Option<Vec<HostRule>>,
    pub detect_hls: Option<bool>,
    pub detect_dash: Option<bool>,
    pub verbose_logging: Option<bool>,
    pub mode: Option<HandoffMode>,
    pub install_context_menu: Option<bool>,
    pub hide_shelf: Option<bool>,
    pub forward_cookies: Option<bool>,
    pub file_types: Option<Vec<String>>,
}

/// User's answer to an [`Inbound::AskHandoff`] prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs-export", derive(TS))]
#[cfg_attr(feature = "ts-rs-export", ts(export, export_to = "wire.d.ts"))]
#[serde(rename_all = "kebab-case")]
pub enum HandoffDecision {
    /// Capture the download — same outcome as `catch-all` mode.
    Capture,
    /// Hand the download straight back to the browser shelf.
    Passthrough,
}

/// Browser → native host. Tagged on `type`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs-export", derive(TS))]
#[cfg_attr(feature = "ts-rs-export", ts(export, export_to = "wire.d.ts"))]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Inbound {
    Ping,
    Download {
        job: DownloadJob,
    },
    DownloadMedia {
        stream: MediaStream,
    },
    Status,
    /// Tauri panel asks for the cached extension settings.
    GetSettings,
    /// Extension (or Tauri panel via the bridge) pushes a partial
    /// update of the extension settings.
    SetSettings {
        patch: SettingsPatch,
    },
    /// Extension prompts Tauri to ask the user whether to capture or
    /// pass through this job. Only sent when the extension is in
    /// `ask-first` mode. The reply travels later as an unsolicited
    /// [`Outbound::HandoffDecision`] with the same `id`.
    AskHandoff {
        /// Extension-generated correlation token. Echoed back on the
        /// matching `HandoffDecision`. UUID-ish in practice; opaque to
        /// the native side.
        id: String,
        job: DownloadJob,
    },
    /// Periodic snapshot of per-rule match counters pushed by the
    /// extension's `chrome.alarms` tick (~6 s). The pipe server caches
    /// the snapshot and re-broadcasts via `CoreEvent::RuleMetricsUpdated`
    /// for the Tauri panel.
    RuleMetrics {
        metrics: Vec<RuleMetric>,
    },
}

/// Native host → browser. `Ack` carries the `core::download::DownloadId`
/// the row was assigned so the extension can correlate the next `Status`
/// push.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs-export", derive(TS))]
#[cfg_attr(feature = "ts-rs-export", ts(export, export_to = "wire.d.ts"))]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Outbound {
    Pong,
    Ack {
        #[cfg_attr(feature = "ts-rs-export", ts(type = "number"))]
        id: i64,
    },
    Status {
        downloads: Vec<StatusEntry>,
    },
    Error {
        message: String,
    },
    /// Direct reply to [`Inbound::GetSettings`].
    Settings {
        full: ExtensionSettings,
    },
    /// Unsolicited broadcast pushed to every connected pipe client
    /// whenever the cached extension settings change. The originating
    /// client receives its own push; clients dedupe via the `since`
    /// watermark in the extension bridge.
    SettingsChanged {
        full: ExtensionSettings,
    },
    /// Resolution of an `ask-first` prompt. Sent unsolicited after
    /// the user clicks Capture or Pass-through on the Tauri modal.
    /// `id` matches the [`Inbound::AskHandoff`] that opened the
    /// prompt.
    HandoffDecision {
        id: String,
        decision: HandoffDecision,
    },
}

/// The well-known Native Messaging host name registered under
/// `HKCU\Software\<browser>\NativeMessagingHosts\com.unduhin.host`.
/// The installer registers these registry hooks; the NSIS hook wires them.
pub const HOST_NAME: &str = "com.unduhin.host";

/// Dev-time extension ID baked into the manifest's `key` field. Production
/// releases stamp the Web Store ID into `com.unduhin.host.json`'s
/// `allowed_origins` at build time (see `extension/native-host/README.md`).
/// Source of truth so the extension's `key` and the host manifest cannot
/// drift.
pub const ALLOWED_DEV_EXTENSION_ID: &str = "blbgjagjodpiiclpecohlfhebgddkejn";

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip a value through serde_json and back; assert equality and
    /// return the pretty-printed JSON so the calling test can also assert
    /// against an expected wire shape.
    fn roundtrip<T>(value: &T) -> String
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string_pretty(value).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(value, &back, "round-trip mismatch");
        json
    }

    fn sample_job() -> DownloadJob {
        DownloadJob {
            final_url: "https://cdn.example.com/file.zip".into(),
            original_url: "https://example.com/download".into(),
            referrer: Some("https://example.com/page".into()),
            filename: Some("file.zip".into()),
            mime: Some("application/zip".into()),
            size: Some(123_456),
            cookie_header: Some("session=abc; csrf=xyz".into()),
            user_agent: Some("Mozilla/5.0 (X11)".into()),
            request_headers: vec![RequestHeader {
                name: "Accept".into(),
                value: "*/*".into(),
            }],
            tab_id: Some(7),
            page_url: Some("https://example.com/page".into()),
        }
    }

    #[test]
    fn ping_pong_roundtrip() {
        let s = roundtrip(&Inbound::Ping);
        assert!(s.contains("\"type\": \"ping\""));
        let s = roundtrip(&Outbound::Pong);
        assert!(s.contains("\"type\": \"pong\""));
    }

    #[test]
    fn download_roundtrip() {
        let msg = Inbound::Download { job: sample_job() };
        let s = roundtrip(&msg);
        // camelCase confirms `serde(rename_all = "camelCase")` is in effect.
        assert!(s.contains("\"finalUrl\""));
        assert!(s.contains("\"cookieHeader\""));
        assert!(s.contains("\"requestHeaders\""));
    }

    #[test]
    fn download_media_roundtrip() {
        let msg = Inbound::DownloadMedia {
            stream: MediaStream {
                kind: MediaKind::Hls,
                manifest_url: "https://example.com/master.m3u8".into(),
                page_url: Some("https://example.com/watch".into()),
                tab_id: Some(11),
                suggested_filename: Some("episode-1".into()),
                referrer: None,
                user_agent: None,
                cookie_header: None,
                request_headers: vec![],
            },
        };
        let s = roundtrip(&msg);
        assert!(s.contains("\"type\": \"downloadMedia\""));
        assert!(s.contains("\"kind\": \"hls\""));
        assert!(s.contains("\"manifestUrl\""));
    }

    #[test]
    fn status_request_roundtrip() {
        let s = roundtrip(&Inbound::Status);
        assert!(s.contains("\"type\": \"status\""));
    }

    #[test]
    fn ack_roundtrip() {
        let s = roundtrip(&Outbound::Ack { id: 42 });
        assert!(s.contains("\"type\": \"ack\""));
        assert!(s.contains("\"id\": 42"));
    }

    #[test]
    fn status_response_roundtrip() {
        let msg = Outbound::Status {
            downloads: vec![StatusEntry {
                id: 1,
                url: "https://example.com/a.zip".into(),
                filename: "a.zip".into(),
                status: "active".into(),
                total_bytes: Some(1024),
                downloaded_bytes: 256,
            }],
        };
        let s = roundtrip(&msg);
        assert!(s.contains("\"type\": \"status\""));
        assert!(s.contains("\"totalBytes\""));
        assert!(s.contains("\"downloadedBytes\""));
    }

    #[test]
    fn error_roundtrip() {
        let s = roundtrip(&Outbound::Error {
            message: "Unduhin not running".into(),
        });
        assert!(s.contains("\"type\": \"error\""));
        assert!(s.contains("\"message\""));
    }

    #[test]
    fn get_settings_roundtrip() {
        let s = roundtrip(&Inbound::GetSettings);
        assert!(s.contains("\"type\": \"getSettings\""));
    }

    #[test]
    fn set_settings_roundtrip_with_sparse_patch() {
        let patch = SettingsPatch {
            mode: Some(HandoffMode::AskFirst),
            install_context_menu: Some(false),
            ..Default::default()
        };
        let msg = Inbound::SetSettings { patch };
        let s = roundtrip(&msg);
        assert!(s.contains("\"type\": \"setSettings\""));
        // The bare flat fields land at the JSON top level under "patch".
        assert!(s.contains("\"mode\": \"ask-first\""));
        assert!(s.contains("\"installContextMenu\": false"));
        // Untouched optional fields serialize as `null` (serde default
        // for Option). The extension's `applyServerSettings` ignores
        // null branches via `Option<T>::is_some`-style guards.
        assert!(s.contains("\"enabled\": null"));
    }

    #[test]
    fn settings_outbound_roundtrip() {
        let full = ExtensionSettings::defaults();
        let s = roundtrip(&Outbound::Settings { full: full.clone() });
        assert!(s.contains("\"type\": \"settings\""));
        assert!(s.contains("\"mode\": \"catch-all\""));
        assert!(s.contains("\"hideShelf\": true"));
        assert!(s.contains("\"forwardCookies\": true"));
        let s = roundtrip(&Outbound::SettingsChanged { full });
        assert!(s.contains("\"type\": \"settingsChanged\""));
    }

    #[test]
    fn ask_handoff_roundtrip() {
        let msg = Inbound::AskHandoff {
            id: "01HXYZ".into(),
            job: sample_job(),
        };
        let s = roundtrip(&msg);
        assert!(s.contains("\"type\": \"askHandoff\""));
        assert!(s.contains("\"id\": \"01HXYZ\""));
        assert!(s.contains("\"finalUrl\""));
    }

    #[test]
    fn handoff_decision_roundtrip() {
        let s = roundtrip(&Outbound::HandoffDecision {
            id: "01HXYZ".into(),
            decision: HandoffDecision::Capture,
        });
        assert!(s.contains("\"type\": \"handoffDecision\""));
        assert!(s.contains("\"decision\": \"capture\""));
        let s = roundtrip(&Outbound::HandoffDecision {
            id: "01HXYZ".into(),
            decision: HandoffDecision::Passthrough,
        });
        assert!(s.contains("\"decision\": \"passthrough\""));
    }

    #[test]
    fn rule_metrics_roundtrip() {
        let msg = Inbound::RuleMetrics {
            metrics: vec![
                RuleMetric {
                    pattern: "*.example.com".into(),
                    match_count: 7,
                    last_match_at: Some(1_700_000_000_000),
                },
                RuleMetric {
                    pattern: "files.example.com".into(),
                    match_count: 0,
                    last_match_at: None,
                },
            ],
        };
        let s = roundtrip(&msg);
        assert!(s.contains("\"type\": \"ruleMetrics\""));
        assert!(s.contains("\"matchCount\": 7"));
        assert!(s.contains("\"lastMatchAt\": null"));
    }

    #[test]
    fn host_rule_roundtrip() {
        let value = HostRule {
            pattern: "*.example.com".into(),
            added_at: 1_700_000_000_000,
        };
        let s = roundtrip(&value);
        assert!(s.contains("\"pattern\""));
        assert!(s.contains("\"addedAt\""));
    }

    #[test]
    fn extension_settings_apply_patches_in_place() {
        let mut settings = ExtensionSettings::defaults();
        settings.apply(SettingsPatch {
            mode: Some(HandoffMode::Passthrough),
            min_size_mb: Some(50),
            file_types: Some(vec!["zip".into(), "mkv".into()]),
            ..Default::default()
        });
        assert_eq!(settings.mode, HandoffMode::Passthrough);
        assert_eq!(settings.min_size_mb, 50);
        assert_eq!(settings.file_types, vec!["zip", "mkv"]);
        // Unset patch field stays at default.
        assert!(settings.enabled);
        assert!(settings.install_context_menu);
    }

    #[test]
    fn parses_fixture_download() {
        // Mirrors what the extension sends on `chrome.downloads.onDeterminingFilename`.
        let raw = r#"{
            "type": "download",
            "job": {
                "finalUrl": "https://cdn.example.com/x.zip",
                "originalUrl": "https://example.com/x.zip",
                "referrer": null,
                "filename": "x.zip",
                "mime": "application/zip",
                "size": 100,
                "cookieHeader": "a=b",
                "userAgent": "ua",
                "requestHeaders": [{"name": "Accept", "value": "*/*"}],
                "tabId": 1,
                "pageUrl": "https://example.com/"
            }
        }"#;
        let parsed: Inbound = serde_json::from_str(raw).expect("parse");
        match parsed {
            Inbound::Download { job } => {
                assert_eq!(job.final_url, "https://cdn.example.com/x.zip");
                assert_eq!(job.cookie_header.as_deref(), Some("a=b"));
                assert_eq!(job.request_headers.len(), 1);
            }
            other => panic!("expected Download, got {other:?}"),
        }
    }
}
