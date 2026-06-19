-- Task 3.6: owner-scope DB history + saved queries (#L11–#L13).
--
-- #L11: db_query_history was keyed by connection_id only — every user sharing a
--       connection could see every other user's executed SQL. Adding user_id lets
--       non-admin callers filter to their own rows; root/admin get the unfiltered
--       list. Legacy rows have user_id = NULL and will not appear in per-user
--       filtered views (acceptable: they predate multi-user).
--
-- The sessions(created_by) index supports future per-user session listing without
-- a full scan when the sessions table is large.

ALTER TABLE db_query_history ADD COLUMN user_id TEXT;
CREATE INDEX IF NOT EXISTS idx_db_query_history_user ON db_query_history(user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_created_by ON sessions(created_by);
