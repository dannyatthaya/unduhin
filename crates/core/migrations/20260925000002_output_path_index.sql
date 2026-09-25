-- Index `downloads.output_path` for the free-name check run on every add
-- (`download::output_path_taken`), which otherwise scans the whole table once
-- per candidate name. Windows and macOS compare paths case-insensitively
-- (`COLLATE NOCASE`, which is also what `LIKE` needs to use an index); Linux
-- compares them exactly. Each platform uses only its own index.
CREATE INDEX idx_downloads_output_path ON downloads(output_path);
CREATE INDEX idx_downloads_output_path_nocase ON downloads(output_path COLLATE NOCASE);
