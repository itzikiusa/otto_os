-- Token-expiry tracking for credential-monitor notices (wave 2).
-- RFC3339 UTC timestamp, NULL = unknown / no expiry. Auto-detected where the
-- provider exposes it (GitHub/GitLab), otherwise user-entered (Bitbucket/Jira).
ALTER TABLE git_accounts ADD COLUMN token_expires_at TEXT;
ALTER TABLE issue_accounts ADD COLUMN token_expires_at TEXT;
