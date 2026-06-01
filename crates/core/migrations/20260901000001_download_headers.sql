-- Phase 8a: persist captured browser request headers (cookies, referer,
-- user-agent, and any other observed headers) per download row.
--
-- Stored as JSON: [[name, value], [name, value], ...]. Null when the
-- row was added without any extension capture (CLI / Add URL dialog).
-- The engine's HEAD probe + ranged GETs replay these so cookie-gated
-- CDNs and Referer-checking sites work end-to-end.

ALTER TABLE downloads ADD COLUMN headers TEXT NULL;
