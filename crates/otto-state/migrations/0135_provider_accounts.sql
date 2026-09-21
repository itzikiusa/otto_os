-- Provider-native login credentials remain in the provider's private home/keychain.
CREATE TABLE provider_accounts (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL CHECK(provider IN ('claude', 'codex')),
    label TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(user_id, provider, label)
);
CREATE INDEX idx_provider_accounts_owner ON provider_accounts(user_id);
