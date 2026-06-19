-- API tokens (personal access tokens) for driving the daemon over HTTP from
-- scripts/CLIs. They reuse the auth_sessions table so they flow through the
-- exact same RbacAuthenticator::authenticate path as interactive login tokens
-- (every /api/v1 route and the WS `?token=` gate keep working unchanged).
--
--   kind = 'session'  -> interactive login token, 30-day SLIDING expiry
--   kind = 'api'      -> long-lived personal access token, expiry never slid
--
-- As before, only the SHA-256 hash of the token is stored. token_prefix keeps
-- the first 12 chars of the raw token (not enough to brute-force the rest) so a
-- token can be identified in a list / revoked by the user. label is a
-- human-friendly name ("cli", "ci", ...).
ALTER TABLE auth_sessions ADD COLUMN kind TEXT NOT NULL DEFAULT 'session';
ALTER TABLE auth_sessions ADD COLUMN label TEXT;
ALTER TABLE auth_sessions ADD COLUMN token_prefix TEXT NOT NULL DEFAULT '';
CREATE INDEX idx_auth_sessions_user_kind ON auth_sessions(user_id, kind);
