-- Composite index on auth_sessions to accelerate share-link listing and
-- extend queries, which filter on scope + revoked + expiry simultaneously.
-- The existing idx_auth_sessions_scope covers only the session_scope column;
-- this wider index avoids a full table scan when otto-server lists or validates
-- active (non-revoked, non-expired) share tokens for a specific session.
CREATE INDEX IF NOT EXISTS idx_auth_sessions_scope_listing
    ON auth_sessions(session_scope, revoked, expires_at);
