-- Refresh Download Link, phase 1: record *why* a download failed, not just
-- the raw error text.
--
-- Values (see `download::ErrorKind`):
--   'expired_auth' — the link or the session died: HTTP 401/403/410, or the
--                    silent variants the completion gate catches (HTTP 200
--                    with an empty body, HTTP 200 with an HTML landing page).
--                    This is the one value the UI branches on: it swaps the
--                    row's "Retry now" button for "Refresh link", because a
--                    plain retry replays the same dead URL and fails again.
--   'network'      — transient status, retries exhausted, truncated body.
--   'disk'         — local I/O failure.
--   'other'        — anything else.
--
-- NULL, not NOT NULL DEFAULT: a row that never failed has no error kind, and
-- NULL states that better than a sentinel. `record_from_row` reads the column
-- with a defensive `try_get`, so rows and databases predating this migration
-- degrade to None rather than failing to load.

ALTER TABLE downloads ADD COLUMN error_kind TEXT NULL;
