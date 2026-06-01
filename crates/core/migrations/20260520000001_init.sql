-- Initial schema for Unduhin core. SQLite.

CREATE TABLE categories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    icon TEXT,
    default_output_path TEXT,
    -- JSON array of lower-cased file extensions, no leading dot.
    extension_rules TEXT NOT NULL DEFAULT '[]'
);

CREATE TABLE downloads (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL,
    filename TEXT NOT NULL,
    output_path TEXT NOT NULL,
    total_bytes INTEGER,
    downloaded_bytes INTEGER NOT NULL DEFAULT 0,
    -- One of: queued, active, paused, completed, failed, cancelled.
    status TEXT NOT NULL DEFAULT 'queued',
    error TEXT,
    category_id INTEGER REFERENCES categories(id) ON DELETE SET NULL,
    -- Higher value = scheduled earlier among queued items.
    priority INTEGER NOT NULL DEFAULT 0,
    segments INTEGER NOT NULL DEFAULT 8,
    created_at TEXT NOT NULL,
    completed_at TEXT,
    etag TEXT,
    last_modified TEXT,
    -- Serialized engine::Meta::segments (Vec<SegmentState>) as JSON, kept
    -- in sync with the sidecar so the DB has an authoritative copy.
    segments_meta TEXT
);

CREATE INDEX idx_downloads_status ON downloads(status);
CREATE INDEX idx_downloads_priority ON downloads(priority DESC, created_at ASC);
CREATE INDEX idx_downloads_category ON downloads(category_id);

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    -- JSON-encoded value. Strings are JSON strings, numbers are JSON numbers.
    value TEXT NOT NULL
);

-- Seeded categories with extension-based auto-categorize rules.
INSERT INTO categories (name, icon, extension_rules) VALUES
    ('Documents', 'document',
     '["pdf","doc","docx","odt","txt","rtf","xls","xlsx","ppt","pptx","csv","md","epub","mobi"]'),
    ('Music', 'music',
     '["mp3","flac","wav","aac","ogg","m4a","opus","wma","alac"]'),
    ('Video', 'video',
     '["mp4","mkv","webm","avi","mov","wmv","flv","m4v","mpeg","mpg","3gp"]'),
    ('Compressed', 'archive',
     '["zip","rar","7z","tar","gz","bz2","xz","zst","tgz","tbz"]'),
    ('Programs', 'app',
     '["exe","msi","dmg","pkg","deb","rpm","appimage"]'),
    ('Other', 'other', '[]');

-- Seeded default settings. Numbers are JSON numbers, paths are JSON strings.
INSERT INTO settings (key, value) VALUES
    ('max_concurrent_downloads', '3'),
    ('default_segments', '8'),
    ('global_speed_limit_bps', '0'),
    ('default_output_path', '""'),
    ('connect_timeout_secs', '15'),
    ('read_timeout_secs', '60');
