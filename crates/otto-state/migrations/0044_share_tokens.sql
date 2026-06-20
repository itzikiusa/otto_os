-- Scoped "share link" tokens (mobile remote-access, plan Task 1.1).
--
-- A share token is a capability bound to ONE session, default read-only, with a
-- short FIXED TTL and an explicit kill switch. It reuses `auth_sessions`,
-- discriminated by `kind='share'` (the `kind` column from 0030 has no CHECK
-- constraint, so the new value needs no schema widening) — the same approach as
-- the PAT ('api') and impersonation ('impersonation') token kinds.
--
-- These columns are NULL / 0 for every existing (and every normal/api/
-- impersonation) row, so adding them is behavior-preserving. `authenticate()`
-- only populates an `AuthContext.scope` when `kind='share'`.
ALTER TABLE auth_sessions ADD COLUMN session_scope TEXT;   -- the single session id a 'share' token may touch; NULL otherwise
ALTER TABLE auth_sessions ADD COLUMN scope_role    TEXT;   -- 'viewer' | 'editor' for share tokens
ALTER TABLE auth_sessions ADD COLUMN revoked       INTEGER NOT NULL DEFAULT 0;  -- explicit kill switch
CREATE INDEX IF NOT EXISTS idx_auth_sessions_scope ON auth_sessions(session_scope);
