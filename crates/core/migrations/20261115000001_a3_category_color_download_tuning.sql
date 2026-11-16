-- Design "Option A3", dialogs / details panel / settings.
--
-- categories.color
--   Named tint for the category's icon tile: one of 'violet', 'orange',
--   'green', 'blue', 'pink', 'yellow', 'neutral'. NULL for rows predating
--   this migration; the UI then derives a tint from the icon, as before.
--
-- downloads.speed_limit_bps
--   Per-download byte-rate cap, on top of the global one. NULL = no cap of
--   its own. HTTP and media downloads only; a torrent shares the session's
--   limiter.
--
-- downloads.help_slow_parts
--   When 1 (the default), a connection that finishes its part takes over
--   half of the part with the most left. HTTP downloads only.
--
-- downloads.sha256
--   Lower-case hex SHA-256 of the finished file, filled in on demand from
--   the details panel. NULL until the user asks for it; cleared whenever
--   the file is downloaded again.

ALTER TABLE categories ADD COLUMN color TEXT NULL;

UPDATE categories SET color = 'violet'  WHERE color IS NULL AND icon = 'video';
UPDATE categories SET color = 'orange'  WHERE color IS NULL AND icon = 'archive';
UPDATE categories SET color = 'green'   WHERE color IS NULL AND icon = 'app';
UPDATE categories SET color = 'blue'    WHERE color IS NULL AND icon = 'document';
UPDATE categories SET color = 'pink'    WHERE color IS NULL AND icon = 'music';

ALTER TABLE downloads ADD COLUMN speed_limit_bps INTEGER NULL;
ALTER TABLE downloads ADD COLUMN help_slow_parts INTEGER NOT NULL DEFAULT 1;
ALTER TABLE downloads ADD COLUMN sha256 TEXT NULL;
