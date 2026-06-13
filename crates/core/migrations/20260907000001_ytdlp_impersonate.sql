-- yt-dlp browser impersonation toggle.
--
-- When `true`, yt-dlp downloads run with `--extractor-args
-- "generic:impersonate"`, making the generic extractor mimic a real
-- browser's TLS/HTTP fingerprint (via curl_cffi). This defeats
-- Cloudflare's anti-bot challenge (HTTP 403) on browser-captured
-- HLS/DASH manifests and pasted stream URLs — forwarding cookies/UA
-- alone can't, because the block is on the TLS handshake, not headers.
--
-- The arg only affects the generic extractor, so site-specific
-- extractors (YouTube, etc.) are unchanged. Default `true`: it degrades
-- to a warning rather than a failure if the bundled yt-dlp lacks
-- impersonation targets, so on-by-default is safe.

INSERT OR IGNORE INTO settings (key, value) VALUES
    ('ytdlp_impersonate', 'true');
