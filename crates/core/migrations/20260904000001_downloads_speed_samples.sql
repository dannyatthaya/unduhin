-- Persisted, downsampled per-download speed series so the detail-pane
-- sparkline can render for downloads that finished before the current
-- session (the live `speedHistory` in the renderer is rebuilt only from
-- in-session `progress_update` events, so a relaunch left it empty).
--
-- Stored as a JSON array of bytes-per-second samples (`[u32, ...]`),
-- capped to a small fixed length by the queue worker and written once on
-- completion. Nullable: rows predating this column, and in-flight rows,
-- simply have no series yet.

ALTER TABLE downloads ADD COLUMN speed_samples TEXT;
