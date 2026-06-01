-- Phase 7f: scheduled downloads (start_at), after-queue priority, and
-- global quiet hours. One table, three kinds, gated by `kind`.

CREATE TABLE schedules (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    kind         TEXT    NOT NULL CHECK (kind IN
                          ('start_at', 'after_queue', 'quiet_hours')),
    -- NULL for the global quiet_hours kind; required for start_at /
    -- after_queue kinds (caller enforces the invariant).
    download_id  INTEGER NULL REFERENCES downloads(id) ON DELETE CASCADE,
    -- start_at: full RFC3339 (UTC). quiet_hours: "HH:MM" in the user's
    -- local timezone (the days_mask is interpreted in the same zone).
    start_iso    TEXT    NULL,
    -- quiet_hours: "HH:MM" local time; NULL for other kinds.
    end_iso      TEXT    NULL,
    -- Bit 0 = Mon … bit 6 = Sun. Default 127 = every day.
    days_mask    INTEGER NOT NULL DEFAULT 127,
    active       INTEGER NOT NULL DEFAULT 1,
    created_at   TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX schedules_download_id_idx ON schedules(download_id);
