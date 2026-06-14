-- Issue tracking accounts (Jira). Mirrors git_accounts layout:
--   email    -> username column
--   base_url -> api_base_url column

CREATE TABLE issue_accounts (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider     TEXT NOT NULL CHECK (provider IN ('jira')),
    label        TEXT NOT NULL,
    username     TEXT NOT NULL,
    token_ref    TEXT NOT NULL,
    api_base_url TEXT,
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_issue_accounts_user ON issue_accounts(user_id);
