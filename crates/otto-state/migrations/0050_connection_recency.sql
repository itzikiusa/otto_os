-- Recency and favorites for connection profiles.
-- `last_opened_at` is stamped on `open` (via DB Explorer or Connections
-- "Open as terminal"); NULL means "never opened". `pinned` allows the user to
-- keep a connection at the top of the list regardless of recency.
ALTER TABLE connections ADD COLUMN last_opened_at TEXT;
ALTER TABLE connections ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;

CREATE INDEX idx_connections_recency ON connections(last_opened_at DESC);
