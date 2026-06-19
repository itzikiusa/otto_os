-- Scope notifications to an owning user. `user_id` NULL means a global / system
-- notice that every authenticated user can see (the credential monitor,
-- session-event hooks, and skill-eval producers all emit these). A non-NULL
-- `user_id` scopes the notice to that single user. Existing rows predate
-- per-user scoping and are treated as global (NULL), preserving prior behavior.
ALTER TABLE notifications ADD COLUMN user_id TEXT;

-- List/filter by owner is the hot path (global OR mine), so index it.
CREATE INDEX idx_notifications_user ON notifications(user_id);

-- De-dupe is per (user_id, source_key): the same recurring key can exist once
-- per owner without colliding across users. Replaces the global-only unique
-- index from 0011.
--
-- NOTE: SQLite treats each NULL as distinct in a UNIQUE index, so this index
-- does NOT enforce uniqueness for global (user_id IS NULL) notices. The
-- authoritative de-dupe lives in NotificationsRepo::create, whose lookup is
-- scoped by (user_id, source_key) with NULL-safe matching, refreshing the
-- existing row in place. The index is a backstop for per-user keyed rows.
DROP INDEX idx_notifications_source_key;
CREATE UNIQUE INDEX idx_notifications_user_source_key
    ON notifications(user_id, source_key)
    WHERE source_key IS NOT NULL;
