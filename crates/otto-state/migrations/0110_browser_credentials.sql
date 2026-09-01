-- Site credentials for the in-app browser: the password is NEVER stored here
-- (or anywhere in SQLite) — only an opaque `keychain_ref` pointing at the
-- macOS Keychain entry (via otto-keychain / OTTO_SECRETS=file in tests). See
-- otto_state::browser_credentials.
CREATE TABLE browser_credentials (
    id             TEXT PRIMARY KEY,
    workspace_id   TEXT NOT NULL,
    domain         TEXT NOT NULL,           -- eTLD+1, lowercased
    username       TEXT NOT NULL,
    keychain_ref   TEXT NOT NULL,           -- otto-keychain key; NEVER the password
    allow_agent_use INTEGER NOT NULL DEFAULT 0,
    notes          TEXT NOT NULL DEFAULT '',
    created_at     TEXT NOT NULL,
    last_used_at   TEXT
);

CREATE UNIQUE INDEX idx_browser_cred_ws_domain_user ON browser_credentials(workspace_id, domain, username);
