-- Phase 5 (yt-dlp integration) — adds the media_info column to downloads
-- and seeds the new settings keys for the runtime-install model.

-- A JSON blob describing the yt-dlp side of a download. NULL for plain
-- direct-file downloads handled by the engine.
ALTER TABLE downloads ADD COLUMN media_info TEXT NULL;

-- New settings keys. Defaults reflect the "managed dir, opt-in install"
-- model: empty path strings mean "use the managed location".
INSERT OR IGNORE INTO settings (key, value) VALUES
    ('ytdlp_binary_path',         '""'),
    ('ffmpeg_binary_path',        '""'),
    ('ytdlp_default_format',      '"bv*+ba/b"'),
    ('ytdlp_probe_timeout_ms',    '3000'),
    ('ytdlp_consent_accepted_at', '""');
