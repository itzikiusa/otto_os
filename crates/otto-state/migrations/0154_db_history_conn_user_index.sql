-- Per-user Database Explorer history list:
--   WHERE connection_id = ? AND user_id = ? ORDER BY created_at DESC LIMIT ?
-- Only (connection_id, created_at) and (user_id) were indexed, so the list
-- walked every user's rows on the connection (or every connection's rows for
-- the user) before filtering. Additive; the older indexes stay.
CREATE INDEX IF NOT EXISTS idx_db_hist_conn_user
    ON db_query_history(connection_id, user_id, created_at DESC);
