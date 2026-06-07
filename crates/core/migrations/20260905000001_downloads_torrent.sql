-- Torrent support, phase 1: make the download backend an explicit
-- discriminator and add a nullable JSON column for torrent state.
--
-- `kind` replaces the older implicit `media_info.is_some()` check the
-- queue worker used to pick a backend:
--   'http'    — multi-segment HTTP/HTTPS via the engine crate.
--   'media'   — yt-dlp subprocess flow.
--   'torrent' — BitTorrent via the crates/torrent facade over librqbit.
--
-- DEFAULT 'http' keeps every pre-torrent row valid (the column is NOT
-- NULL). The backfill UPDATE upgrades existing yt-dlp rows — the ones
-- carrying a non-empty `media_info` blob — to 'media'; everything else is
-- correctly 'http'. `record_from_row` reads both new columns defensively
-- (`try_get(..).ok()..`), matching the `source` / `speed_samples`
-- precedent, so a NULL / unmigrated column degrades rather than panics.
--
-- `torrent` holds the logical torrent state (info-hash, source, file
-- selection, last swarm snapshot) as one nullable JSON blob, exactly like
-- `media_info` / `headers`. The librqbit piece bitfield / fastresume
-- lives outside the DB in <app_data>/torrents/<infohash>/.

ALTER TABLE downloads ADD COLUMN kind TEXT NOT NULL DEFAULT 'http';
ALTER TABLE downloads ADD COLUMN torrent TEXT NULL;

UPDATE downloads SET kind = 'media' WHERE media_info IS NOT NULL AND media_info <> '';

-- Seed the torrent settings (see design §3.G). INSERT OR IGNORE so a
-- re-run or a user who already set a key never clobbers their value.
INSERT OR IGNORE INTO settings (key, value) VALUES
  ('torrent_listen_port','0'),
  ('torrent_enable_dht','true'),
  ('torrent_enable_upnp','true'),
  ('torrent_seed_ratio_milli','0'),
  ('torrent_max_peers','100'),
  ('torrent_download_dir','""');
