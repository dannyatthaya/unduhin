-- Phase 9c: provenance column on `downloads`. Records which surface
-- created the row so the Settings → Browser status card can light up
-- the "downloads captured this week" counter and the "last handoff"
-- timestamp without a JOIN against the (non-existent) sessions table.
--
-- Values:
--   'manual'         — Add URL dialog / Tauri commands.
--   'extension_pipe' — Phase 8 native messaging host hand-off.
--   'cli'            — `unduhin add` from the CLI.
--
-- DEFAULT 'manual' keeps every pre-Phase-9c row valid (the column is
-- NOT NULL). Three call-sites — `commands::add_download`,
-- `pipe::handle_download` / `handle_download_media`, and
-- `cli::run_add` — set the value explicitly going forward.

ALTER TABLE downloads ADD COLUMN source TEXT NOT NULL DEFAULT 'manual';
