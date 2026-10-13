-- Add 'linear' to the issue_accounts.provider CHECK constraint.
--
-- Linear accounts are stored like Jira ones: the API key rides `token_ref` →
-- Keychain, `username` may be empty (Linear authenticates with the key alone)
-- and `api_base_url` holds the workspace link. The 0003 CHECK only allowed
-- 'jira', so inserts of a Linear account would fail — this widens it.
--
-- SQLite cannot ALTER a CHECK in place, so the table is rebuilt (create-new →
-- copy → drop → rename), keeping every column. Nothing references
-- issue_accounts by foreign key, so the DROP cascades nowhere.
--
-- The DDL below is the current shape: the 0003 base plus 0012's
-- token_expires_at. Nothing else has altered `issue_accounts`.
CREATE TABLE issue_accounts_new (
    id               TEXT PRIMARY KEY,
    user_id          TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider         TEXT NOT NULL CHECK (provider IN ('jira','linear')),
    label            TEXT NOT NULL,
    username         TEXT NOT NULL,
    token_ref        TEXT NOT NULL,
    api_base_url     TEXT,
    created_at       TEXT NOT NULL,
    token_expires_at TEXT
);

INSERT INTO issue_accounts_new
    (id, user_id, provider, label, username, token_ref, api_base_url, created_at, token_expires_at)
SELECT
    id, user_id, provider, label, username, token_ref, api_base_url, created_at, token_expires_at
FROM issue_accounts;

DROP TABLE issue_accounts;
ALTER TABLE issue_accounts_new RENAME TO issue_accounts;
CREATE INDEX idx_issue_accounts_user ON issue_accounts(user_id);
