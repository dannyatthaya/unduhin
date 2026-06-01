-- Phase 4 (Settings page) — adds new setting keys and category ordering.

-- New settings keys. INSERT OR IGNORE so a re-run is a no-op.
INSERT OR IGNORE INTO settings (key, value) VALUES
    ('theme_mode',            '"system"'),
    ('autostart',             'false'),
    ('start_minimized',       'false'),
    ('close_behavior',        '"ask"'),
    ('confirm_on_quit',       'true'),
    ('notify_complete',       'true'),
    ('notify_fail',           'true'),
    ('notify_queue_empty',    'false'),
    ('max_retries',           '5'),
    ('retry_backoff_base_ms', '500'),
    ('user_agent',            '""');

-- Bump max_concurrent_downloads from 3 to 4 only when the row still equals
-- the original seeded value; never overwrite a user-customized number.
UPDATE settings SET value = '4'
    WHERE key = 'max_concurrent_downloads' AND value = '3';

-- Category ordering. SQLite ALTER TABLE only supports ADD COLUMN; the
-- backfill below numbers existing rows in current id order so they keep
-- a stable display order.
ALTER TABLE categories ADD COLUMN display_order INTEGER NOT NULL DEFAULT 0;

UPDATE categories
SET display_order = (
    SELECT COUNT(*)
    FROM categories AS c2
    WHERE c2.id <= categories.id
);
