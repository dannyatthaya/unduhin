-- Phase 9g — opt-in clipboard watcher.
--
-- Stored as a Tauri-canonical boolean (not synced via the extension's
-- chrome.storage) because the OS clipboard belongs to the desktop
-- session, not the browser tab. The frontend toggle in Settings →
-- Browser writes through the existing `useSettingsStore.set` path; the
-- polling composable mounted at app start gates on this row.
--
-- Default is `false` — the watcher only reads the clipboard after the
-- user opts in. See PRIVACY.md for the rationale.

INSERT OR IGNORE INTO settings (key, value) VALUES
    ('watch_clipboard', 'false');
