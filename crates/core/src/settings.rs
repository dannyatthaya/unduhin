//! Key/value settings stored as JSON-encoded values.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::error::{CoreError, Result};

/// The known, validated setting keys. Unknown keys are still allowed by
/// [`crate::Core::set_setting`] (the table is open-ended), but using these
/// constants keeps the CLI/UI honest.
pub mod settings_keys {
    pub const MAX_CONCURRENT_DOWNLOADS: &str = "max_concurrent_downloads";
    pub const DEFAULT_SEGMENTS: &str = "default_segments";
    pub const GLOBAL_SPEED_LIMIT_BPS: &str = "global_speed_limit_bps";
    pub const DEFAULT_OUTPUT_PATH: &str = "default_output_path";
    pub const CONNECT_TIMEOUT_SECS: &str = "connect_timeout_secs";
    pub const READ_TIMEOUT_SECS: &str = "read_timeout_secs";

    pub const THEME_MODE: &str = "theme_mode";
    pub const AUTOSTART: &str = "autostart";
    pub const START_MINIMIZED: &str = "start_minimized";
    pub const CLOSE_BEHAVIOR: &str = "close_behavior";
    pub const CONFIRM_ON_QUIT: &str = "confirm_on_quit";
    pub const NOTIFY_COMPLETE: &str = "notify_complete";
    pub const NOTIFY_FAIL: &str = "notify_fail";
    pub const NOTIFY_QUEUE_EMPTY: &str = "notify_queue_empty";
    pub const MAX_RETRIES: &str = "max_retries";
    pub const RETRY_BACKOFF_BASE_MS: &str = "retry_backoff_base_ms";
    pub const USER_AGENT: &str = "user_agent";

    // yt-dlp integration.
    pub const YTDLP_BINARY_PATH: &str = "ytdlp_binary_path";
    pub const FFMPEG_BINARY_PATH: &str = "ffmpeg_binary_path";
    pub const YTDLP_DEFAULT_FORMAT: &str = "ytdlp_default_format";
    pub const YTDLP_PROBE_TIMEOUT_MS: &str = "ytdlp_probe_timeout_ms";
    pub const YTDLP_CONSENT_ACCEPTED_AT: &str = "ytdlp_consent_accepted_at";

    /// When `true`, yt-dlp downloads are launched with
    /// `--extractor-args "generic:impersonate"`, which makes the *generic*
    /// extractor mimic a real browser's TLS/HTTP fingerprint via curl_cffi.
    /// This is what defeats Cloudflare's anti-bot challenge (HTTP 403) on
    /// browser-captured HLS/DASH manifests and pasted stream URLs — header
    /// forwarding alone can't, because the block is on the TLS handshake,
    /// not the headers. The arg only affects the generic extractor, so
    /// site-specific extractors (YouTube, etc.) are unchanged. Default
    /// `true`; degrades to a warning (not a failure) if the bundled yt-dlp
    /// lacks impersonation targets.
    pub const YTDLP_IMPERSONATE: &str = "ytdlp_impersonate";

    // Installer + auto-updates + About page.
    /// Release channel: `"stable"` or `"beta"`. Selects which manifest the
    /// updater fetches.
    pub const UPDATE_CHANNEL: &str = "update_channel";
    /// Whether to check for an update once on app startup (off-by-default
    /// pending a future "diagnostics" round; default `true` per About page
    /// IA — users expect "auto-checks daily").
    pub const UPDATE_CHECK_ON_STARTUP: &str = "update_check_on_startup";
    /// Opt-in: send anonymous crash reports. Default `false`.
    pub const SEND_CRASH_REPORTS: &str = "send_crash_reports";
    /// Opt-in: send anonymous feature counts + timing. Default `false`.
    pub const SEND_USAGE_STATS: &str = "send_usage_stats";
    /// ISO-8601 timestamp of the most recent update check ("" if never).
    pub const LAST_UPDATE_CHECK_AT: &str = "last_update_check_at";
    /// One of: `""`, `"up_to_date"`, `"update_available"`, `"error"`.
    pub const LAST_UPDATE_CHECK_RESULT: &str = "last_update_check_result";

    /// How the UI should treat the Remove action. One of:
    /// - `"ask"` — show a prompt asking whether to also delete the file
    /// - `"row_only"` — silently delete the DB row only
    /// - `"row_and_data"` — silently delete the DB row and the on-disk file
    pub const DELETE_DEFAULT_ACTION: &str = "delete_default_action";

    /// When `true`, the Add download dialog auto-expands the filename
    /// override field and pre-fills it with the engine's derived
    /// filename (HEAD probe → Content-Disposition / final URL / MIME
    /// → URL tail fallback). Off by default — the field stays hidden
    /// behind a "Use a different filename" disclosure.
    pub const ALWAYS_ASK_FILENAME: &str = "always_ask_filename";

    /// Downloads-view sort + layout preference. Persisted as a JSON
    /// object `{ "view": "grouped" | "flat", "column": "...", "dir":
    /// "asc" | "desc" }`. Validated as an object — shape parity is
    /// enforced on the frontend so a forward-compat reader sees a
    /// known-shape object even if the column list grows. Default is
    /// applied frontend-side when the key is missing.
    pub const DOWNLOADS_SORT: &str = "downloads_sort";

    /// UI language. One of `"en"`, `"id"`, or `"system"`. The frontend
    /// resolves `"system"` from `navigator.language` (`id-*` → `id`,
    /// otherwise `en`). Tray menu labels + tooltip strings live in
    /// Rust and re-read this value on the matching `SettingChanged`
    /// event.
    pub const LANGUAGE: &str = "language";

    /// Opt-in clipboard watcher. When `true`, the Tauri shell polls the
    /// OS clipboard every ~1.5 s and offers to capture URLs whose path
    /// matches the extension allowlist. Tauri-canonical (not synced via
    /// the extension's `chrome.storage`) because the clipboard belongs
    /// to the OS, not the browser. Default `false`.
    pub const WATCH_CLIPBOARD: &str = "watch_clipboard";
}

/// Thin wrapper around [`serde_json::Value`] so consumers don't have to
/// pull serde_json into their public API.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SettingValue(pub serde_json::Value);

impl SettingValue {
    pub fn from_u64(v: u64) -> Self {
        Self(serde_json::Value::from(v))
    }
    pub fn from_string(s: impl Into<String>) -> Self {
        Self(serde_json::Value::String(s.into()))
    }
    pub fn from_bool(b: bool) -> Self {
        Self(serde_json::Value::Bool(b))
    }
    pub fn as_u64(&self) -> Option<u64> {
        self.0.as_u64()
    }
    pub fn as_str(&self) -> Option<&str> {
        self.0.as_str()
    }
    pub fn as_bool(&self) -> Option<bool> {
        self.0.as_bool()
    }
}

impl std::fmt::Display for SettingValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.to_string())
    }
}

/// Parse a JSON literal from a string. Falls back to wrapping the input
/// as a JSON string if parsing fails — this is what the CLI wants so the
/// user can write `unduhin settings set default_output_path /downloads`
/// without quoting.
pub fn parse_user_value(raw: &str) -> SettingValue {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
        return SettingValue(v);
    }
    SettingValue::from_string(raw)
}

pub(crate) async fn get(pool: &SqlitePool, key: &str) -> Result<Option<SettingValue>> {
    let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    match row {
        None => Ok(None),
        Some(r) => {
            let raw: String = r.get("value");
            let v: serde_json::Value = serde_json::from_str(&raw)?;
            Ok(Some(SettingValue(v)))
        }
    }
}

pub(crate) async fn set(pool: &SqlitePool, key: &str, value: &SettingValue) -> Result<()> {
    validate(key, value)?;
    let raw = serde_json::to_string(&value.0)?;
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(raw)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn all(pool: &SqlitePool) -> Result<HashMap<String, SettingValue>> {
    let rows = sqlx::query("SELECT key, value FROM settings")
        .fetch_all(pool)
        .await?;
    let mut out = HashMap::with_capacity(rows.len());
    for r in rows {
        let key: String = r.get("key");
        let value: String = r.get("value");
        let v: serde_json::Value = serde_json::from_str(&value)?;
        out.insert(key, SettingValue(v));
    }
    Ok(out)
}

fn validate(key: &str, value: &SettingValue) -> Result<()> {
    use settings_keys::*;
    match key {
        MAX_CONCURRENT_DOWNLOADS
        | DEFAULT_SEGMENTS
        | GLOBAL_SPEED_LIMIT_BPS
        | CONNECT_TIMEOUT_SECS
        | READ_TIMEOUT_SECS => {
            value.as_u64().ok_or_else(|| CoreError::InvalidSetting {
                key: key.into(),
                message: "expected a non-negative integer".into(),
            })?;
            Ok(())
        }
        DEFAULT_OUTPUT_PATH
        | USER_AGENT
        | YTDLP_BINARY_PATH
        | FFMPEG_BINARY_PATH
        | YTDLP_CONSENT_ACCEPTED_AT
        | LAST_UPDATE_CHECK_AT => {
            value.as_str().ok_or_else(|| CoreError::InvalidSetting {
                key: key.into(),
                message: "expected a JSON string".into(),
            })?;
            Ok(())
        }
        UPDATE_CHANNEL => match value.as_str() {
            Some("stable") | Some("beta") => Ok(()),
            _ => Err(CoreError::InvalidSetting {
                key: key.into(),
                message: "expected one of: stable, beta".into(),
            }),
        },
        LAST_UPDATE_CHECK_RESULT => match value.as_str() {
            Some("") | Some("up_to_date") | Some("update_available") | Some("error") => Ok(()),
            _ => Err(CoreError::InvalidSetting {
                key: key.into(),
                message: "expected one of: \"\", up_to_date, update_available, error".into(),
            }),
        },
        UPDATE_CHECK_ON_STARTUP | SEND_CRASH_REPORTS | SEND_USAGE_STATS => {
            value.as_bool().ok_or_else(|| CoreError::InvalidSetting {
                key: key.into(),
                message: "expected a boolean".into(),
            })?;
            Ok(())
        }
        YTDLP_DEFAULT_FORMAT => {
            let s = value.as_str().ok_or_else(|| CoreError::InvalidSetting {
                key: key.into(),
                message: "expected a JSON string".into(),
            })?;
            if s.is_empty() {
                return Err(CoreError::InvalidSetting {
                    key: key.into(),
                    message: "yt-dlp format selector cannot be empty".into(),
                });
            }
            Ok(())
        }
        YTDLP_PROBE_TIMEOUT_MS => {
            let n = value.as_u64().ok_or_else(|| CoreError::InvalidSetting {
                key: key.into(),
                message: "expected a non-negative integer".into(),
            })?;
            if !(500..=30_000).contains(&n) {
                return Err(CoreError::InvalidSetting {
                    key: key.into(),
                    message: "expected 500..=30000 (milliseconds)".into(),
                });
            }
            Ok(())
        }
        THEME_MODE => match value.as_str() {
            Some("light") | Some("dark") | Some("system") => Ok(()),
            _ => Err(CoreError::InvalidSetting {
                key: key.into(),
                message: "expected one of: light, dark, system".into(),
            }),
        },
        CLOSE_BEHAVIOR => match value.as_str() {
            Some("minimize") | Some("exit") | Some("ask") => Ok(()),
            _ => Err(CoreError::InvalidSetting {
                key: key.into(),
                message: "expected one of: minimize, exit, ask".into(),
            }),
        },
        DELETE_DEFAULT_ACTION => match value.as_str() {
            Some("ask") | Some("row_only") | Some("row_and_data") => Ok(()),
            _ => Err(CoreError::InvalidSetting {
                key: key.into(),
                message: "expected one of: ask, row_only, row_and_data".into(),
            }),
        },
        LANGUAGE => match value.as_str() {
            Some("en") | Some("id") | Some("system") => Ok(()),
            _ => Err(CoreError::InvalidSetting {
                key: key.into(),
                message: "expected one of: en, id, system".into(),
            }),
        },
        DOWNLOADS_SORT => {
            // The frontend owns the shape (view/column/dir). We just
            // make sure the value is a JSON object so a typo doesn't
            // silently land a number or a stray string under the key.
            if !value.0.is_object() {
                return Err(CoreError::InvalidSetting {
                    key: key.into(),
                    message: "expected a JSON object".into(),
                });
            }
            Ok(())
        }
        AUTOSTART | START_MINIMIZED | CONFIRM_ON_QUIT | NOTIFY_COMPLETE | NOTIFY_FAIL
        | NOTIFY_QUEUE_EMPTY | ALWAYS_ASK_FILENAME | WATCH_CLIPBOARD | YTDLP_IMPERSONATE => {
            value.as_bool().ok_or_else(|| CoreError::InvalidSetting {
                key: key.into(),
                message: "expected a boolean".into(),
            })?;
            Ok(())
        }
        MAX_RETRIES => {
            let n = value.as_u64().ok_or_else(|| CoreError::InvalidSetting {
                key: key.into(),
                message: "expected a non-negative integer".into(),
            })?;
            if !(1..=100).contains(&n) {
                return Err(CoreError::InvalidSetting {
                    key: key.into(),
                    message: "expected 1..=100".into(),
                });
            }
            Ok(())
        }
        RETRY_BACKOFF_BASE_MS => {
            let n = value.as_u64().ok_or_else(|| CoreError::InvalidSetting {
                key: key.into(),
                message: "expected a non-negative integer".into(),
            })?;
            if !(100..=60_000).contains(&n) {
                return Err(CoreError::InvalidSetting {
                    key: key.into(),
                    message: "expected 100..=60000 (milliseconds)".into(),
                });
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vstr(s: &str) -> SettingValue {
        SettingValue::from_string(s)
    }
    fn vnum(n: u64) -> SettingValue {
        SettingValue::from_u64(n)
    }
    fn vbool(b: bool) -> SettingValue {
        SettingValue::from_bool(b)
    }

    #[test]
    fn theme_mode_accepts_known_strings() {
        for ok in ["light", "dark", "system"] {
            assert!(validate(settings_keys::THEME_MODE, &vstr(ok)).is_ok());
        }
        assert!(validate(settings_keys::THEME_MODE, &vstr("auto")).is_err());
        assert!(validate(settings_keys::THEME_MODE, &vnum(1)).is_err());
    }

    #[test]
    fn close_behavior_accepts_known_strings() {
        for ok in ["minimize", "exit", "ask"] {
            assert!(validate(settings_keys::CLOSE_BEHAVIOR, &vstr(ok)).is_ok());
        }
        assert!(validate(settings_keys::CLOSE_BEHAVIOR, &vstr("hide")).is_err());
    }

    #[test]
    fn bool_keys() {
        for k in [
            settings_keys::AUTOSTART,
            settings_keys::START_MINIMIZED,
            settings_keys::CONFIRM_ON_QUIT,
            settings_keys::NOTIFY_COMPLETE,
            settings_keys::NOTIFY_FAIL,
            settings_keys::NOTIFY_QUEUE_EMPTY,
            settings_keys::ALWAYS_ASK_FILENAME,
            settings_keys::WATCH_CLIPBOARD,
            settings_keys::YTDLP_IMPERSONATE,
        ] {
            assert!(validate(k, &vbool(true)).is_ok());
            assert!(validate(k, &vbool(false)).is_ok());
            assert!(validate(k, &vstr("yes")).is_err());
        }
    }

    #[test]
    fn max_retries_bounds() {
        assert!(validate(settings_keys::MAX_RETRIES, &vnum(1)).is_ok());
        assert!(validate(settings_keys::MAX_RETRIES, &vnum(100)).is_ok());
        assert!(validate(settings_keys::MAX_RETRIES, &vnum(0)).is_err());
        assert!(validate(settings_keys::MAX_RETRIES, &vnum(101)).is_err());
    }

    #[test]
    fn retry_backoff_bounds() {
        assert!(validate(settings_keys::RETRY_BACKOFF_BASE_MS, &vnum(100)).is_ok());
        assert!(validate(settings_keys::RETRY_BACKOFF_BASE_MS, &vnum(60_000)).is_ok());
        assert!(validate(settings_keys::RETRY_BACKOFF_BASE_MS, &vnum(99)).is_err());
        assert!(validate(settings_keys::RETRY_BACKOFF_BASE_MS, &vnum(60_001)).is_err());
    }

    #[test]
    fn user_agent_accepts_empty_and_strings() {
        assert!(validate(settings_keys::USER_AGENT, &vstr("")).is_ok());
        assert!(validate(settings_keys::USER_AGENT, &vstr("curl/8.6")).is_ok());
        assert!(validate(settings_keys::USER_AGENT, &vnum(1)).is_err());
    }

    #[test]
    fn ytdlp_binary_path_is_a_string() {
        assert!(validate(settings_keys::YTDLP_BINARY_PATH, &vstr("")).is_ok());
        assert!(validate(settings_keys::YTDLP_BINARY_PATH, &vstr("C:/yt-dlp.exe")).is_ok());
        assert!(validate(settings_keys::YTDLP_BINARY_PATH, &vbool(true)).is_err());

        assert!(validate(settings_keys::FFMPEG_BINARY_PATH, &vstr("")).is_ok());
        assert!(validate(settings_keys::FFMPEG_BINARY_PATH, &vnum(42)).is_err());
    }

    #[test]
    fn ytdlp_default_format_rejects_empty() {
        assert!(validate(settings_keys::YTDLP_DEFAULT_FORMAT, &vstr("bv*+ba/b")).is_ok());
        assert!(validate(settings_keys::YTDLP_DEFAULT_FORMAT, &vstr("140")).is_ok());
        assert!(validate(settings_keys::YTDLP_DEFAULT_FORMAT, &vstr("")).is_err());
        assert!(validate(settings_keys::YTDLP_DEFAULT_FORMAT, &vbool(false)).is_err());
    }

    #[test]
    fn update_channel_accepts_known_values() {
        for ok in ["stable", "beta"] {
            assert!(validate(settings_keys::UPDATE_CHANNEL, &vstr(ok)).is_ok());
        }
        assert!(validate(settings_keys::UPDATE_CHANNEL, &vstr("nightly")).is_err());
        assert!(validate(settings_keys::UPDATE_CHANNEL, &vnum(1)).is_err());
    }

    #[test]
    fn last_update_check_result_accepts_known_values() {
        for ok in ["", "up_to_date", "update_available", "error"] {
            assert!(validate(settings_keys::LAST_UPDATE_CHECK_RESULT, &vstr(ok)).is_ok());
        }
        assert!(validate(settings_keys::LAST_UPDATE_CHECK_RESULT, &vstr("partial")).is_err());
    }

    #[test]
    fn telemetry_keys_are_bools() {
        for k in [
            settings_keys::UPDATE_CHECK_ON_STARTUP,
            settings_keys::SEND_CRASH_REPORTS,
            settings_keys::SEND_USAGE_STATS,
        ] {
            assert!(validate(k, &vbool(true)).is_ok());
            assert!(validate(k, &vbool(false)).is_ok());
            assert!(validate(k, &vstr("yes")).is_err());
        }
    }

    #[test]
    fn watch_clipboard_is_a_bool() {
        assert!(validate(settings_keys::WATCH_CLIPBOARD, &vbool(true)).is_ok());
        assert!(validate(settings_keys::WATCH_CLIPBOARD, &vbool(false)).is_ok());
        assert!(validate(settings_keys::WATCH_CLIPBOARD, &vstr("yes")).is_err());
        assert!(validate(settings_keys::WATCH_CLIPBOARD, &vnum(1)).is_err());
    }

    #[test]
    fn language_accepts_known_values() {
        for ok in ["en", "id", "system"] {
            assert!(validate(settings_keys::LANGUAGE, &vstr(ok)).is_ok());
        }
        assert!(validate(settings_keys::LANGUAGE, &vstr("fr")).is_err());
        assert!(validate(settings_keys::LANGUAGE, &vnum(1)).is_err());
    }

    #[test]
    fn downloads_sort_requires_object() {
        let obj = SettingValue(serde_json::json!({
            "view": "grouped",
            "column": "added_at",
            "dir": "desc"
        }));
        assert!(validate(settings_keys::DOWNLOADS_SORT, &obj).is_ok());
        assert!(validate(settings_keys::DOWNLOADS_SORT, &vstr("desc")).is_err());
        assert!(validate(settings_keys::DOWNLOADS_SORT, &vnum(1)).is_err());
    }

    #[test]
    fn ytdlp_probe_timeout_bounds() {
        assert!(validate(settings_keys::YTDLP_PROBE_TIMEOUT_MS, &vnum(500)).is_ok());
        assert!(validate(settings_keys::YTDLP_PROBE_TIMEOUT_MS, &vnum(3000)).is_ok());
        assert!(validate(settings_keys::YTDLP_PROBE_TIMEOUT_MS, &vnum(30_000)).is_ok());
        assert!(validate(settings_keys::YTDLP_PROBE_TIMEOUT_MS, &vnum(499)).is_err());
        assert!(validate(settings_keys::YTDLP_PROBE_TIMEOUT_MS, &vnum(30_001)).is_err());
    }
}
