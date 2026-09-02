-- Feature: AWS console — the account (credential profile) registry.
-- Conventions: TEXT ULID ids, RFC3339 TEXT timestamps, `*_json` TEXT blobs,
-- INTEGER booleans.
--
-- An "account" is how a user reaches AWS from Otto. Two auth modes:
--   profile     — a named profile in ~/.aws/config (SSO, assume-role, MFA,
--                 credential_process… all handled by the aws CLI itself).
--                 `profile` holds the name; no secret is stored.
--   access_keys — static keys entered in the UI. The access-key id is in
--                 params_json; the secret key (and optional session token)
--                 live in the Keychain under `secret_ref` (opaque ref, same
--                 shape as connections). They are injected as env vars into
--                 every `aws` subprocess — never written to ~/.aws.
-- Accounts are a global library (no workspace axis), gated by the `aws`
-- feature (Admin to manage) — mirrors 0023_global_connections.
CREATE TABLE IF NOT EXISTS aws_accounts (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    auth_mode     TEXT NOT NULL CHECK (auth_mode IN ('profile', 'access_keys')),
    profile       TEXT,
    region        TEXT NOT NULL DEFAULT 'us-east-1',
    -- Non-secret extras: {"access_key_id": "...", "role_arn": "...", "color": "..."}
    params_json   TEXT NOT NULL DEFAULT '{}',
    secret_ref    TEXT,
    -- Cached identity from the last successful `sts get-caller-identity`:
    -- {"account": "123456789012", "arn": "...", "user_id": "..."}
    identity_json TEXT,
    -- Cached per-service probe result, see docs/features/aws-console.md.
    permissions_json TEXT,
    permissions_checked_at TEXT,
    environment   TEXT NOT NULL DEFAULT 'dev' CHECK (environment IN ('dev', 'staging', 'prod')),
    created_by    TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    last_used_at  TEXT
);
CREATE INDEX IF NOT EXISTS idx_aws_accounts_name ON aws_accounts(name);
