-- Phase 6 (Installer + auto-updates + About page) — adds setting keys for
-- the release channel selector, opt-in telemetry toggles, and the last
-- update check status surfaced on the About page.

INSERT OR IGNORE INTO settings (key, value) VALUES
    ('update_channel',            '"stable"'),
    ('update_check_on_startup',   'true'),
    ('send_crash_reports',        'false'),
    ('send_usage_stats',          'false'),
    ('last_update_check_at',      '""'),
    ('last_update_check_result',  '""');
