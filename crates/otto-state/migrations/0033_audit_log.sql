-- Append-only audit log of security-relevant actions (the Trust & Safety
-- Center reads this). Rows are written best-effort by `ServerCtx::audit` at a
-- handful of sensitive sites — login success/failure, API-token mint/revoke,
-- settings changes, network-listener toggles, and confirmed DB write-gate
-- overrides — and are NEVER updated or deleted by the app (no UPDATE/DELETE
-- path exists in AuditRepo). Treat it as a forward-only ledger.
--
--   user_id  NULL when the actor is unauthenticated (e.g. a failed login for an
--            unknown username) or a daemon-internal caller. Not a FK: an audit
--            row must survive the user being deleted.
--   action   stable snake_case verb, e.g. 'login.success', 'token.mint',
--            'settings.change', 'network_listener.toggle', 'db.write_confirmed'.
--   target   optional subject of the action (a username, token id, connection
--            name, setting key list, …) — free-form, for display only.
--   detail   optional JSON blob with action-specific context.
--   ip       optional client IP (the real socket peer; forwarding headers are
--            deliberately not trusted, mirroring the login throttle).
CREATE TABLE audit_log (
    id      TEXT PRIMARY KEY,
    ts      TEXT NOT NULL,
    user_id TEXT,
    action  TEXT NOT NULL,
    target  TEXT,
    detail  TEXT,
    ip      TEXT
);

-- The list view orders newest-first and filters by action / actor / time
-- window, so index the columns those predicates and the sort hit.
CREATE INDEX idx_audit_log_ts ON audit_log(ts DESC);
CREATE INDEX idx_audit_log_action ON audit_log(action);
CREATE INDEX idx_audit_log_user ON audit_log(user_id);
